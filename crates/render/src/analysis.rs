//! Surface analysis: zebra stripes, highlight lines, a curvature map and
//! draft, drawn in place of the shading (docs/design/surfacing.md, stage 1).
//!
//! This module is the definition; `craft3d_gpu` repeats it in
//! `scene.wgsl` (`analysis_of`) from the numbers of [`Resolved`], narrowed
//! last. Every mode is a colour per sample from what the renderer already
//! interpolates: the normal (the tessellator's normals are the exact
//! surface's at the vertices), the point, and for the curvature map a
//! value per vertex that the caller computes from the exact surface
//! (`craft3d_engine::analysis`), never from the mesh.
//!
//! - **Zebra** (reflection stripes): the environment is a tunnel of
//!   parallel light tubes along an axis `d` in the screen plane, at
//!   infinity. The reflected view ray `r = 2 (n·v) n − v` (the normal
//!   turned to the viewer) is a direction; its angle about `d`, measured
//!   from the direction towards the viewer, `φ = atan2(r·b₂, r·b₁)` with
//!   `b₁ = −forward` and `b₂ = d × b₁`, gives the phase `φ / 2π · count +
//!   offset`. A sample is dark where the phase's fractional part is below
//!   `width`. The stripes move with the view, as reflections do, and show a
//!   surface's continuity: a kink across a seam where only the tangent
//!   plane is continuous (G1), smooth where the curvature is too (G2).
//! - **Highlight lines** (Beier & Chen, *Highlight-line algorithm for
//!   realtime surface-quality assessment*, CAD 26(4), 1994): view
//!   independent. Light lines lie in a plane above the model (normal `M =
//!   +Z`, a quarter of the diagonal of the model's box above its top, through the box's
//!   centre), along `E = (cos a, sin a, 0)` and spaced by `diagonal /
//!   count` along `S = M × E`. The normal line through a point `P`,
//!   `P + t N`, meets the plane at `Q = P + t N` with `t = (A − P)·M /
//!   (N·M)`; the phase is `(Q − A)·S / spacing + offset`, dark below
//!   `width` as for zebra. Where `|N·M|` is below [`GRAZING`] (0.05) the normal
//!   line runs along the plane and the sample has no phase: it is the
//!   stripes' mean, as the filtered stripes are just above the bound.
//! - **Band limit** (zebra and highlight lines): the square wave of the
//!   stripes is filtered over the sample's footprint in phase, the
//!   phase's change from one sample to the next, `|∂p/∂x| + |∂p/∂y|`
//!   ([`Resolved::footprint`]), with a tent `±footprint` wide (the box
//!   over the footprint taken twice). The square wave's integrals are
//!   closed form, and so is the filter ([`dark_share`]): a stripe edge is
//!   an S-shaped ramp two footprints wide, centred where the edge was, and
//!   stripes finer than a sample fade to their mean, `width` dark, within
//!   `1 / footprint²` (a box leaves `1 / footprint`, a faint moiré). The
//!   derivatives are exact: the
//!   perspective-correct interpolation's of the normal and the point,
//!   chained through the phase (the GPU takes the interpolants' from
//!   `dpdxFine`/`dpdyFine` and chains them the same way).
//! - **Curvature map**: the value at the sample (interpolated
//!   perspective-correctly from the vertices, like the normal) placed in
//!   `lo..hi` and coloured by a ramp of five stops ([`ramp`]), lit by
//!   `ANALYSIS_AMBIENT + ANALYSIS_DIFFUSE · |n·v|` so that the shape still
//!   reads. A value that is not a number is the neutral colour.
//! - **Draft**: `s = n·p` with `p` the unit pull direction and `n` the
//!   normal as the mesh has it (outward for a solid, not turned to the
//!   viewer). The draft angle is `asin s`: at least the minimum (`s ≥ sin
//!   min`) is *in* (the face releases along the pull), at most minus the
//!   minimum is *out* (it releases against it, towards the other half of
//!   the mould), and between them *too shallow*. Lit like the map.

use crate::ViewAxes;
use craft3d_math::Portable;
use craft3d_math::{Bbox3, Vec3};

/// Below this `|N·M|` a highlight line's normal line runs along the light
/// plane: no phase, and the sample is the stripes' mean (`width` dark), the
/// value the filtered stripes tend to as `|N·M|` falls: at the zoom where
/// the bound is usually seen (the whole model in view) they are that mean
/// just above it already, so the band does not show; seen close, where the
/// stripes just above the bound are still resolved, a step remains. The
/// phase grows as `stripes / |N·M|` periods, and its rounding with it: at
/// most 512 / 0.05 ≈ 10⁴ periods keeps the `f32` phase of the GPU within
/// about 10⁻³ of a period of the CPU's (their footprint grows as
/// `1 / |N·M|²`).
pub const GRAZING: f64 = 0.05;
/// A footprint above this many periods is the mean, `width`, outright: the
/// box over it is the mean within `1 / footprint` (`dark_share`).
pub const FLAT_FOOTPRINT: f64 = 1e6;
/// The lighting of the curvature map and of draft: ambient share.
pub const ANALYSIS_AMBIENT: f64 = 0.55;
/// The lighting of the curvature map and of draft: share by `|n·v|`.
pub const ANALYSIS_DIFFUSE: f64 = 0.45;
/// The most stripes accepted (per turn for zebra, across the box for
/// highlight lines): finer than this aliases on any screen.
pub const MAX_STRIPES: f64 = 512.0;

