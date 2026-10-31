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
//!   line runs along the plane and the sample has no phase (the neutral
//!   colour).
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
/// plane: no phase. The phase grows as `stripes / |N·M|` periods, and its
/// rounding with it: at most 512 / 0.05 ≈ 10⁴ periods keeps the `f32`
/// phase of the GPU within about 10⁻³ of a period of the CPU's, and the
/// stripes below this are some twenty times finer than over the model's
/// top, finer than a pixel on any view that shows them whole.
pub const GRAZING: f64 = 0.05;
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
    /// Where there is no value (a curvature the surface does not have, a
    /// highlight line's grazing normal, no normal).
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
    pub colors: AnalysisColors,
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
    pub fn color(&self, normal: Option<Vec3>, to_viewer: Vec3, rel: Vec3, value: f64) -> [u8; 3] {
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
            AnalysisMode::Zebra | AnalysisMode::Highlight => match self.phase(n, to_viewer, rel) {
                Some(phase) => {
                    if phase - phase.floor() < self.width {
                        c.dark
                    } else {
                        c.light
                    }
                }
                None => c.none,
            },
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