/// What is drawn in place of the shading.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnalysisMode {
    Zebra,
    Highlight,
    Curvature,
    Draft,
}

impl AnalysisMode {
    /// The number `scene.wgsl` knows the mode by (0 is no analysis).
    pub fn code(self) -> u32 {
        match self {
            AnalysisMode::Zebra => 1,
            AnalysisMode::Highlight => 2,
            AnalysisMode::Curvature => 3,
            AnalysisMode::Draft => 4,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            AnalysisMode::Zebra => "zebra",
            AnalysisMode::Highlight => "highlight",
            AnalysisMode::Curvature => "curvature",
            AnalysisMode::Draft => "draft",
        }
    }
}

/// The colours of the analyses (the window takes them from its theme).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnalysisColors {
    /// Zebra and highlight lines: the dark stripes.
    pub dark: [u8; 3],
    /// Zebra and highlight lines: between the stripes.
    pub light: [u8; 3],
    /// Draft: at least the minimum along the pull.
    pub draft_in: [u8; 3],
    /// Draft: at least the minimum against the pull.
    pub draft_out: [u8; 3],
    /// Draft: less than the minimum either way.
    pub draft_shallow: [u8; 3],
    /// Where there is no value (a curvature the surface does not have, no
    /// normal).
    pub none: [u8; 3],
    /// The curvature map's ramp, from `lo` to `hi`.
    pub ramp: [[u8; 3]; 5],
}

impl Default for AnalysisColors {
    fn default() -> Self {
        Self {
            dark: [20, 22, 26],
            light: [245, 246, 248],
            draft_in: [67, 160, 71],
            draft_out: [30, 136, 229],
            draft_shallow: [229, 57, 53],
            none: [128, 128, 128],
            ramp: [
                [44, 123, 182],
                [171, 217, 233],
                [250, 250, 210],
                [253, 174, 97],
                [215, 25, 28],
            ],
        }
    }
}

/// An analysis view (view state only; nothing of it is in a document).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Analysis {
    pub mode: AnalysisMode,
    /// Zebra: stripe periods per turn about the tubes' axis (rounded to a
    /// whole number); highlight
    /// lines: periods across the diagonal of the model's box. Clamped to
    /// 1..=[`MAX_STRIPES`].
    pub stripes: f64,
    /// Zebra: the tubes' direction on the screen, radians from the screen's
    /// right towards its top; highlight lines: the light lines' direction
    /// in the model's XY plane, radians from X.
    pub direction: f64,
    /// The dark share of a period, clamped to 0..=1.
    pub width: f64,
    /// Shifts the stripes, in periods.
    pub offset: f64,
    /// Draft: the pull direction (any length but zero).
    pub pull: Vec3,
    /// Draft: the least draft angle, radians, clamped to 0..=π/2.
    pub min_draft: f64,
    /// Curvature map: the values at the two ends of the ramp.
    pub range: [f64; 2],
    /// Highlight lines: the least `|N·M|` with a phase ([`GRAZING`] by
    /// default; clamped to 10⁻³..=0.5).
    pub grazing: f64,
    pub colors: AnalysisColors,
}

impl Analysis {
    /// `mode` with the defaults: 16 stripes, horizontal, half dark; a pull
    /// along +Z with a 1° minimum; a range of −1 to 1.
    pub fn new(mode: AnalysisMode) -> Self {
        Self {
            mode,
            stripes: 16.0,
            direction: 0.0,
            width: 0.5,
            offset: 0.0,
            pull: Vec3::new(0.0, 0.0, 1.0),
            min_draft: 1f64.to_radians(),
            range: [-1.0, 1.0],
            grazing: GRAZING,
            colors: AnalysisColors::default(),
        }
    }
}

/// An [`Analysis`] made ready for one picture: its numbers in model
/// coordinates for the camera and the mesh's box. Points are relative to
/// the eye, as the GPU has them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Resolved {
    pub mode: AnalysisMode,
    pub count: f64,
    pub width: f64,
    pub offset: f64,
    /// `sin` of the minimum draft.
    pub sin_min: f64,
    /// Zebra: the tubes' axis `d`; highlight lines: `E`; draft: the pull.
    pub axis: Vec3,
    /// Zebra: `b₁`, towards the viewer; highlight lines: `S`.
    pub first: Vec3,
    /// Zebra: `b₂ = d × b₁`; highlight lines: the plane's normal `M`.
    pub second: Vec3,
    /// Highlight lines: the plane's point `A`, relative to the eye.
    pub point: Vec3,
    /// Highlight lines: the spacing of the light lines.
    pub spacing: f64,
    pub lo: f64,
    pub hi: f64,
    pub grazing: f64,
    /// Whether the view is in perspective: the direction to the viewer
    /// then changes with the point (zebra's footprint).
    pub perspective: bool,
    pub colors: AnalysisColors,
}

/// How a sample's interpolated attributes change towards the next sample
/// to the right (`[0]`) and the next one down (`[1]`): their exact
/// derivatives in the samples' grid, for the stripes' footprint.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Slopes {
    /// The interpolated normal before it is made unit (zero where there is
    /// none: the footprint is then zero, the triangle's own normal being
    /// constant).
    pub normal: Vec3,
    /// The derivatives of [`Slopes::normal`].
    pub d_normal: [Vec3; 2],
    /// The derivatives of the point (relative to the eye).
    pub d_rel: [Vec3; 2],
}

/// The share of a sample that is dark: the square wave dark where the
/// phase's fraction is below `width`, averaged with a tent `±footprint`
/// periods wide centred on `phase` (weights `1 − |s| / footprint`: the box
/// over the footprint taken twice, so that what stripes finer than it leave
/// over the mean falls as `1 / footprint²`, not as `1 / footprint`).
///
/// In three regimes, each closed form and none cancelling where it is used
/// (a second difference of values of order 0.1 divided by `f²` loses
/// everything for a small `f`: in `f32` at `f = 10⁻⁴`, in `f64` at 10⁻⁸):
/// - the tent reaches no edge (`f` at most the distance to the nearest of
///   `0`, `width`, `1`): the point sample, `0` or `1` exactly;
/// - it can reach one edge only (`2f < min(width, 1 − width)`): the tent's
///   share beyond that edge, a quadratic in `d / f` with `d` the signed
///   distance to it;
/// - wider: the wave is `width + g` with `g` periodic of mean zero; the
///   tent's average of `g` is the second difference of `G = ∫∫ g`
///   ([`wave_integral`]) over the footprint, divided by its square.
///
/// A footprint that is not a number, infinite or above [`FLAT_FOOTPRINT`],
/// or a phase that is not finite, is the mean, `width`.
pub fn dark_share(phase: f64, footprint: f64, width: f64) -> f64 {
    if !phase.is_finite() {
        return width;
    }
    if width <= 0.0 {
        return 0.0;
    }
    if width >= 1.0 {
        return 1.0;
    }
    // Relative to the period the sample is in, so that a phase of many
    // periods loses nothing to the subtractions.
    let x = phase - phase.floor();
    let point = if x < width { 1.0 } else { 0.0 };
    if footprint == 0.0 {
        return point;
    }
    if !(footprint > 0.0 && footprint <= FLAT_FOOTPRINT) {
        return width;
    }
    let f = footprint;
    // The distances to the edges: into the dark at 0 (and 1), out of it at
    // `width`.
    let (to_start, to_end, to_next) = (x, (x - width).abs(), 1.0 - x);
    if f <= to_start.min(to_end).min(to_next) {
        return point;
    }
    if 2.0 * f < width.min(1.0 - width) {
        // One edge in reach: the nearest. `beyond(u)` is the tent's share
        // past the point `u · f` from its centre.
        let beyond = |u: f64| {
            let u = u.clamp(-1.0, 1.0);
            if u >= 0.0 {
                (1.0 - u) * (1.0 - u) / 2.0
            } else {
                1.0 - (1.0 + u) * (1.0 + u) / 2.0
            }
        };
        return if to_end <= to_start.min(to_next) {
            // Dark before `width`.
            1.0 - beyond((width - x) / f)
        } else if to_start <= to_next {
            // Dark after 0.
            beyond(-x / f)
        } else {
            // Dark after 1.
            beyond(to_next / f)
        };
    }
    let second =
        wave_integral(x + f, width) - 2.0 * wave_integral(x, width) + wave_integral(x - f, width);
    (width + second / (f * f)).clamp(0.0, 1.0)
}

/// `G(t) = ∫₀ᵗ ∫₀ᵘ g`, with `g(u) = [u − ⌊u⌋ < w] − w` the square wave less
/// its mean: within a period (`u` its fraction) `∫ g` is `u(1 − w)` up to
/// `w` and `w(1 − u)` after, back to zero at its end, and its integral over
/// a whole period is `w(1 − w) / 2`.
pub fn wave_integral(t: f64, w: f64) -> f64 {
    let whole = t.floor();
    let u = t - whole;
    let inner = if u < w {
        (1.0 - w) * u * u / 2.0
    } else {
        (1.0 - w) * w * w / 2.0 + w * ((u - w) - (u * u - w * w) / 2.0)
    };
    whole * w * (1.0 - w) / 2.0 + inner
}

fn finite_or(v: f64, fallback: f64) -> f64 {
    if v.is_finite() { v } else { fallback }
}

impl Resolved {
    /// `analysis` for the view `axes` of a mesh in `bounds`.
    pub fn new(analysis: &Analysis, axes: &ViewAxes, bounds: &Bbox3) -> Self {
        let count = finite_or(analysis.stripes, 16.0).clamp(1.0, MAX_STRIPES);
        // Zebra's phase runs once round the tubes' axis: a whole number of
        // stripes, so that the stripes close up behind (where the
        // reflected ray points away from the viewer).
        let count = if analysis.mode == AnalysisMode::Zebra {
            count.round()
        } else {
            count
        };
        let width = finite_or(analysis.width, 0.5).clamp(0.0, 1.0);
        let offset = finite_or(analysis.offset, 0.0);
        let angle = finite_or(analysis.direction, 0.0);
        let min = finite_or(analysis.min_draft, 0.0).clamp(0.0, std::f64::consts::FRAC_PI_2);
        let [lo, hi] = analysis.range.map(|v| finite_or(v, 0.0));
        let (sin_a, cos_a) = angle.psin_cos();
        let mut out = Self {
            mode: analysis.mode,
            count,
            width,
            offset,
            sin_min: min.psin(),
            axis: Vec3::ZERO,
            first: Vec3::ZERO,
            second: Vec3::ZERO,
            point: Vec3::ZERO,
            spacing: 1.0,
            lo,
            hi,
            grazing: finite_or(analysis.grazing, GRAZING).clamp(1e-3, 0.5),
            perspective: matches!(axes.projection, crate::Projection::Perspective { .. }),
            colors: analysis.colors,
        };
        match analysis.mode {
            AnalysisMode::Zebra => {
                let d = axes.right * cos_a + axes.up * sin_a;
                let b1 = -axes.forward;
                out.axis = d;
                out.first = b1;
                out.second = d.cross(b1);
            }
            AnalysisMode::Highlight => {
                let (centre, diagonal, top) = match bounds.corners() {
                    Some((min, max)) => {
                        let d = (max - min).length();
                        (
                            (min + max) * 0.5,
                            if d.is_finite() && d > 0.0 { d } else { 1.0 },
                            max.z,
                        )
                    }
                    None => (Vec3::ZERO, 1.0, 0.0),
                };
                let a = Vec3::new(centre.x, centre.y, top + 0.25 * diagonal);
                out.axis = Vec3::new(cos_a, sin_a, 0.0);
                out.second = Vec3::new(0.0, 0.0, 1.0);
                out.first = out.second.cross(out.axis);
                out.point = a - axes.eye;
                out.spacing = diagonal / count;
            }
            AnalysisMode::Draft => {
                out.axis = analysis
                    .pull
                    .normalized()
                    .unwrap_or(Vec3::new(0.0, 0.0, 1.0));
            }
            AnalysisMode::Curvature => {}
        }
        out
    }

    /// The colour of a sample: `normal` as interpolated (unit, not turned
    /// to the viewer; `None` where there is none), `to_viewer` the unit
    /// vector from the point to the eye, `rel` the point relative to the
    /// eye, `value` the interpolated value per vertex.
    /// Stripes are point sampled; [`Resolved::color_at`] filters them.
    pub fn color(&self, normal: Option<Vec3>, to_viewer: Vec3, rel: Vec3, value: f64) -> [u8; 3] {
        self.color_at(normal, to_viewer, rel, value, None)
    }

    /// [`Resolved::color`] with the stripes filtered over the sample's
    /// footprint (`slopes`, see the module); `None` point samples them.
    pub fn color_at(
        &self,
        normal: Option<Vec3>,
        to_viewer: Vec3,
        rel: Vec3,
        value: f64,
        slopes: Option<&Slopes>,
    ) -> [u8; 3] {
        let c = &self.colors;
        let Some(n) = normal else {
            return c.none;
        };
        let facing = n.dot(to_viewer);
        let lit = |rgb: [u8; 3]| {
            let k = ANALYSIS_AMBIENT + ANALYSIS_DIFFUSE * facing.abs().min(1.0);
            rgb.map(|v| (f64::from(v) * k).round().clamp(0.0, 255.0) as u8)
        };
        match self.mode {
            AnalysisMode::Zebra | AnalysisMode::Highlight => {
                // Without a phase (a highlight line's grazing normal) the
                // stripes are finer than anything: their mean.
                let dark = match self.phase(n, to_viewer, rel) {
                    Some(phase) => {
                        let footprint =
                            slopes.map_or(0.0, |s| self.footprint(n, to_viewer, rel, s));
                        dark_share(phase, footprint, self.width)
                    }
                    None => self.width,
                };
                if dark >= 1.0 {
                    c.dark
                } else if dark <= 0.0 {
                    c.light
                } else {
                    let mut out = [0u8; 3];
                    for (o, (&d, &l)) in out.iter_mut().zip(c.dark.iter().zip(c.light.iter())) {
                        let (d, l) = (f64::from(d), f64::from(l));
                        *o = (l + (d - l) * dark).round().clamp(0.0, 255.0) as u8;
                    }
                    out
                }
            }
            AnalysisMode::Curvature => {
                let span = self.hi - self.lo;
                if !value.is_finite() {
                    return lit(c.none);
                }
                let t = if span.abs() > 0.0 && span.is_finite() {
                    (value - self.lo) / span
                } else {
                    0.5
                };
                lit(ramp(&c.ramp, t))
            }
            AnalysisMode::Draft => {
                let s = n.dot(self.axis);
                lit(if s >= self.sin_min {
                    c.draft_in
                } else if s <= -self.sin_min {
                    c.draft_out
                } else {
                    c.draft_shallow
                })
            }
        }
    }

    /// Whether the mode draws stripes (zebra, highlight lines), which
    /// [`Resolved::color_at`] filters over a footprint.
    pub fn has_stripes(&self) -> bool {
        matches!(self.mode, AnalysisMode::Zebra | AnalysisMode::Highlight)
    }

    /// The stripe phase of zebra or highlight lines at a sample (see the
    /// module), in periods; `None` where it has none.
    pub fn phase(&self, n: Vec3, to_viewer: Vec3, rel: Vec3) -> Option<f64> {
        match self.mode {
            AnalysisMode::Zebra => {
                let n = if n.dot(to_viewer) < 0.0 { -n } else { n };
                let r = n * (2.0 * n.dot(to_viewer)) - to_viewer;
                let phi = r.dot(self.second).patan2(r.dot(self.first));
                Some(phi / std::f64::consts::TAU * self.count + self.offset)
            }
            AnalysisMode::Highlight => {
                let nm = n.dot(self.second);
                if nm.is_nan() || nm.abs() < self.grazing {
                    return None;
                }
                let t = (self.point - rel).dot(self.second) / nm;
                let q = rel + n * t;
                let phase = (q - self.point).dot(self.first) / self.spacing + self.offset;
                phase.is_finite().then_some(phase)
            }
            AnalysisMode::Curvature | AnalysisMode::Draft => None,
        }
    }

    /// The footprint of a sample in phase, periods: `|∂p/∂x| + |∂p/∂y|`
    /// over the samples' grid, from the attributes' `slopes` chained
    /// through [`Resolved::phase`] (`n` the unit normal, `to_viewer` and
    /// `rel` as there). Infinite where the phase turns faster than can be
    /// told (zebra's reflected ray along the tubes' axis); zero for the
    /// modes without stripes.
    pub fn footprint(&self, n: Vec3, to_viewer: Vec3, rel: Vec3, slopes: &Slopes) -> f64 {
        // The unit normal's derivative: the raw one's, less its part along
        // the normal, over the raw length.
        let len = slopes.normal.length();
        let unit = |d: Vec3| {
            if len > 0.0 && len.is_finite() {
                (d - n * n.dot(d)) * (1.0 / len)
            } else {
                Vec3::ZERO
            }
        };
        let mut sum = 0.0;
        for (&dn, &drel) in slopes.d_normal.iter().zip(slopes.d_rel.iter()) {
            let dn = unit(dn);
            let d = match self.mode {
                AnalysisMode::Zebra => {
                    let (m, dm) = if n.dot(to_viewer) < 0.0 {
                        (-n, -dn)
                    } else {
                        (n, dn)
                    };
                    let v = to_viewer;
                    // v = −rel / |rel| in perspective, constant otherwise.
                    let dv = if self.perspective {
                        let l = rel.length();
                        if l > 0.0 && l.is_finite() {
                            (drel - v * v.dot(drel)) * (-1.0 / l)
                        } else {
                            Vec3::ZERO
                        }
                    } else {
                        Vec3::ZERO
                    };
                    let s = m.dot(v);
                    let ds = dm.dot(v) + m.dot(dv);
                    let r = m * (2.0 * s) - v;
                    let dr = m * (2.0 * ds) + dm * (2.0 * s) - dv;
                    let (x, y) = (r.dot(self.first), r.dot(self.second));
                    let (dx, dy) = (dr.dot(self.first), dr.dot(self.second));
                    let q = x * x + y * y;
                    if q.is_nan() || q <= 0.0 {
                        return f64::INFINITY;
                    }
                    (x * dy - y * dx) / q / std::f64::consts::TAU * self.count
                }
                AnalysisMode::Highlight => {
                    let m = self.second;
                    let nm = n.dot(m);
                    if nm.is_nan() || nm == 0.0 {
                        return f64::INFINITY;
                    }
                    let t = (self.point - rel).dot(m) / nm;
                    let dt = (-drel.dot(m) - t * dn.dot(m)) / nm;
                    let dq = drel + dn * t + n * dt;
                    dq.dot(self.first) / self.spacing
                }
                AnalysisMode::Curvature | AnalysisMode::Draft => 0.0,
            };
            sum += d.abs();
        }
        if sum.is_nan() { f64::INFINITY } else { sum }
    }
}

/// The ramp's colour at `t` (0 to 1, clamped): linear between five
/// evenly spaced stops, rounded.
pub fn ramp(stops: &[[u8; 3]; 5], t: f64) -> [u8; 3] {
    let t = if t.is_finite() {
        t.clamp(0.0, 1.0)
    } else {
        0.5
    };
    let x = t * 4.0;
    let k = (x.floor() as usize).min(3);
    let f = x - k as f64;
    let (a, b) = match (stops.get(k), stops.get(k + 1)) {
        (Some(a), Some(b)) => (*a, *b),
        _ => return stops.last().copied().unwrap_or([128, 128, 128]),
    };
    let mut out = [0u8; 3];
    for (o, (&x, &y)) in out.iter_mut().zip(a.iter().zip(b.iter())) {
        *o = (f64::from(x) + (f64::from(y) - f64::from(x)) * f)
            .round()
            .clamp(0.0, 255.0) as u8;
    }
    out
}

/// Draws `polylines` (curvature combs) over `frame`, the picture seen
/// with `camera`, in `rgb`: one pixel wide, at half-pixel steps, left out
/// where the frame's surface is nearer than the point by more than
/// `slack` model units (hidden behind a body). For pictures made without
/// a window (`view.render`); the window draws its lines itself.
pub fn draw_lines(
    frame: &mut crate::Frame,
    camera: &crate::Camera,
    polylines: &[Vec<Vec3>],
    rgb: [u8; 3],
    slack: f64,
) -> Result<(), crate::RenderError> {
    let view = crate::Viewpoint::new(camera, frame.width, frame.height)?;
    let (w, h) = (frame.width as usize, frame.height as usize);
    // At most 2^24 steps in all: a line is a few hundred pixels.
    let mut budget: u64 = 1 << 24;
    for line in polylines {
        for pair in line.windows(2) {
            let [a, b] = [pair.first(), pair.get(1)];
            let (Some(&a), Some(&b)) = (a, b) else {
                continue;
            };
            // In perspective, the part in front of the eye: an end behind
            // it (or at it) moves along the segment to a millionth of the
            // other end's distance, as the mesh's near plane is at most a
            // millionth of the farthest (`clip`).
            let (a, b) = if view.is_perspective() {
                let (da, db) = (view.distance(a), view.distance(b));
                let near = 1e-6 * da.max(db);
                if near.is_nan() || near <= 0.0 || (da < near && db < near) {
                    continue;
                }
                let toward = |from: Vec3, d_from: f64, to: Vec3, d_to: f64| {
                    // `from` is in front, `to` is not.
                    let t = ((d_from - near) / (d_from - d_to)).clamp(0.0, 1.0);
                    from + (to - from) * t
                };
                if da < near {
                    (toward(b, db, a, da), b)
                } else if db < near {
                    (a, toward(a, da, b, db))
                } else {
                    (a, b)
                }
            } else {
                (a, b)
            };
            let (Some(pa), Some(pb)) = (view.project(a), view.project(b)) else {
                continue;
            };
            let pixels = (pb[0] - pa[0]).phypot(pb[1] - pa[1]);
            if !pixels.is_finite() {
                continue;
            }
            let steps = (pixels * 2.0).ceil().clamp(1.0, 65536.0) as u64;
            budget = budget.saturating_sub(steps);
            if budget == 0 {
                return Ok(());
            }
            for k in 0..=steps {
                let t = k as f64 / steps as f64;
                let p = a + (b - a) * t;
                let Some([x, y, dist]) = view.project(p) else {
                    continue;
                };
                if !(x >= 0.0 && y >= 0.0) {
                    continue;
                }
                let (x, y) = (x as usize, y as usize);
                if x >= w || y >= h {
                    continue;
                }
                let i = y * w + x;
                let front = frame.depth.get(i).copied().unwrap_or(f64::INFINITY);
                if front + slack < dist {
                    continue;
                }
                if let Some(px) = frame.rgb.get_mut(3 * i..3 * i + 3) {
                    px.copy_from_slice(&rgb);
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Camera, Projection};

    fn axes() -> ViewAxes {
        Camera {
            eye: Vec3::new(0.0, 0.0, 10.0),
            target: Vec3::ZERO,
            up: Vec3::new(0.0, 1.0, 0.0),
            projection: Projection::Orthographic { half_height: 2.0 },
        }
        .axes()
        .unwrap()
    }

    #[test]
    fn zebra_phase_on_a_sphere_is_twice_the_angle_along_the_meridian() {
        let r = Resolved::new(&Analysis::new(AnalysisMode::Zebra), &axes(), &Bbox3::EMPTY);
        let v = Vec3::new(0.0, 0.0, 1.0);
        for k in 1..8 {
            let theta = 0.1 * f64::from(k);
            let n = Vec3::new(0.0, theta.psin(), theta.pcos());
            let phase = r.phase(n, v, Vec3::ZERO).unwrap();
            let expect = 2.0 * theta / std::f64::consts::TAU * 16.0;
            assert!((phase.abs() - expect).abs() < 1e-12, "{phase} {expect}");
        }
    }

    #[test]
    fn dark_share_is_the_tent_filtered_square_wave() {
        // A point sample without a footprint.
        assert_eq!(dark_share(3.2, 0.0, 0.3), 1.0);
        assert_eq!(dark_share(3.4, 0.0, 0.3), 0.0);
        // A tent a whole number of periods wide is the mean wherever it
        // starts.
        for p in [0.0, 0.17, 0.3, 0.71, 1234.5] {
            for f in [1.0, 2.0, 7.0] {
                assert!((dark_share(p, f, 0.3) - 0.3).abs() < 1e-9, "{p} {f}");
            }
        }
        // Not a number, infinite, beyond FLAT_FOOTPRINT: the mean.
        for f in [f64::NAN, f64::INFINITY, -1.0, 2.0 * FLAT_FOOTPRINT] {
            assert_eq!(dark_share(0.1, f, 0.3), 0.3);
        }
        // Against the wave averaged with the tent by brute force.
        for &(p, f, w) in &[
            (0.05, 0.2, 0.3),
            (0.29, 0.1, 0.3),
            (0.9, 0.5, 0.5),
            (2.6, 1.7, 0.25),
            (0.4, 3.3, 0.7),
        ] {
            let n = 200_000;
            let (mut dark, mut total) = (0.0, 0.0);
            for k in 0..n {
                let s = -1.0 + 2.0 * (f64::from(k) + 0.5) / f64::from(n);
                let weight = 1.0 - s.abs();
                let t = p + s * f;
                total += weight;
                if t - t.floor() < w {
                    dark += weight;
                }
            }
            let brute = dark / total;
            assert!((dark_share(p, f, w) - brute).abs() < 1e-4, "{p} {f} {w}");
        }
        // Far from an edge the tent sees one colour: the edge does not move.
        assert!((dark_share(0.15, 0.1, 0.3) - 1.0).abs() < 1e-12);
        assert!(dark_share(0.65, 0.2, 0.3).abs() < 1e-12);
        // At an edge, half.
        assert!((dark_share(0.3, 0.2, 0.3) - 0.5).abs() < 1e-12);
    }

    /// `scene.wgsl`'s `analysis_dark_share` in `f32`, line by line.
    fn dark_share_f32(phase: f32, footprint: f32, width: f32) -> f32 {
        fn wave(t: f32, w: f32) -> f32 {
            let whole = t.floor();
            let u = t - whole;
            let mut inner = (1.0 - w) * w * w / 2.0 + w * ((u - w) - (u * u - w * w) / 2.0);
            if u < w {
                inner = (1.0 - w) * u * u / 2.0;
            }
            whole * w * (1.0 - w) / 2.0 + inner
        }
        fn beyond(u: f32) -> f32 {
            let u = u.clamp(-1.0, 1.0);
            if u >= 0.0 {
                (1.0 - u) * (1.0 - u) / 2.0
            } else {
                1.0 - (1.0 + u) * (1.0 + u) / 2.0
            }
        }
        if !phase.is_finite() {
            return width;
        }
        if width <= 0.0 {
            return 0.0;
        }
        if width >= 1.0 {
            return 1.0;
        }
        let x = phase - phase.floor();
        let point = if x < width { 1.0 } else { 0.0 };
        if footprint == 0.0 {
            return point;
        }
        // ANALYSIS_FLAT_FOOTPRINT, `FLAT_FOOTPRINT` in `f32`.
        if !(footprint > 0.0 && footprint <= 1e6) {
            return width;
        }
        let f = footprint;
        let (to_start, to_end, to_next) = (x, (x - width).abs(), 1.0 - x);
        if f <= to_start.min(to_end).min(to_next) {
            return point;
        }
        if 2.0 * f < width.min(1.0 - width) {
            return if to_end <= to_start.min(to_next) {
                1.0 - beyond((width - x) / f)
            } else if to_start <= to_next {
                beyond(-x / f)
            } else {
                beyond(to_next / f)
            };
        }
        let second = wave(x + f, width) - 2.0 * wave(x, width) + wave(x - f, width);
        (width + second / (f * f)).clamp(0.0, 1.0)
    }

    /// The tent's average of the wave, by brute force, about `phase` given
    /// as a period's fraction (`x`) for a small footprint.
    fn brute_tent(x: f64, f: f64, w: f64) -> f64 {
        let n = 20_000;
        let (mut dark, mut total) = (0.0, 0.0);
        for k in 0..n {
            let s = -1.0 + 2.0 * (f64::from(k) + 0.5) / f64::from(n);
            let weight = 1.0 - s.abs();
            let t = x + s * f;
            total += weight;
            if t - t.floor() < w {
                dark += weight;
            }
        }
        dark / total
    }

    /// Small footprints (the reviewer's finding on 9cc2c64f): the second
    /// difference cancelled, in `f32` at 10⁻⁴ and in `f64` at 10⁻⁸ (a plane
    /// in orthographic zebra, whose footprint is 10⁻¹⁷ of rounding, came
    /// out the mean). Now the point sample away from an edge, exact, and
    /// the tent's share at an edge, from 10⁻¹² to 10⁻³, in `f64` and in the
    /// GPU's `f32` alike.
    #[test]
    fn dark_share_holds_at_small_footprints() {
        let w = 0.3;
        for f in [
            1e-3, 1e-4, 1e-5, 1e-6, 1e-7, 1e-8, 1e-9, 1e-10, 1e-11, 1e-12,
        ] {
            // Away from every edge: the point sample, exactly.
            for x in [0.05, 0.15, 0.29, 0.31, 0.6, 0.99] {
                let expect = if x < w { 1.0 } else { 0.0 };
                assert_eq!(dark_share(x, f, w), expect, "{x} {f}");
                assert_eq!(dark_share(x + 7.0, f, w), expect, "{x} {f}");
            }
            // At an edge: the brute-force tent.
            for k in [-0.9, -0.5, -0.1, 0.0, 0.3, 0.7] {
                for edge in [w, 1.0] {
                    let x = edge + k * f;
                    let brute = brute_tent(x, f, w);
                    let x = x - x.floor();
                    assert!(
                        (dark_share(x, f, w) - brute).abs() < 1e-3,
                        "{x} {f} {k} {edge}"
                    );
                }
            }
        }
        // The GPU's `f32` path, from 10⁻⁷ (a few ulp of a phase near 1) up.
        let w32 = 0.3f32;
        for f in [1e-3f32, 1e-4, 1e-5, 1e-6, 1e-7] {
            for x in [0.05f32, 0.15, 0.29, 0.31, 0.6, 0.99] {
                let expect = if x < w32 { 1.0 } else { 0.0 };
                assert_eq!(dark_share_f32(x, f, w32), expect, "f32 {x} {f}");
                assert_eq!(dark_share_f32(x + 5.0, f, w32), expect, "f32 {x} {f}");
            }
            for k in [-0.9f32, -0.5, -0.1, 0.0, 0.3, 0.7] {
                for edge in [w32, 1.0] {
                    let x = edge + k * f;
                    let truth = dark_share(f64::from(x), f64::from(f), f64::from(w32));
                    let got = f64::from(dark_share_f32(x, f, w32));
                    assert!((got - truth).abs() < 1e-3, "f32 {x} {f} {k}: {got} {truth}");
                }
            }
        }
        // Large footprints agree too (the second difference in `f32`).
        for &(p, f) in &[(0.2f32, 0.2f32), (3.7, 1.3), (0.95, 40.0)] {
            let truth = dark_share(f64::from(p), f64::from(f), 0.3);
            let got = f64::from(dark_share_f32(p, f, 0.3));
            assert!((got - truth).abs() < 1e-4, "f32 {p} {f}: {got} {truth}");
        }
        // A phase that is not finite is the mean, whatever the footprint.
        assert_eq!(dark_share(f64::NAN, 0.0, w), w);
        assert_eq!(dark_share(f64::INFINITY, 1e-9, w), w);
    }

    /// The footprint's chain rule against a central difference of the
    /// phase, along a path of the interpolants (the raw normal and the
    /// point; the direction to the viewer follows the point in
    /// perspective).
    #[test]
    fn the_footprint_is_the_phase_derivative() {
        let bounds = Bbox3::from_points([Vec3::new(-1.0, -1.0, -1.0), Vec3::new(1.0, 1.0, 1.0)]);
        let perspective = Camera {
            eye: Vec3::new(2.0, -6.0, 4.0),
            target: Vec3::ZERO,
            up: Vec3::new(0.0, 0.0, 1.0),
            projection: Projection::Perspective { fov_y_deg: 35.0 },
        }
        .axes()
        .unwrap();
        let raw0 = Vec3::new(0.3, -0.8, 0.45);
        let d_raw = Vec3::new(0.02, 0.013, -0.031);
        let p0 = Vec3::new(0.2, -0.5, 0.6);
        let d_p = Vec3::new(0.004, -0.002, 0.003);
        for (mode, axes) in [
            (AnalysisMode::Zebra, axes()),
            (AnalysisMode::Zebra, perspective),
            (AnalysisMode::Highlight, perspective),
        ] {
            let mut a = Analysis::new(mode);
            a.direction = 0.4;
            let r = Resolved::new(&a, &axes, &bounds);
            let at = |s: f64| {
                let n = (raw0 + d_raw * s).normalized().unwrap();
                let p = p0 + d_p * s;
                let v = if r.perspective {
                    (axes.eye - p).normalized().unwrap()
                } else {
                    -axes.forward
                };
                (n, v, p - axes.eye)
            };
            let h = 1e-5;
            let phase = |s: f64| {
                let (n, v, rel) = at(s);
                r.phase(n, v, rel).unwrap()
            };
            let numeric = (phase(h) - phase(-h)) / (2.0 * h);
            let (n, v, rel) = at(0.0);
            let slopes = Slopes {
                normal: raw0,
                d_normal: [d_raw, Vec3::ZERO],
                d_rel: [d_p, Vec3::ZERO],
            };
            let analytic = r.footprint(n, v, rel, &slopes);
            assert!(
                (analytic - numeric.abs()).abs() < 1e-6 * (1.0 + numeric.abs()),
                "{mode:?}: {analytic} against {numeric}"
            );
            assert!(analytic > 1e-3, "{mode:?}: {analytic}");
        }
    }

    #[test]
    fn draft_classes_by_the_sine_of_the_minimum() {
        let mut a = Analysis::new(AnalysisMode::Draft);
        a.min_draft = 2f64.to_radians();
        let r = Resolved::new(&a, &axes(), &Bbox3::EMPTY);
        // The viewer square to the normal: lit by the ambient share only.
        let lit = |c: [u8; 3]| c.map(|x| (f64::from(x) * ANALYSIS_AMBIENT).round() as u8);
        let side = |deg: f64| {
            let s = f64::to_radians(deg);
            let n = Vec3::new(s.pcos(), 0.0, s.psin());
            r.color(Some(n), Vec3::new(0.0, 1.0, 0.0), Vec3::ZERO, 0.0)
        };
        let c = AnalysisColors::default();
        assert_eq!(side(3.0), lit(c.draft_in));
        assert_eq!(side(-3.0), lit(c.draft_out));
        assert_eq!(side(1.0), lit(c.draft_shallow));
        assert_eq!(side(0.0), lit(c.draft_shallow));
    }

    #[test]
    fn ramp_ends_and_middle() {
        let c = AnalysisColors::default();
        assert_eq!(ramp(&c.ramp, 0.0), c.ramp[0]);
        assert_eq!(ramp(&c.ramp, 1.0), c.ramp[4]);
        assert_eq!(ramp(&c.ramp, 0.5), c.ramp[2]);
        assert_eq!(ramp(&c.ramp, -3.0), c.ramp[0]);
        assert_eq!(ramp(&c.ramp, f64::NAN), c.ramp[2]);
    }

    #[test]
    fn highlight_lines_do_not_depend_on_the_eye() {
        let a = Analysis::new(AnalysisMode::Highlight);
        let bounds = Bbox3::from_points([Vec3::new(-1.0, -1.0, -1.0), Vec3::new(1.0, 1.0, 1.0)]);
        let one = axes();
        let mut other = one;
        other.eye = Vec3::new(3.0, -2.0, 7.0);
        let (r1, r2) = (
            Resolved::new(&a, &one, &bounds),
            Resolved::new(&a, &other, &bounds),
        );
        let p = Vec3::new(0.3, 0.2, 0.9);
        let n = Vec3::new(0.2, 0.1, 1.0).normalized().unwrap();
        let v = Vec3::new(0.0, 0.0, 1.0);
        let p1 = r1.phase(n, v, p - one.eye).unwrap();
        let p2 = r2.phase(n, v, p - other.eye).unwrap();
        assert!((p1 - p2).abs() < 1e-12);
    }
}
