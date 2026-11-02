//! The picture in the viewport: the scene's bodies shaded by a renderer
//! (the GPU's or the CPU reference renderer, see [`crate::backend`]), and
//! the lines drawn over that image (the edges of the bodies, sketches, the
//! world axes) projected with the same camera.
//!
//! The lines are tested against the renderer's depth buffer, a pixel at a
//! time: the part of a body's edge that lies behind a body is left out,
//! and the part of a sketch behind a body is marked hidden, which is
//! drawn dimmed, so a sketch inside or behind a part can still be found.
//! The world axes are a gizmo and always drawn in full.
//!
//! No egui here: the window paints the [`Overlay`] with its painter, and
//! `ui.screenshot` rasterises the same overlay into the same image, so the
//! two show the same thing.

use crate::backend::{Backend, BodyRenderer};
use crate::camera::{Projected, Projector};
use crate::state::{CameraState, EdgeRef, VertexRef};
use crate::theme::{EdgeColors, Rgb, Tokens};
use craft3d_engine::{Scene, SceneDatum, SceneDatumShape, SceneSketch};
use craft3d_math::Portable;
use craft3d_math::Vec3;
use craft3d_render::{Frame, Highlight, RenderParams, SUBPIXEL_GRAIN};

/// Smallest and largest side of a viewport image, in pixels.
pub const MIN_SIDE: u32 = 16;
pub const MAX_SIDE: u32 = 4096;

/// The world axes are drawn this long, as a fraction of the camera's
/// distance to its target: the same size on screen at any zoom.
pub const AXIS_LENGTH: f64 = 0.06;

/// What a line of the overlay is, which decides its colour and width.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    /// Sketch geometry that is part of a profile.
    Sketch,
    /// Construction geometry of a sketch.
    Construction,
    /// Geometry of the selected feature's sketch.
    Selected,
    /// A world axis at the origin: 0 is X, 1 is Y, 2 is Z.
    Axis(usize),
    /// An edge of a body.
    Edge,
    /// An edge or a vertex that is pointed at ([`Marks`]).
    Highlight,
    /// The outline of a datum plane, a datum axis, or (as a dot) a datum
    /// point.
    Datum,
    /// A point's path over the motion (`view.trace`).
    Trace,
    /// A curvature comb's spike or its line through the tips
    /// (`craft3d_engine::analysis::combs`).
    Comb,
}

/// The surface analysis the viewport draws (`craft3d_render::analysis`),
/// made by the shell from its `AnalysisState` and the shown scene: the
/// renderers' analysis (none for combs alone), the curvature map's values
/// per vertex of the scene's mesh (empty for the other modes), the
/// combs, and what the legend says.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewAnalysis {
    pub analysis: Option<craft3d_render::analysis::Analysis>,
    pub values: std::sync::Arc<Vec<f32>>,
    /// The mesh the values belong to when it is not the scene's: the
    /// curvature map's, refined where the curvature changes fast
    /// (`craft3d_engine::analysis::refined_map`, as `view.render` draws it).
    pub mesh: Option<std::sync::Arc<craft3d_tess::Mesh>>,
    pub combs: Vec<Vec<Vec3>>,
    /// Which curvature the map shows (`gaussian`, `mean`, `max`).
    pub curvature: String,
    /// The model's length unit, for the legend.
    pub unit: String,
    pub legend: bool,
}

impl ViewAnalysis {
    /// The mesh the analysis is drawn on: its own, or `scene`'s.
    pub fn mesh_or<'a>(&'a self, scene: &'a craft3d_tess::Mesh) -> &'a craft3d_tess::Mesh {
        match &self.mesh {
            Some(mesh) if !self.values.is_empty() => mesh,
            _ => scene,
        }
    }

    /// What tells uploads apart: the values' identity (0 for none).
    pub fn values_key(&self) -> usize {
        if self.values.is_empty() {
            0
        } else {
            std::sync::Arc::as_ptr(&self.values) as usize
        }
    }
}

/// A row of a legend: its colours (side by side) and its words.
pub type LegendRow = (Vec<[u8; 3]>, String);

/// The legend of an analysis as rows to draw: a title, and per row a
/// colour (or a ramp's stops for the curvature map) and its words.
pub fn legend_rows(view: &ViewAnalysis) -> Option<(String, Vec<LegendRow>)> {
    use craft3d_render::analysis::AnalysisMode;
    let a = view.analysis.as_ref().filter(|_| view.legend)?;
    let c = &a.colors;
    match a.mode {
        AnalysisMode::Curvature => {
            let (name, power) = match view.curvature.as_str() {
                "mean" => ("Mean curvature", "1/"),
                "max" => ("Largest |curvature|", "1/"),
                _ => ("Gaussian curvature", "1/²"),
            };
            let unit = if power == "1/²" {
                format!("1/{}²", view.unit)
            } else {
                format!("1/{}", view.unit)
            };
            let [lo, hi] = a.range;
            let mid = 0.5 * (lo + hi);
            Some((
                format!("{name} ({unit})"),
                vec![
                    (vec![c.ramp[4]], format!("{hi:.4}")),
                    (vec![c.ramp[2]], format!("{mid:.4}")),
                    (vec![c.ramp[0]], format!("{lo:.4}")),
                ],
            ))
        }
        AnalysisMode::Draft => {
            let min = a.min_draft.to_degrees();
            Some((
                "Draft".to_string(),
                vec![
                    (vec![c.draft_in], format!("≥ {min:.1}° along the pull")),
                    (vec![c.draft_shallow], format!("below {min:.1}°")),
                    (vec![c.draft_out], format!("≥ {min:.1}° against it")),
                ],
            ))
        }
        AnalysisMode::Zebra => Some((
            format!("Zebra: {} stripes", a.stripes),
            vec![(vec![c.dark, c.light], "reflected stripes".to_string())],
        )),
        AnalysisMode::Highlight => Some((
            format!("Highlight lines: {}", a.stripes),
            vec![(vec![c.dark, c.light], "view-independent".to_string())],
        )),
    }
}

/// A line in image pixels (x right, y down, from the top-left corner).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OverlayLine {
    pub from: [f64; 2],
    pub to: [f64; 2],
    pub kind: LineKind,
    /// Behind a body: drawn dimmed.
    pub hidden: bool,
    /// For an edge of a body, how it is inked; `None` for every other
    /// line, and for the outline of a section's cap.
    pub edge: Option<EdgeInk>,
}

/// How an edge of a body is inked: in a shade of the body's own colour
/// ([`Tokens::edge_colors`]), wider and darker where it is the body's
/// outline against what lies behind it, and faded where it is short on
/// screen or where edges crowd ([`edge_fade`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EdgeInk {
    /// The body's own colour (its appearance); `None` for the theme's.
    pub body: Option<Rgb>,
    /// The edge's extent on screen in pixels: the diagonal of its box in
    /// space, projected. Infinite when a corner of the box is not in front
    /// of the eye.
    pub extent: f64,
    /// This stretch of the edge is the body's outline: the surface on one
    /// side of it steps back ([`surface_steps_back`]).
    pub silhouette: bool,
    /// This stretch goes on from the line before it, on the same segment
    /// of the edge (it was cut there only because the outline starts or
    /// stops): the density counts the two as one line, as the GPU does.
    pub joined: bool,
    /// The stretch starts, and ends, inside its segment of the edge (where
    /// the edge goes behind a body, or the outline starts or stops)
    /// rather than at a point of the edge: the edge is cut square there,
    /// as the GPU cuts it where the point of the segment under a pixel is
    /// hidden, not rounded.
    pub cut: [bool; 2],
    /// The distances along the view of the stretch's two ends (`from`,
    /// `to`), for the lens's blur along it ([`Overlay::blur`]).
    pub depth: [f64; 2],
}

impl OverlayLine {
    /// The colour the line is drawn in: an edge of a body in its body's
    /// shade, every other line in its kind's.
    pub fn color(&self, tokens: &Tokens) -> Rgb {
        match self.edge {
            Some(edge) => edge.colors(tokens).pick(edge.silhouette),
            None => self.kind.color(tokens),
        }
    }

    /// In points: an outline is drawn wider than an edge between faces.
    pub fn width(&self, tokens: &Tokens) -> f32 {
        match self.edge {
            Some(EdgeInk {
                silhouette: true, ..
            }) => tokens.edge_silhouette_width,
            _ => self.kind.width(tokens),
        }
    }
}

impl EdgeInk {
    pub fn colors(&self, tokens: &Tokens) -> EdgeColors {
        tokens.edge_colors(self.body.unwrap_or(tokens.body))
    }
}

impl EdgeColors {
    /// The outline's colour or the inner edges'.
    pub fn pick(self, silhouette: bool) -> Rgb {
        if silhouette {
            self.silhouette
        } else {
            self.inner
        }
    }
}

/// A dot in image pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OverlayPoint {
    pub at: [f64; 2],
    pub kind: LineKind,
    /// Behind a body: drawn dimmed.
    pub hidden: bool,
}

/// Everything drawn over the rendered image.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Overlay {
    pub lines: Vec<OverlayLine>,
    /// The point entities of sketches, as dots.
    pub points: Vec<OverlayPoint>,
    /// The studio look's lens, when the bodies' edges follow it
    /// (`look.edges: soft` with depth of field): each edge then fades by
    /// [`EdgeBlur::fade`] at its distance. `None`: drawn as ever.
    pub blur: Option<EdgeBlur>,
}

/// How much the studio look's lens blurs what lies at each distance, for
/// the bodies' edges over its picture (`look.edges: soft`). The edges are
/// drawn after the picture is shaded, focused and bloomed, so they are not
/// blurred with it; instead each pixel of an edge keeps the share of its
/// ink that a line of its width keeps at its middle once spread over the
/// lens's circle of confusion: a line `w` pixels wide over a disc of
/// radius `c` covers about `2 w / (π c)` of it when the disc is much
/// wider, which `w / (w + π c / 2)` follows from 1 in focus down. In focus
/// an edge is crisp; far from it, it fades into the blurred picture. The
/// GPU's lines (`lines.wgsl`) take the same numbers from the same lens.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EdgeBlur {
    /// The blur radius in pixels of what lies infinitely far:
    /// `a h / (4 half zf)` (`craft3d_render::studio::polish::coc_radius`).
    pub scale: f64,
    /// The distance in focus, along the view.
    pub focus: f64,
    /// The largest blur radius, in pixels (`DOF_MAX_COC` of the height).
    pub cap: f64,
}

impl EdgeBlur {
    /// The blur of `look`'s lens for a picture `height` pixels high of a
    /// mesh within `bounds`, seen with `camera`; `None` without a lens,
    /// or where the lens blurs nothing ([`craft3d_render::studio::polish::Lens::new`]).
    pub fn new(
        look: &craft3d_render::studio::Look,
        camera: &craft3d_render::Camera,
        bounds: &craft3d_math::Bbox3,
        height: u32,
    ) -> Option<EdgeBlur> {
        let dof = look.dof.as_ref()?;
        let lens = craft3d_render::studio::polish::Lens::new(dof, camera, bounds, height as usize)?;
        let scale = lens.aperture * lens.height / (4.0 * lens.half * lens.focus);
        (scale.is_finite() && lens.focus.is_finite() && lens.cap.is_finite()).then_some(EdgeBlur {
            scale,
            focus: lens.focus,
            cap: lens.cap,
        })
    }

    /// The blur radius in pixels at view distance `z`, capped: the cap
    /// at or behind the eye, none for a distance that is not a number.
    pub fn radius(&self, z: f64) -> f64 {
        let c = if z > 0.0 {
            self.scale * (z - self.focus).abs() / z
        } else if z <= 0.0 {
            self.cap
        } else {
            0.0
        };
        if c.is_nan() { 0.0 } else { c.min(self.cap) }
    }

    /// The share of its ink a line `width` pixels wide keeps at view
    /// distance `z`: 1 in focus, falling as the blur grows.
    pub fn fade(&self, z: f64, width: f64) -> f64 {
        let c = self.radius(z);
        let w = if width.is_finite() {
            width.max(0.0)
        } else {
            0.0
        };
        if c > 0.0 {
            w / (w + std::f64::consts::FRAC_PI_2 * c)
        } else {
            1.0
        }
    }

    /// The fade of `line` at the fraction `t` of the way from its run's
    /// start, at the depths `ends` of that run: the reciprocal of the
    /// distance changes evenly along a line on the screen.
    pub fn fade_along(&self, ends: [f64; 2], t: f64, width: f64) -> f64 {
        let t = if t.is_finite() {
            t.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let [a, b] = ends;
        let z = if a > 0.0 && b > 0.0 {
            let inverse = 1.0 / a + (1.0 / b - 1.0 / a) * t;
            if inverse > 0.0 {
                1.0 / inverse
            } else {
                a + (b - a) * t
            }
        } else {
            a + (b - a) * t
        };
        self.fade(z, width)
    }
}

/// How opaque a line or dot is drawn, 0 to 255: full, or the theme's
/// opacity for what is hidden behind a body.
pub fn opacity(hidden: bool, tokens: &Tokens) -> u8 {
    if hidden { tokens.hidden_opacity } else { 255 }
}

impl LineKind {
    pub fn color(self, tokens: &Tokens) -> Rgb {
        match self {
            LineKind::Sketch => tokens.sketch,
            LineKind::Construction => tokens.sketch_construction,
            LineKind::Selected => tokens.sketch_selected,
            LineKind::Axis(axis) => tokens
                .axes
                .get(axis)
                .copied()
                .unwrap_or(tokens.viewport_text),
            LineKind::Edge => tokens.edge,
            LineKind::Highlight => tokens.face_highlight,
            LineKind::Datum => tokens.datum,
            LineKind::Trace => tokens.trace,
            LineKind::Comb => tokens.comb,
        }
    }

    /// In points.
    pub fn width(self, tokens: &Tokens) -> f32 {
        match self {
            LineKind::Sketch | LineKind::Selected => tokens.sketch_width,
            LineKind::Construction => tokens.construction_width,
            LineKind::Axis(_) => tokens.axis_width,
            LineKind::Edge => tokens.edge_width,
            LineKind::Highlight => tokens.highlight_width,
            LineKind::Datum => tokens.datum_width,
            LineKind::Trace => tokens.trace_width,
            LineKind::Comb => tokens.comb_width,
        }
    }

    /// The radius of a dot of this kind, in points.
    pub fn dot_radius(self, tokens: &Tokens) -> f32 {
        match self {
            LineKind::Highlight => tokens.highlight_radius,
            LineKind::Datum => tokens.datum_point_radius,
            _ => tokens.point_radius,
        }
    }
}

/// The section a frame is cut open with, made ready for the scene's mesh:
/// what the renderers take, and what the lines over the picture are cut
/// with.
#[derive(Clone, Debug, PartialEq)]
pub struct SectionCut {
    /// For the renderers (`RenderParams::section`).
    pub section: craft3d_render::Section,
    /// The same plane with the tolerance of the scene's mesh: the edges of
    /// the bodies are drawn where this keeps them.
    pub cut: craft3d_render::Cut,
    /// Where the plane crosses the surface of the bodies: the outline of
    /// the cap, drawn as edges.
    pub outline: std::sync::Arc<Vec<[Vec3; 2]>>,
}

impl SectionCut {
    /// `section` for the mesh of `scene`; `None` for a plane that is none.
    pub fn new(scene: &Scene, section: craft3d_render::Section) -> Option<Self> {
        let cut = craft3d_render::Cut::new(&section, &craft3d_render::section_bounds(&scene.mesh))?;
        Some(Self {
            section,
            cut,
            outline: std::sync::Arc::new(cut.outline(&scene.mesh)),
        })
    }
}

/// What a frame shows besides the bodies as they are. The edges and the
/// vertices that are pointed at, as the pieces of line the scene draws
/// them with: both paths of the viewport draw them from here (the lines
/// of [`Overlay`] and the segments of [`crate::direct::segments_marked`]),
/// over everything else, in the theme's highlight colour; the part behind
/// a body is drawn dimmed, like a sketch there, so that an edge an agent
/// points at is found also from the wrong side. And the section the
/// picture is cut open with: the bodies are rendered with it, their edges
/// are drawn where it keeps them, and its outline is drawn as edges. And
/// the datums that are shown (`UiState::show`): a plane as a translucent
/// square in the picture with its outline over it, an axis as a line, a
/// point as a dot, in the theme's datum colour, tested against the bodies
/// like a sketch (dimmed where a body is in front); the section does not
/// cut them.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Marks {
    pub lines: Vec<[Vec3; 2]>,
    /// The highlighted vertices, drawn as dots.
    pub points: Vec<Vec3>,
    /// The section, when the picture is cut open.
    pub cut: Option<SectionCut>,
    /// The datums drawn, in history order.
    pub datums: Vec<SceneDatum>,
    /// The sketches are not drawn (`UiState::show`).
    pub hide_sketches: bool,
    /// The bodies' edges, and the outline of the section's cap, are not
    /// drawn (`UiState::show`, or the studio look's `edges: off`). The
    /// lines and points of these marks are drawn all the same.
    pub hide_edges: bool,
    /// The bodies' edges fade where the studio look's lens blurs the
    /// model ([`EdgeBlur`]; `edges: soft`). Without a lens nothing
    /// changes.
    pub soft_edges: bool,
    /// The paths of points over the motion (`view.trace`), as polylines,
    /// tested against the bodies like a sketch.
    pub traces: Vec<Vec<Vec3>>,
    /// Curvature combs (`craft3d_engine::analysis::combs`), as polylines,
    /// tested like a trace.
    pub combs: Vec<Vec<Vec3>>,
}

impl Marks {
    /// True when nothing is pointed at (a section and datums are not
    /// marks that point).
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty() && self.points.is_empty()
    }

    /// These with the selection's marks `chosen` added, leaving out the
    /// lines and points these have already: what was clicked last is
    /// both pointed at and selected, and is drawn once.
    pub fn with_selection(mut self, chosen: &Marks) -> Marks {
        if chosen.lines.is_empty() && chosen.points.is_empty() {
            return self;
        }
        let key = |v: Vec3| [v.x.to_bits(), v.y.to_bits(), v.z.to_bits()];
        let lines: std::collections::BTreeSet<_> =
            self.lines.iter().map(|&[a, b]| (key(a), key(b))).collect();
        let points: std::collections::BTreeSet<_> = self.points.iter().map(|&p| key(p)).collect();
        self.lines.extend(
            chosen
                .lines
                .iter()
                .filter(|&&[a, b]| !lines.contains(&(key(a), key(b))))
                .copied(),
        );
        self.points.extend(
            chosen
                .points
                .iter()
                .filter(|&&p| !points.contains(&key(p)))
                .copied(),
        );
        self
    }

    /// Both together.
    pub fn with(mut self, other: Marks) -> Marks {
        self.lines.extend(other.lines);
        self.points.extend(other.points);
        self.datums.extend(other.datums);
        self.cut = self.cut.or(other.cut);
        self.hide_sketches |= other.hide_sketches;
        self.hide_edges |= other.hide_edges;
        self.soft_edges |= other.soft_edges;
        self.traces.extend(other.traces);
        self.combs.extend(other.combs);
        self
    }

    /// The section for the renderers.
    pub fn section(&self) -> Option<craft3d_render::Section> {
        self.cut.as_ref().map(|cut| cut.section)
    }

    /// The part of the segment from `a` to `b` that the section keeps:
    /// all of it without a section, `None` when it is cut away.
    pub fn kept(&self, a: Vec3, b: Vec3) -> Option<[Vec3; 2]> {
        match &self.cut {
            Some(section) => section.cut.clip_segment(a, b),
            None => Some([a, b]),
        }
    }

    /// The lines the section adds: the outline of its cap (none when the
    /// edges are hidden).
    pub fn outline(&self) -> &[[Vec3; 2]] {
        match &self.cut {
            Some(cut) if !self.hide_edges => cut.outline.as_slice(),
            _ => &[],
        }
    }

    /// The lines of the datums: the four sides of each plane's square
    /// and each axis, in history order.
    pub fn datum_lines(&self) -> Vec<[Vec3; 2]> {
        let mut out = Vec::new();
        for datum in &self.datums {
            match datum.shape {
                SceneDatumShape::Plane { center, u, v, .. } => {
                    let corners = plane_corners(center, u, v);
                    for k in 0..4 {
                        if let (Some(&a), Some(&b)) = (corners.get(k), corners.get((k + 1) % 4)) {
                            out.push([a, b]);
                        }
                    }
                }
                SceneDatumShape::Axis { from, to } => out.push([from, to]),
                SceneDatumShape::Point(_) => {}
            }
        }
        out
    }

    /// The datum points, drawn as dots.
    pub fn datum_points(&self) -> Vec<Vec3> {
        self.datums
            .iter()
            .filter_map(|datum| match datum.shape {
                SceneDatumShape::Point(p) => Some(p),
                _ => None,
            })
            .collect()
    }

    /// The squares of the datum planes, as (centre, half side along the
    /// plane's x, along its y).
    pub fn datum_planes(&self) -> Vec<(Vec3, Vec3, Vec3)> {
        self.datums
            .iter()
            .filter_map(|datum| match datum.shape {
                SceneDatumShape::Plane { center, u, v, .. } => Some((center, u, v)),
                _ => None,
            })
            .collect()
    }
}

/// The corners of the square `center ± u ± v`, going round.
fn plane_corners(center: Vec3, u: Vec3, v: Vec3) -> [Vec3; 4] {
    [
        center - u - v,
        center + u - v,
        center + u + v,
        center - u + v,
    ]
}

/// The sketches of `scene` that are drawn: all of them, unless the
/// marks hide them.
fn shown_sketches<'a>(
    scene: Option<&'a Scene>,
    marks: Option<&Marks>,
) -> impl Iterator<Item = &'a SceneSketch> {
    let hidden = marks.is_some_and(|marks| marks.hide_sketches);
    scene
        .into_iter()
        .flat_map(|scene| &scene.sketches)
        .filter(move |_| !hidden)
}

/// The datums of `scene` as marks, when they are shown.
pub fn datum_marks(scene: Option<&Scene>, shown: bool) -> Marks {
    Marks {
        datums: if shown {
            scene.map(|scene| scene.datums.clone()).unwrap_or_default()
        } else {
            Vec::new()
        },
        ..Marks::default()
    }
}

/// Where the name of each datum is written in an image of `width` ×
/// `height` seen with `camera`, in pixels: beside a corner of a plane's
/// square, the end of an axis, a point. A datum whose place is not in
/// front of the camera has none.
pub fn datum_labels(
    marks: &Marks,
    camera: &CameraState,
    width: u32,
    height: u32,
) -> Vec<([f64; 2], String)> {
    let Some(projector) = Projector::new(&camera.to_render(), f64::from(width), f64::from(height))
    else {
        return Vec::new();
    };
    marks
        .datums
        .iter()
        .filter_map(|datum| {
            let anchor = match datum.shape {
                SceneDatumShape::Plane { center, u, v, .. } => center - u + v,
                SceneDatumShape::Axis { to, .. } => to,
                SceneDatumShape::Point(p) => p,
            };
            let p = projector.project(anchor)?;
            Some(([p.x, p.y], datum.name.clone()))
        })
        .collect()
}

/// Fills the squares of the datum planes of `marks` into `rgb`, the
/// picture of a frame of `width` × `height` pixels whose distances along
/// the view are `depth` (taken at `sample` in each pixel, [`Frame::sample`]),
/// seen with `camera`: per pixel, the ray through
/// its centre meets the plane inside the square, in front of the camera,
/// and not behind a body (the test of the overlay's lines, with `slack`),
/// and then the pixel is blended with the theme's datum colour at its
/// fill opacity, rounded. Pixels outside every square keep their bytes:
/// without datum planes the picture does not change.
#[allow(clippy::too_many_arguments)]
pub fn fill_datum_planes(
    rgb: &mut [u8],
    width: u32,
    height: u32,
    depth: &[f64],
    sample: f64,
    camera: &CameraState,
    marks: &Marks,
    tokens: &Tokens,
    slack: f64,
) {
    let planes = marks.datum_planes();
    if planes.is_empty() || width == 0 || height == 0 {
        return;
    }
    let Some(projector) = Projector::new(&camera.to_render(), f64::from(width), f64::from(height))
    else {
        return;
    };
    let map = DepthMap::from_parts(width as usize, height as usize, depth, sample, slack);
    let alpha = f64::from(tokens.datum_fill_opacity) / 255.0;
    let ink = tokens.datum;
    let (w, h) = (width as usize, height as usize);
    for (center, u, v) in planes {
        let normal = u.cross(v);
        let (uu, vv) = (u.dot(u), v.dot(v));
        if !(uu > 0.0 && vv > 0.0 && normal.length() > 0.0) {
            continue;
        }
        // The pixels the square can cover: those of its projected corners,
        // or all of them when a corner is not in front of the camera.
        let mut range = (0_usize, w, 0_usize, h);
        let projected: Option<Vec<Projected>> = plane_corners(center, u, v)
            .iter()
            .map(|&c| projector.project(c))
            .collect();
        if let Some(corners) = projected {
            let (mut x0, mut x1, mut y0, mut y1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
            for c in &corners {
                x0 = x0.min(c.x);
                x1 = x1.max(c.x);
                y0 = y0.min(c.y);
                y1 = y1.max(c.y);
            }
            let clamp = |value: f64, size: usize| value.clamp(0.0, size as f64) as usize;
            range = (
                clamp((x0 - 1.0).floor(), w),
                clamp((x1 + 2.0).floor(), w),
                clamp((y0 - 1.0).floor(), h),
                clamp((y1 + 2.0).floor(), h),
            );
        }
        let (x0, x1, y0, y1) = range;
        for py in y0..y1 {
            for px in x0..x1 {
                let (x, y) = (px as f64 + 0.5, py as f64 + 0.5);
                let (start, direction) = projector.ray(x, y);
                let along = direction.dot(normal);
                if along == 0.0 || !along.is_finite() {
                    continue;
                }
                let t = (center - start).dot(normal) / along;
                if !(t.is_finite() && t > projector.near()) {
                    continue;
                }
                let r = start + direction * t - center;
                if (r.dot(u) / uu).abs() > 1.0 || (r.dot(v) / vv).abs() > 1.0 {
                    continue;
                }
                if map.is_some_and(|map| map.covers(x, y, t)) {
                    continue;
                }
                let index = (py * w + px) * 3;
                let Some(pixel) = rgb.get_mut(index..index + 3) else {
                    continue;
                };
                for (channel, &c) in pixel.iter_mut().zip(&ink) {
                    let under = f64::from(*channel);
                    let blended = under + (f64::from(c) - under) * alpha;
                    *channel = blended.round().clamp(0.0, 255.0) as u8;
                }
            }
        }
    }
}

/// The vertices of `scene` that `vertex` refers to, as marks. None when
/// the scene has no such vertex.
pub fn vertex_marks(scene: Option<&Scene>, vertex: &VertexRef) -> Marks {
    let mut marks = Marks::default();
    let Some(scene) = scene else {
        return marks;
    };
    let Ok(found) = scene.vertices_named(
        vertex.feature.as_deref(),
        vertex.vertex.as_deref(),
        &vertex.corner,
    ) else {
        return marks;
    };
    let Some(body) = scene.bodies.get(found.body_index) else {
        return marks;
    };
    marks.points.extend(
        found
            .vertices
            .iter()
            .filter_map(|&k| body.vertices.get(k).copied()),
    );
    marks
}

/// The edges of `scene` that `edge` refers to, as marks. None when the
/// scene has no such edge: a highlight is view state and may outlive what
/// it names.
pub fn edge_marks(scene: Option<&Scene>, edge: &EdgeRef) -> Marks {
    let mut marks = Marks::default();
    let Some(scene) = scene else {
        return marks;
    };
    let Ok(found) = scene.edges_named(edge.feature.as_deref(), edge.edge.as_deref(), &edge.between)
    else {
        return marks;
    };
    let Some(body) = scene.bodies.get(found.body_index) else {
        return marks;
    };
    for polyline in found.edges.iter().filter_map(|&k| body.edges.get(k)) {
        for pair in polyline.windows(2) {
            if let [a, b] = *pair {
                marks.lines.push([a, b]);
            }
        }
    }
    marks
}

/// The longest line is cut into at most this many pieces for the depth
/// test: more than the diagonal of the largest image.
const MAX_DEPTH_STEPS: f64 = 8192.0;

/// How much further than the surface under it a line may be and still
/// count as lying on that surface, as a fraction of the camera's distance
/// to its target. A sketch on a face and an edge between two faces are at
/// the very depth of the surface, give or take rounding.
pub const DEPTH_SLACK: f64 = 1e-6;

/// The GPU renderer's distance is an `f32` computation: sixteen roundings
/// of 2⁻²⁴ on numbers as large as the eye's distance to the model's centre
/// plus the model's diagonal (the `reach` of docs/design/rendering.md).
/// The depth test of the overlay allows a GPU frame that much more.
pub const GPU_DEPTH_ERROR: f64 = 16.0 / 16_777_216.0;

/// How far behind the surface under it a line may be and still count as
/// lying on it, in model units: [`DEPTH_SLACK`] of the camera's distance,
/// and for a frame the GPU made its [`GPU_DEPTH_ERROR`] of the reach too.
pub fn depth_slack(backend: Backend, scene: Option<&Scene>, camera: &CameraState) -> f64 {
    let base = camera.distance.abs() * DEPTH_SLACK;
    match backend {
        Backend::Cpu => base,
        Backend::Gpu => {
            let reach = scene
                .and_then(|scene| scene.bbox.center().map(|c| (c, scene.bbox.diagonal())))
                .map_or(0.0, |(center, diagonal)| {
                    (camera.eye() - center).length() + diagonal
                });
            let extra = reach * GPU_DEPTH_ERROR;
            if extra.is_finite() {
                base + extra
            } else {
                base
            }
        }
    }
}

/// Where the surface beside an edge is looked at, in pixels across the
/// line on either side: near it and twice as far. From these two the
/// surface is carried on up to the edge ([`surface_steps_back`]). Both
/// lie clear of the samples the edge itself passes between, so a surface
/// that ends at the edge is never read on the wrong side.
pub const OUTLINE_NEAR: f64 = 1.5;
pub const OUTLINE_FAR: f64 = 3.0;

/// How much of the surface's own change per pixel beside an edge is
/// allowed besides the step: the place of the edge on the screen, and of
/// a surface's border in the depth buffer, within half a pixel.
pub const OUTLINE_SLOPE: f64 = 0.5;

/// The step beside an edge that makes it an outline, in pixels' worth of
/// the view at the edge's distance (the size one pixel shows there), on
/// top of the surface's slope: two pixels, so a face that continues from
/// the edge is never taken for a step by the rounding of the depth
/// buffer, while a flange in front of its web, a bar in front of
/// another, or the background is.
pub const OUTLINE_STEP: f64 = 2.0;

/// Whether the surface on one side of an edge steps back from it: the
/// edge is then the outline of its body against what lies behind it,
/// the background, another body, or a part of the same body further
/// back. `depth` is the edge's view distance at the point, `near` and
/// `far` the distances of the surface beside it at [`OUTLINE_NEAR`] and
/// [`OUTLINE_FAR`] pixels across (`None` where nothing is drawn), and
/// `allowance` the step that does not count yet, in model units
/// ([`OUTLINE_STEP`] pixels' worth at the edge's depth, plus the slack of
/// the depth test).
///
/// Nothing beside the edge is a step. Otherwise the surface is carried
/// on to the edge along the straight line through the two (in the
/// reciprocal of the distance in perspective, which is what changes
/// evenly across the screen on a plane, so a plane beside the edge
/// reaches it exactly); without a surface at `far` the one at `near`
/// stands for it, its change counted as slope. The surface steps back
/// where it reaches the edge further than the edge by more than the
/// allowance and [`OUTLINE_SLOPE`] of its change per pixel. A face that
/// continues from the edge, at any slope, does not; a surface in front
/// of the edge does not either (the edge is then behind it).
pub fn surface_steps_back(
    depth: f64,
    near: Option<f64>,
    far: Option<f64>,
    perspective: bool,
    allowance: f64,
) -> bool {
    if !(depth.is_finite() && allowance.is_finite()) {
        return false;
    }
    let Some(near) = near.filter(|z| z.is_finite()) else {
        return true;
    };
    let (reached, slope) = match far.filter(|z| z.is_finite()) {
        Some(far) => {
            let reached = if perspective && near > 0.0 && far > 0.0 {
                let inverse = 2.0 / near - 1.0 / far;
                if inverse > 0.0 {
                    1.0 / inverse
                } else {
                    f64::INFINITY
                }
            } else {
                2.0 * near - far
            };
            (reached, (far - near).abs() / (OUTLINE_FAR - OUTLINE_NEAR))
        }
        None => (near, (near - depth).abs() / OUTLINE_NEAR),
    };
    reached - depth > allowance + slope * OUTLINE_SLOPE
}

/// The renderer's depth buffer, as the lines are tested against it.
#[derive(Clone, Copy)]
struct DepthMap<'a> {
    width: usize,
    height: usize,
    /// Distance along the view direction per pixel, row-major; infinite
    /// where there is no body.
    depth: &'a [f64],
    /// Where in its pixel each distance was taken ([`Frame::sample`]).
    sample: f64,
    slack: f64,
}

/// The rule of the overlay's depth test, shared with the GPU's
/// `depth_test.wgsl` (the lines and the datum fills of the direct path):
/// whether a surface of the picture lies in front of a point at distance
/// `depth` by more than `slack`, from the distances `z` of the 4 × 4
/// samples around the point (rows top to bottom, columns left to right;
/// the point inside the middle cell, at `fu`, `fv` across it; a sample
/// that is not a number or infinite shows nothing, one at minus infinity
/// is a surface right at the eye).
///
/// The surface under the point is reconstructed, not looked up: the
/// distances of the four samples around it, interpolated bilinearly,
/// which is exact on a plane (a depth buffer is affine across a triangle
/// on the screen), so an edge any amount behind a flat face is told from
/// one on it at any zoom. Where the surface is not one plane within the
/// cell the reconstruction is off, and it matters where it comes out
/// nearer than the surface: in the hollow where two faces meet, whose
/// crease is further than the samples on either side. There the samples
/// show it, as the *kink*: along a row or a column of the cell whose two
/// samples lie on either side of such a crease, both second differences
/// (at either sample, with its neighbour beyond the cell) say the sample
/// lies further than the chord of its neighbours by more than the slack
/// (rounding makes no hollow, and a crease nearer to a sample than that
/// is missed by less than the slack), and the larger of the
/// two is how far apart the two faces' planes lie at the sample across
/// the crease from the other; the interpolation at a point misses at most
/// the weighted sum, over the samples across the crease from it, of that
/// (bilinear weights, summing to at most one), and a line splits the four
/// samples so that the point's side can be taken to hold a sample next to
/// every sample across, so the largest such pair covers it. A ridge or a
/// change of slope the other way makes the reconstruction further than
/// the surface, which is never strict, and a step to another body makes
/// one second difference of the pair say the opposite: neither gets an
/// allowance, so nothing behind a plate shows along the outline of what
/// stands in front of it, and the allowance never reaches beyond the
/// cell. The surface's change over [`SUBPIXEL_GRAIN`] of a pixel (the
/// largest difference between the four, times the grain) covers how a
/// rasteriser places the surface. The point is hidden when it lies
/// further than the surface plus the kink, that change and the slack.
///
/// So an edge in a hollow, on a face or on a silhouette stays in sight,
/// while an edge behind a thin plate is hidden however far out the view
/// is zoomed. A sample where nothing was drawn among the four hides
/// nothing; a second difference that needs one is left out.
pub fn surface_hides(z: &[[f64; 4]; 4], fu: f64, fv: f64, depth: f64, slack: f64) -> bool {
    if !(depth.is_finite() && fu.is_finite() && fv.is_finite()) {
        return false;
    }
    let [_, [_, a, b, _], [_, c, d, _], _] = *z;
    // A surface right at the eye hides everything.
    if [a, b, c, d].contains(&f64::NEG_INFINITY) {
        return true;
    }
    if !(a.is_finite() && b.is_finite() && c.is_finite() && d.is_finite()) {
        return false;
    }
    let surface = (a * (1.0 - fu) + b * fu) * (1.0 - fv) + (c * (1.0 - fu) + d * fu) * fv;
    let at = |r: usize, k: usize| {
        z.get(r)
            .and_then(|row| row.get(k))
            .copied()
            .unwrap_or(f64::NAN)
    };
    // How far the middle sample lies beyond the chord of its neighbours;
    // nothing where it lies before it or a neighbour shows nothing.
    let beyond = |p: f64, q: f64, r: f64| {
        if p.is_finite() && q.is_finite() && r.is_finite() {
            (2.0 * q - p - r).max(0.0)
        } else {
            0.0
        }
    };
    let mut kink: f64 = 0.0;
    for n in 1..3 {
        // Row n of the cell at its two samples, and column n at its two.
        let pairs = [
            (
                beyond(at(n, 0), at(n, 1), at(n, 2)),
                beyond(at(n, 1), at(n, 2), at(n, 3)),
            ),
            (
                beyond(at(0, n), at(1, n), at(2, n)),
                beyond(at(1, n), at(2, n), at(3, n)),
            ),
        ];
        // Both by more than the slack: rounding makes no hollow, and a
        // crease that close to a sample is missed by less than the slack.
        for (first, second) in pairs {
            if first > slack && second > slack {
                kink = kink.max(first.max(second));
            }
        }
    }
    let change = (b - a)
        .abs()
        .max((d - c).abs())
        .max((c - a).abs())
        .max((d - b).abs());
    surface + kink + change * SUBPIXEL_GRAIN + slack < depth
}

impl<'a> DepthMap<'a> {
    /// `None` when the buffer is not the size it says, or the slack is
    /// not a number: nothing is hidden then, rather than the wrong thing.
    fn new(frame: &'a Frame, slack: f64) -> Option<Self> {
        Self::from_parts(
            frame.width as usize,
            frame.height as usize,
            &frame.depth,
            frame.sample,
            slack,
        )
    }

    fn from_parts(
        width: usize,
        height: usize,
        depth: &'a [f64],
        sample: f64,
        slack: f64,
    ) -> Option<Self> {
        (width.checked_mul(height) == Some(depth.len()) && slack.is_finite() && sample.is_finite())
            .then_some(Self {
                width,
                height,
                depth,
                sample,
                slack,
            })
    }

    /// The distance of the surface at pixel position (`x`, `y`): the
    /// bilinear interpolation of the four samples around it (clamped at
    /// the border of the image), `None` where one of them shows nothing.
    fn surface(&self, x: f64, y: f64) -> Option<f64> {
        if !(x.is_finite() && y.is_finite()) || self.width == 0 || self.height == 0 {
            return None;
        }
        let (u, v) = (x - self.sample, y - self.sample);
        let (i0, j0) = (u.floor(), v.floor());
        let (fu, fv) = (u - i0, v - j0);
        let (wi, hi) = (self.width as f64 - 1.0, self.height as f64 - 1.0);
        let at = |i: f64, j: f64| {
            // Clamped into the image, so the casts fit.
            let (i, j) = (i.clamp(0.0, wi) as usize, j.clamp(0.0, hi) as usize);
            self.depth
                .get(j * self.width + i)
                .copied()
                .filter(|z| z.is_finite())
        };
        let a = at(i0, j0)?;
        let b = at(i0 + 1.0, j0)?;
        let c = at(i0, j0 + 1.0)?;
        let d = at(i0 + 1.0, j0 + 1.0)?;
        Some((a * (1.0 - fu) + b * fu) * (1.0 - fv) + (c * (1.0 - fu) + d * fu) * fv)
    }

    /// Whether the point at pixel position (`x`, `y`) and `depth` of a
    /// line running along `direction` on the screen (a unit vector) is
    /// an outline: the surface on either side of it steps back
    /// ([`surface_steps_back`]), with `allowance` in model units.
    fn outline_at(
        &self,
        x: f64,
        y: f64,
        depth: f64,
        direction: [f64; 2],
        allowance: f64,
        perspective: bool,
    ) -> bool {
        let normal = [-direction[1], direction[0]];
        [1.0, -1.0].into_iter().any(|side: f64| {
            let beside = |reach: f64| {
                self.surface(x + normal[0] * side * reach, y + normal[1] * side * reach)
            };
            surface_steps_back(
                depth,
                beside(OUTLINE_NEAR),
                beside(OUTLINE_FAR),
                perspective,
                allowance,
            )
        })
    }

    /// True when a body is in front of the point at pixel position (`x`,
    /// `y`) and `depth`, by the rule of [`surface_hides`] on the 4 × 4
    /// samples around the point (clamped at the border of the image).
    fn covers(&self, x: f64, y: f64, depth: f64) -> bool {
        if !(x.is_finite() && y.is_finite() && depth.is_finite()) {
            return false;
        }
        if x < 0.0 || y < 0.0 || x >= self.width as f64 || y >= self.height as f64 {
            return false;
        }
        let (u, v) = (x - self.sample, y - self.sample);
        let (i0, j0) = (u.floor(), v.floor());
        let (fu, fv) = (u - i0, v - j0);
        // Within the image (give or take a sample) by the check above.
        let (i0, j0) = (i0 as i64, j0 as i64);
        let (wi, hi) = (self.width as i64, self.height as i64);
        let mut z = [[f64::INFINITY; 4]; 4];
        for (r, row) in z.iter_mut().enumerate() {
            let y = (j0 + r as i64 - 1).clamp(0, hi - 1);
            for (k, cell) in row.iter_mut().enumerate() {
                let x = (i0 + k as i64 - 1).clamp(0, wi - 1);
                // Both clamped into the image.
                let index = y as usize * self.width + x as usize;
                *cell = self.depth.get(index).copied().unwrap_or(f64::INFINITY);
            }
        }
        surface_hides(&z, fu, fv, depth, self.slack)
    }
}

/// A stretch of a segment over the picture: from and to as fractions of
/// the segment, whether it is behind a body, and whether it is the
/// outline of a body (only asked of the edges of bodies).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Stretch {
    from: f64,
    to: f64,
    hidden: bool,
    outline: bool,
}

/// How the outline of a body is told along an edge: the step that does
/// not count yet, in pixels' worth of the view at the edge's depth.
#[derive(Clone, Copy, Debug)]
struct OutlineTest {
    step: f64,
}

/// The stretches of the segment `a`-`b` that are in front of every body
/// and behind one, in order and covering 0 to 1; with `outline`, the
/// stretches in front are told apart by whether they are an outline,
/// tested at the middle of each piece like the depth.
fn stretches(
    map: Option<&DepthMap<'_>>,
    projector: &Projector,
    a: &Projected,
    b: &Projected,
    outline: Option<OutlineTest>,
) -> Vec<Stretch> {
    let piece = |from: f64, to: f64, hidden: bool| Stretch {
        from,
        to,
        hidden,
        outline: false,
    };
    let whole = vec![piece(0.0, 1.0, false)];
    let Some(map) = map else {
        return whole;
    };
    // Only what is over the image can be behind something in it.
    let Some((first, last)) = clip_range(
        [a.x, a.y],
        [b.x, b.y],
        [0.0, 0.0],
        [map.width as f64, map.height as f64],
    ) else {
        return whole;
    };
    let pixels = ((b.x - a.x).abs().max((b.y - a.y).abs()) * (last - first)).ceil();
    if !(pixels.is_finite() && last > first) {
        // A dot: one test decides.
        let hidden = map.covers(a.x, a.y, a.depth);
        return vec![piece(0.0, 1.0, hidden)];
    }
    let length = (b.x - a.x).phypot(b.y - a.y);
    let direction = [(b.x - a.x) / length, (b.y - a.y) / length];
    let outline = outline.filter(|_| direction.iter().all(|c| c.is_finite()));
    // At least one, at most MAX_DEPTH_STEPS: fits an integer.
    let steps = pixels.clamp(1.0, MAX_DEPTH_STEPS) as usize;
    let mut out: Vec<Stretch> = Vec::new();
    let mut push = |next: Stretch| {
        if next.to <= next.from {
            return;
        }
        match out.last_mut() {
            Some(last) if last.hidden == next.hidden && last.outline == next.outline => {
                last.to = next.to;
            }
            _ => out.push(next),
        }
    };
    push(piece(0.0, first, false));
    for step in 0..steps {
        let at = |k: usize| first + (last - first) * (k as f64 / steps as f64);
        let (from, to) = (at(step), at(step + 1));
        // Tested at the middle of the piece.
        let t = 0.5 * (from + to);
        let (x, y) = (a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t);
        let depth = projector.depth_between(a, b, t);
        let hidden = map.covers(x, y, depth);
        let outline = !hidden
            && outline.is_some_and(|test| {
                let allowance = test.step * projector.footprint(depth) + map.slack;
                map.outline_at(x, y, depth, direction, allowance, projector.perspective())
            });
        push(Stretch {
            from,
            to,
            hidden,
            outline,
        });
    }
    push(piece(last, 1.0, false));
    if out.is_empty() { whole } else { out }
}

/// The lines over an image of `width` × `height` pixels seen with
/// `camera`, with nothing hidden: every edge of every body shows, as in a
/// wireframe. `selected` is the id of the selected feature, whose sketch
/// is highlighted. A camera that looks nowhere gives an empty overlay.
pub fn overlay(
    scene: Option<&Scene>,
    camera: &CameraState,
    width: u32,
    height: u32,
    selected: Option<&str>,
) -> Overlay {
    build_overlay(scene, camera, width, height, selected, None, None)
}

/// [`overlay`] with `marks` (the highlighted edges and vertices, the
/// datums, the section's outline), with nothing hidden.
pub fn overlay_untested(
    scene: Option<&Scene>,
    camera: &CameraState,
    width: u32,
    height: u32,
    selected: Option<&str>,
    marks: &Marks,
) -> Overlay {
    build_overlay(
        scene,
        camera,
        width,
        height,
        selected,
        None,
        Some((marks, false)),
    )
}

/// The lines over `frame`, the image of the scene's bodies seen with
/// `camera` as the CPU renderer made it: what [`overlay`] gives, tested
/// against the frame's depth buffer. Edges of bodies are cut where a body
/// is in front of them; sketch lines and points are marked hidden there.
pub fn overlay_on(
    scene: Option<&Scene>,
    camera: &CameraState,
    frame: &Frame,
    selected: Option<&str>,
) -> Overlay {
    overlay_on_with(
        scene,
        camera,
        frame,
        selected,
        depth_slack(Backend::Cpu, scene, camera),
    )
}

/// [`overlay_on`] with the depth test's slack given in model units (see
/// [`depth_slack`]): for a frame another renderer made.
pub fn overlay_on_with(
    scene: Option<&Scene>,
    camera: &CameraState,
    frame: &Frame,
    selected: Option<&str>,
    slack: f64,
) -> Overlay {
    overlay_marked(scene, camera, frame, selected, slack, &Marks::default())
}

/// [`overlay_on_with`] with `marks` drawn over everything else.
pub fn overlay_marked(
    scene: Option<&Scene>,
    camera: &CameraState,
    frame: &Frame,
    selected: Option<&str>,
    slack: f64,
    marks: &Marks,
) -> Overlay {
    overlay_see_through(scene, camera, frame, selected, slack, marks, false)
}

/// [`overlay_marked`] for a picture whose bodies are see-through or not:
/// through see-through bodies the hidden part of a body's edge is drawn
/// too, dimmed as a hidden sketch is, so that the edges of a hole or a
/// cavity inside show and the shape stays readable.
pub fn overlay_see_through(
    scene: Option<&Scene>,
    camera: &CameraState,
    frame: &Frame,
    selected: Option<&str>,
    slack: f64,
    marks: &Marks,
    see_through: bool,
) -> Overlay {
    let map = DepthMap::new(frame, slack);
    build_overlay(
        scene,
        camera,
        frame.width,
        frame.height,
        selected,
        map.as_ref(),
        Some((marks, see_through)),
    )
}

fn build_overlay(
    scene: Option<&Scene>,
    camera: &CameraState,
    width: u32,
    height: u32,
    selected: Option<&str>,
    map: Option<&DepthMap<'_>>,
    marked: Option<(&Marks, bool)>,
) -> Overlay {
    let marks = marked.map(|(marks, _)| marks);
    let see_through = marked.is_some_and(|(_, see_through)| see_through);
    let mut out = Overlay::default();
    let Some(projector) = Projector::new(&camera.to_render(), f64::from(width), f64::from(height))
    else {
        return out;
    };
    // `tested`: against the depth buffer; `dimmed`: what is hidden is
    // kept and marked, not left out; `edge`: an edge of a body, whose
    // stretches in sight are told apart by whether they are its outline.
    let mut line_of =
        |a: Vec3, b: Vec3, kind: LineKind, tested: bool, dimmed: bool, edge: Option<EdgeInk>| {
            let Some((from, to)) = projector.segment(a, b) else {
                return;
            };
            let at = |t: f64| [from.x + (to.x - from.x) * t, from.y + (to.y - from.y) * t];
            let depth_at = |t: f64| projector.depth_between(&from, &to, t.clamp(0.0, 1.0));
            let outline = edge.map(|_| OutlineTest { step: OUTLINE_STEP });
            let mut joined = false;
            for stretch in stretches(map.filter(|_| tested), &projector, &from, &to, outline) {
                if stretch.hidden && !dimmed {
                    joined = false;
                    continue;
                }
                out.lines.push(OverlayLine {
                    // The ends themselves, not the ends computed back.
                    from: if stretch.from <= 0.0 {
                        [from.x, from.y]
                    } else {
                        at(stretch.from)
                    },
                    to: if stretch.to >= 1.0 {
                        [to.x, to.y]
                    } else {
                        at(stretch.to)
                    },
                    kind,
                    hidden: stretch.hidden,
                    edge: edge.map(|edge| EdgeInk {
                        silhouette: stretch.outline,
                        joined,
                        cut: [stretch.from > 0.0, stretch.to < 1.0],
                        depth: [depth_at(stretch.from), depth_at(stretch.to)],
                        ..edge
                    }),
                });
                joined = true;
            }
        };
    // The edges of the bodies first, then the world axes over them (an
    // edge along an axis must not paint it out), then the sketches.
    // In a picture that is cut open an edge is drawn where it is kept, and
    // the outline of the cut is drawn as edges.
    let kept = |a: Vec3, b: Vec3| match marks {
        Some(marks) => marks.kept(a, b),
        None => Some([a, b]),
    };
    let edges_shown = !marks.is_some_and(|marks| marks.hide_edges);
    for (body, color) in scene
        .into_iter()
        .filter(|_| edges_shown)
        .flat_map(body_colors)
    {
        for edge in body.edges.iter() {
            let ink = EdgeInk {
                body: color,
                extent: extent_on_screen(&projector, edge),
                silhouette: false,
                joined: false,
                cut: [false; 2],
                depth: [0.0; 2],
            };
            for pair in edge.windows(2) {
                if let [a, b] = *pair
                    && let Some([a, b]) = kept(a, b)
                {
                    line_of(a, b, LineKind::Edge, true, see_through, Some(ink));
                }
            }
        }
    }
    for &[a, b] in marks.into_iter().flat_map(Marks::outline) {
        line_of(a, b, LineKind::Edge, true, see_through, None);
    }
    let length = camera.distance * AXIS_LENGTH;
    for (axis, direction) in [Vec3::X, Vec3::Y, Vec3::Z].into_iter().enumerate() {
        line_of(
            Vec3::ZERO,
            direction * length,
            LineKind::Axis(axis),
            false,
            false,
            None,
        );
    }
    // The datums, tested like a sketch, under the sketches.
    for [a, b] in marks.map(Marks::datum_lines).unwrap_or_default() {
        line_of(a, b, LineKind::Datum, true, true, None);
    }
    for trace in marks.into_iter().flat_map(|marks| &marks.traces) {
        for pair in trace.windows(2) {
            if let [a, b] = *pair {
                line_of(a, b, LineKind::Trace, true, true, None);
            }
        }
    }
    for comb in marks.into_iter().flat_map(|marks| &marks.combs) {
        for pair in comb.windows(2) {
            if let [a, b] = *pair {
                line_of(a, b, LineKind::Comb, true, true, None);
            }
        }
    }
    for sketch in shown_sketches(scene, marks) {
        let chosen = selected == Some(sketch.feature.as_str());
        for polyline in &sketch.polylines {
            let kind = match (polyline.construction, chosen) {
                (true, _) => LineKind::Construction,
                (false, true) => LineKind::Selected,
                (false, false) => LineKind::Sketch,
            };
            for pair in polyline.points.windows(2) {
                if let [a, b] = *pair {
                    line_of(a, b, kind, true, true, None);
                }
            }
        }
    }
    // What is pointed at, over everything else.
    for &[a, b] in marks.into_iter().flat_map(|marks| &marks.lines) {
        if let Some([a, b]) = kept(a, b) {
            line_of(a, b, LineKind::Highlight, true, true, None);
        }
    }
    for sketch in shown_sketches(scene, marks) {
        let kind = if selected == Some(sketch.feature.as_str()) {
            LineKind::Selected
        } else {
            LineKind::Sketch
        };
        for point in &sketch.points {
            if let Some(p) = projector.project(*point) {
                out.points.push(OverlayPoint {
                    at: [p.x, p.y],
                    kind,
                    hidden: map.is_some_and(|map| map.covers(p.x, p.y, p.depth)),
                });
            }
        }
    }
    for point in marks.map(Marks::datum_points).unwrap_or_default() {
        if let Some(p) = projector.project(point) {
            out.points.push(OverlayPoint {
                at: [p.x, p.y],
                kind: LineKind::Datum,
                hidden: map.is_some_and(|map| map.covers(p.x, p.y, p.depth)),
            });
        }
    }
    for point in marks.into_iter().flat_map(|marks| &marks.points) {
        if kept(*point, *point).is_some()
            && let Some(p) = projector.project(*point)
        {
            out.points.push(OverlayPoint {
                at: [p.x, p.y],
                kind: LineKind::Highlight,
                hidden: map.is_some_and(|map| map.covers(p.x, p.y, p.depth)),
            });
        }
    }
    out
}

/// The bodies of `scene` with their own colours (their appearance; `None`
/// for the theme's), which their edges are drawn in a shade of: the
/// colour of a body's first face, since an appearance is a body's.
pub fn body_colors(
    scene: &Scene,
) -> impl Iterator<Item = (&craft3d_engine::SceneBody, Option<Rgb>)> {
    scene
        .bodies
        .iter()
        .map(|body| (body, scene.face_colors.get(body.first_face_id)))
}

/// The extent of an edge on screen, in pixels: the diagonal of the box
/// around its points, projected. A straight edge's own length, a circle's
/// diameter or more seen edge-on; infinite when a corner of the box is
/// not in front of the eye (the edge is then not short on screen), zero
/// for an edge of one point.
pub fn extent_on_screen(projector: &Projector, points: &[Vec3]) -> f64 {
    let Some((&first, rest)) = points.split_first() else {
        return 0.0;
    };
    let (mut lo, mut hi) = (first, first);
    for p in rest {
        lo = lo.min(*p);
        hi = hi.max(*p);
    }
    match (projector.project(lo), projector.project(hi)) {
        (Some(a), Some(b)) => {
            let length = (b.x - a.x).phypot(b.y - a.y);
            if length.is_finite() {
                length
            } else {
                f64::INFINITY
            }
        }
        _ => f64::INFINITY,
    }
}

/// How an image is to be rendered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Quality {
    pub width: u32,
    pub height: u32,
    /// Supersampling per axis, 1 to 4.
    pub ssaa: u32,
}

/// What either renderer is given for an image of `quality` in the theme's
/// colours, with no face highlighted.
pub fn render_params(quality: Quality, tokens: &Tokens) -> RenderParams {
    RenderParams {
        width: quality.width,
        height: quality.height,
        ssaa: quality.ssaa,
        background: tokens.viewport_background,
        color: tokens.body,
        highlight: Highlight::NONE,
        opacity: 1.0,
        section: None,
        analysis: None,
    }
}

/// The faces of `scene` that `face` names (a persistent name, with the
/// feature of its body when given), as the renderers highlight them in the
/// theme's colour. No face when the scene has none of that name: a
/// highlight is view state and may outlive the face it names.
pub fn face_highlight(
    scene: Option<&Scene>,
    feature: Option<&str>,
    face: &str,
    tokens: &Tokens,
) -> Highlight {
    scene
        .and_then(|scene| scene.faces_named(feature, face).ok())
        .map_or(Highlight::NONE, |faces| {
            Highlight::new(tokens.face_highlight, faces.ids).0
        })
}

/// The bodies of `scene` as the CPU reference renderer shades them, in the
/// theme's colours. No scene, or a scene without bodies, is the background.
pub fn render_bodies(
    scene: Option<&Scene>,
    camera: &CameraState,
    quality: Quality,
    tokens: &Tokens,
) -> Result<Frame, String> {
    let params = render_params(quality, tokens);
    let empty = craft3d_tess::Mesh::default();
    let mesh = scene.map_or(&empty, |scene| &scene.mesh);
    let no_colors = craft3d_render::FaceColors::new();
    let colors = scene.map_or(&no_colors, |scene| &scene.face_colors);
    craft3d_render::render_colored(mesh, colors, &camera.to_render(), &params)
        .map_err(|e| e.to_string())
}

/// One image of the viewport as the window shows it: the bodies as
/// pixels, and what is drawn over them as lines in the image's pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct Composed {
    pub width: u32,
    pub height: u32,
    /// 3 bytes per pixel, row-major from the top-left.
    pub rgb: Vec<u8>,
    pub overlay: Overlay,
    /// Fraction of the pixels covered by a body.
    pub coverage: f64,
    pub triangles: usize,
    /// Which renderer shaded the bodies.
    pub backend: Backend,
}

/// Renders the bodies with the CPU reference renderer and works out the
/// overlay for the same camera and image size: [`compose_with`] on the
/// oracle.
pub fn compose(
    scene: Option<&Scene>,
    camera: &CameraState,
    quality: Quality,
    tokens: &Tokens,
    selected: Option<&str>,
) -> Result<Composed, String> {
    compose_with(
        &mut BodyRenderer::cpu(),
        scene,
        camera,
        quality,
        tokens,
        selected,
    )
}

/// Renders the bodies with `renderer` and works out the overlay for the
/// same camera and image size, tested against the frame's depth buffer
/// with the slack that renderer needs. No egui and no shared state: the
/// window calls it on its render thread.
pub fn compose_with(
    renderer: &mut BodyRenderer,
    scene: Option<&Scene>,
    camera: &CameraState,
    quality: Quality,
    tokens: &Tokens,
    selected: Option<&str>,
) -> Result<Composed, String> {
    let (mut frame, backend) = renderer.render(scene, camera, quality, tokens)?;
    let coverage = frame.coverage();
    let slack = depth_slack(backend, scene, camera);
    let (width, height) = (frame.width, frame.height);
    fill_datum_planes(
        &mut frame.rgb,
        width,
        height,
        &frame.depth,
        frame.sample,
        camera,
        renderer.marks(),
        tokens,
        slack,
    );
    let mut overlay = overlay_see_through(
        scene,
        camera,
        &frame,
        selected,
        slack,
        renderer.marks(),
        renderer.opacity() < 1.0,
    );
    overlay.blur = renderer.edge_blur(scene, camera, frame.height);
    Ok(Composed {
        width: frame.width,
        height: frame.height,
        rgb: frame.rgb,
        overlay,
        coverage,
        triangles: renderer.drawn_triangles(scene),
        backend,
    })
}

/// A finished picture of the viewport: bodies and overlay.
#[derive(Clone, Debug, PartialEq)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    /// 3 bytes per pixel, row-major from the top-left.
    pub rgb: Vec<u8>,
    /// Fraction of the pixels covered by a body.
    pub coverage: f64,
    /// Fraction of the pixels the overlay changed.
    pub overlay_coverage: f64,
    pub triangles: usize,
}

/// Renders the bodies with the CPU reference renderer and draws the
/// overlay into the image: what the window shows, without a window, from
/// the oracle (`ui.screenshot`). `scale` is the number of pixels per
/// point, for the widths of the lines.
pub fn picture(
    scene: Option<&Scene>,
    camera: &CameraState,
    quality: Quality,
    tokens: &Tokens,
    selected: Option<&str>,
    scale: f64,
) -> Result<Picture, String> {
    let composed = compose(scene, camera, quality, tokens, selected)?;
    Ok(rasterise(composed, tokens, scale))
}

/// [`picture`] with faces highlighted: what the window shows when a face
/// was clicked or pointed at.
#[allow(clippy::too_many_arguments)]
pub fn picture_with(
    scene: Option<&Scene>,
    camera: &CameraState,
    quality: Quality,
    tokens: &Tokens,
    selected: Option<&str>,
    highlight: Highlight,
    scale: f64,
) -> Result<Picture, String> {
    picture_marked(
        scene,
        camera,
        quality,
        tokens,
        selected,
        highlight,
        &Marks::default(),
        1.0,
        scale,
    )
}

/// [`picture_with`] with `marks` drawn over the picture, and the bodies
/// `opacity` opaque (1: as ever): what the window shows when an edge was
/// clicked or pointed at, or the bodies were made see-through.
#[allow(clippy::too_many_arguments)]
pub fn picture_marked(
    scene: Option<&Scene>,
    camera: &CameraState,
    quality: Quality,
    tokens: &Tokens,
    selected: Option<&str>,
    highlight: Highlight,
    marks: &Marks,
    opacity: f64,
    scale: f64,
) -> Result<Picture, String> {
    picture_lit(
        scene,
        camera,
        quality,
        tokens,
        selected,
        highlight,
        marks,
        opacity,
        (None, None),
        scale,
    )
}

/// [`picture_marked`] in the studio look when `studio` is given, with a
/// surface analysis when `analysis` is (drawn in the plain look).
#[allow(clippy::too_many_arguments)]
pub fn picture_lit(
    scene: Option<&Scene>,
    camera: &CameraState,
    quality: Quality,
    tokens: &Tokens,
    selected: Option<&str>,
    highlight: Highlight,
    marks: &Marks,
    opacity: f64,
    (studio, analysis): (
        Option<craft3d_gpu::Studio>,
        Option<std::sync::Arc<ViewAnalysis>>,
    ),
    scale: f64,
) -> Result<Picture, String> {
    let mut renderer = BodyRenderer::cpu();
    renderer.set_highlight(highlight);
    renderer.set_marks(marks.clone());
    renderer.set_opacity(opacity);
    renderer.set_studio(studio);
    renderer.set_analysis(analysis);
    let composed = compose_with(&mut renderer, scene, camera, quality, tokens, selected)?;
    Ok(rasterise(composed, tokens, scale))
}

/// Draws the overlay of `composed` into its pixels. `scale` is the number
/// of pixels per point, for the widths of the lines.
pub fn rasterise(composed: Composed, tokens: &Tokens, scale: f64) -> Picture {
    let Composed {
        width,
        height,
        mut rgb,
        overlay,
        coverage,
        triangles,
        backend: _,
    } = composed;
    let before = rgb.clone();
    let scale = if scale.is_finite() && scale > 0.0 {
        scale.min(16.0)
    } else {
        1.0
    };
    let density = EdgeDensity::new(&overlay, width, height, tokens, scale);
    let mut colors = EdgeColorCache::default();
    let blur = overlay.blur;
    let lines = &overlay.lines;
    let mut k = 0;
    while let Some(line) = lines.get(k) {
        let Some(edge) = line.edge else {
            draw_line(
                &mut rgb,
                width,
                height,
                line.from,
                line.to,
                f64::from(line.width(tokens)) * scale,
                line.color(tokens),
                opacity(line.hidden, tokens),
            );
            k += 1;
            continue;
        };
        // The stretches of one segment of an edge, drawn as one line whose
        // every pixel takes the look of the stretch at its foot on the
        // line, as the GPU draws a segment: no stretch inks over the next.
        let run_end = k
            + 1
            + lines
                .get(k + 1..)
                .unwrap_or_default()
                .iter()
                .take_while(|l| l.edge.is_some_and(|e| e.joined))
                .count();
        let run = lines.get(k..run_end).unwrap_or_default();
        k = run_end.max(k + 1);
        let (from, to) = (line.from, run.last().map_or(line.to, |l| l.to));
        let (dx, dy) = (to[0] - from[0], to[1] - from[1]);
        let length2 = dx * dx + dy * dy;
        // Where along the run each stretch ends, and how it is drawn.
        let looks: Vec<(f64, f64, Rgb, f64)> = run
            .iter()
            .map(|l| {
                let end = if length2 > 0.0 {
                    ((l.to[0] - from[0]) * dx + (l.to[1] - from[1]) * dy) / length2
                } else {
                    1.0
                };
                let silhouette = l.edge.is_some_and(|e| e.silhouette);
                let half = (f64::from(l.width(tokens)) * scale * 0.5).clamp(0.35, 64.0);
                let color = colors.get(edge.body, tokens).pick(silhouette);
                (
                    end,
                    half,
                    color,
                    f64::from(opacity(l.hidden, tokens)) / 255.0,
                )
            })
            .collect();
        let widest = looks.iter().map(|look| look.1).fold(0.35, f64::max);
        let cut = [
            edge.cut[0],
            run.last()
                .and_then(|l| l.edge)
                .map_or(edge.cut[1], |e| e.cut[1]),
        ];
        let long = fade_short(edge.extent, tokens, scale);
        // The run's ends lie on one segment: its depth between them.
        let ends = [
            edge.depth[0],
            run.last()
                .and_then(|l| l.edge)
                .map_or(edge.depth[1], |e| e.depth[1]),
        ];
        let w = width as usize;
        cover_line(
            width,
            height,
            from,
            to,
            widest * 2.0,
            |index, _, distance, along| {
                if (cut[0] && along < 0.0) || (cut[1] && along > 1.0) {
                    return;
                }
                let Some(&(_, half, color, alpha)) =
                    looks.iter().find(|look| along <= look.0).or(looks.last())
                else {
                    return;
                };
                let (px, py) = (index % w.max(1), index / w.max(1));
                let mut alpha = (half + 0.5 - distance).clamp(0.0, 1.0)
                    * alpha
                    * edge_fade(long, density.at(px as f64 + 0.5, py as f64 + 0.5), tokens);
                if let Some(blur) = &blur {
                    alpha *= blur.fade_along(ends, along, half * 2.0);
                }
                if alpha <= 0.0 {
                    return;
                }
                let Some(pixel) = rgb.get_mut(index * 3..index * 3 + 3) else {
                    return;
                };
                for (channel, &ink) in pixel.iter_mut().zip(&color) {
                    let under = f64::from(*channel);
                    let blended = under + (f64::from(ink) - under) * alpha;
                    *channel = blended.round().clamp(0.0, 255.0) as u8;
                }
            },
        );
    }
    for point in &overlay.points {
        // A dot is a line without length.
        draw_line(
            &mut rgb,
            width,
            height,
            point.at,
            point.at,
            f64::from(point.kind.dot_radius(tokens)) * 2.0 * scale,
            point.kind.color(tokens),
            opacity(point.hidden, tokens),
        );
    }
    let changed = rgb
        .as_chunks::<3>()
        .0
        .iter()
        .zip(before.as_chunks::<3>().0)
        .filter(|(a, b)| a != b)
        .count();
    let pixels = (rgb.len() / 3).max(1);
    Picture {
        width,
        height,
        rgb,
        coverage,
        overlay_coverage: changed as f64 / pixels as f64,
        triangles,
    }
}

/// The same as fractions of the segment: it is inside the box from the
/// first to the second.
fn clip_range(a: [f64; 2], b: [f64; 2], lo: [f64; 2], hi: [f64; 2]) -> Option<(f64, f64)> {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let (mut t0, mut t1) = (0.0_f64, 1.0_f64);
    for (delta, start, low, high) in [(dx, a[0], lo[0], hi[0]), (dy, a[1], lo[1], hi[1])] {
        if delta == 0.0 {
            if start < low || start > high {
                return None;
            }
            continue;
        }
        let (mut near, mut far) = ((low - start) / delta, (high - start) / delta);
        if near > far {
            std::mem::swap(&mut near, &mut far);
        }
        t0 = t0.max(near);
        t1 = t1.min(far);
        if t0 > t1 {
            return None;
        }
    }
    (t0.is_finite() && t1.is_finite()).then_some((t0, t1))
}

/// The colours of the edges of each body colour met, worked out once
/// (a few bodies' colours, many edges).
#[derive(Default)]
pub struct EdgeColorCache {
    known: Vec<(Option<Rgb>, EdgeColors)>,
}

impl EdgeColorCache {
    pub fn get(&mut self, body: Option<Rgb>, tokens: &Tokens) -> EdgeColors {
        if let Some((_, colors)) = self.known.iter().find(|(b, _)| *b == body) {
            return *colors;
        }
        let colors = tokens.edge_colors(body.unwrap_or(tokens.body));
        self.known.push((body, colors));
        colors
    }
}

/// Hermite's smooth step from 0 at `from` to 1 at `to`: no jump when an
/// edge crosses a threshold while the view zooms. A step at one point
/// when the two are equal.
pub fn smooth_step(from: f64, to: f64, x: f64) -> f64 {
    if !x.is_finite() {
        return if x > 0.0 { 1.0 } else { 0.0 };
    }
    if to.partial_cmp(&from) != Some(std::cmp::Ordering::Greater) {
        return if x >= to { 1.0 } else { 0.0 };
    }
    let t = ((x - from) / (to - from)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// How much an edge of `extent` pixels on screen keeps of its ink for
/// being short: 0 at `edge_fade_short` points or less, 1 from
/// `edge_fade_long` on (both times `scale` pixels per point).
pub fn fade_short(extent: f64, tokens: &Tokens, scale: f64) -> f64 {
    smooth_step(
        f64::from(tokens.edge_fade_short) * scale,
        f64::from(tokens.edge_fade_long) * scale,
        extent,
    )
}

/// The opacity an edge is drawn with, as a share of its own: `long` (what
/// [`fade_short`] keeps) times what the `density` of the edges around
/// leaves (all of it up to `edge_dense_cap`, beyond that the cap over the
/// density: in a crowded place the edges together cover no more than
/// the cap, however many they are), lifted so that nothing goes below
/// `edge_fade_floor`.
pub fn edge_fade(long: f64, density: f64, tokens: &Tokens) -> f64 {
    let floor = f64::from(tokens.edge_fade_floor).clamp(0.0, 1.0);
    let cap = f64::from(tokens.edge_dense_cap);
    let sparse = if density > cap { cap / density } else { 1.0 };
    let keep = (long * sparse).clamp(0.0, 1.0);
    if keep.is_finite() {
        floor + (1.0 - floor) * keep
    } else {
        1.0
    }
}

/// The side of a tile of [`EdgeDensity`] in pixels: `edge_tile` points,
/// whole pixels, 4 to 64.
pub fn tile_pixels(tokens: &Tokens, scale: f64) -> u32 {
    let side = (f64::from(tokens.edge_tile) * scale).round();
    if side.is_finite() {
        side.clamp(4.0, 64.0) as u32
    } else {
        16
    }
}

/// How crowded with edges the picture is: per tile of [`tile_pixels`],
/// the share of its pixels the edges of the bodies in sight cover (their
/// ink at the width of an edge between faces, `edge_width`, summed where
/// edges cross, as [`draw_line`] inks them: so a tile crossed by one edge
/// of one pixel is 1/16 covered, by eight such edges a half), and between
/// the tiles' centres blended bilinearly, so the measure changes smoothly
/// as edges move across the screen. Measured in closed form, without
/// drawing: the length of each line in each tile times the ink across a
/// line, and the ink of its round ends in the tiles they lie in. The
/// GPU's viewport measures the same per pixel, with an ink pass and a pass
/// over the tiles (`craft3d_gpu::Onscreen`), within the tolerance of
/// `tests/gpu_viewport.rs`.
pub struct EdgeDensity {
    tile: u32,
    columns: usize,
    rows: usize,
    means: Vec<f64>,
}

impl EdgeDensity {
    pub fn new(overlay: &Overlay, width: u32, height: u32, tokens: &Tokens, scale: f64) -> Self {
        let tile = tile_pixels(tokens, scale);
        let (w, h) = (width as usize, height as usize);
        let columns = w.div_ceil(tile as usize);
        let rows = h.div_ceil(tile as usize);
        let mut sums = vec![0.0_f64; columns.saturating_mul(rows)];
        // The ink of a line as `cover_line` inks it, per pixel of length
        // across it, and at a round end beyond it (the profile
        // `clamp(half + 0.5 - distance, 0, 1)` over a line and over half a
        // disc).
        let half = (f64::from(tokens.edge_width) * scale * 0.5).clamp(0.35, 64.0);
        let across = if half >= 0.5 {
            2.0 * half
        } else {
            (half + 0.5) * (half + 0.5)
        };
        let a = half - 0.5;
        let end_ink = if a >= 0.0 {
            std::f64::consts::PI * (a * a / 2.0 + a / 2.0 + 1.0 / 6.0)
        } else {
            std::f64::consts::PI * (half + 0.5).ppowi(3) / 6.0
        };
        let side = f64::from(tile);
        let (wf, hf) = (width as f64, height as f64);
        let mut add = |x: f64, y: f64, ink: f64| {
            if !(x >= 0.0 && y >= 0.0 && x < wf && y < hf && ink.is_finite()) {
                return;
            }
            // Within the image, so the casts fit.
            let (i, j) = ((x / side) as usize, (y / side) as usize);
            if let Some(sum) = sums.get_mut(j * columns + i) {
                *sum += ink;
            }
        };
        // The edges in sight, a segment of an edge as one line where the
        // outline cut it into stretches; a round end where the segment
        // ends, none where the edge goes behind a body.
        let mut runs: Vec<([f64; 2], [f64; 2], [bool; 2])> = Vec::new();
        let mut open = false;
        for line in &overlay.lines {
            let Some(edge) = line.edge.filter(|_| !line.hidden) else {
                open = false;
                continue;
            };
            match runs.last_mut() {
                Some(run) if open && edge.joined => {
                    run.1 = line.to;
                    run.2[1] = edge.cut[1];
                }
                _ => runs.push((line.from, line.to, edge.cut)),
            }
            open = true;
        }
        for (from, to, cut) in runs {
            let (dx, dy) = (to[0] - from[0], to[1] - from[1]);
            let length = dx.phypot(dy);
            if !length.is_finite() {
                continue;
            }
            if length <= 0.0 {
                // A dot: both half discs.
                if !(cut[0] || cut[1]) {
                    add(from[0], from[1], 2.0 * end_ink);
                }
                continue;
            }
            for (end, cut) in [(from, cut[0]), (to, cut[1])] {
                if !cut {
                    add(end[0], end[1], end_ink);
                }
            }
            // The line's ink is spread across it: four strands, a quarter
            // of it each, at a quarter and three quarters of its half width
            // on either side, so that a line along a tile's border puts
            // its ink on both sides as the pixels do. Along each strand,
            // where it crosses the tiles' borders, in order: each piece
            // between two crossings lies in one tile.
            let normal = [-dy / length, dx / length];
            for offset in [-0.75, -0.25, 0.25, 0.75] {
                let shift = [normal[0] * offset * half, normal[1] * offset * half];
                let start_at = [from[0] + shift[0], from[1] + shift[1]];
                let mut cuts = vec![0.0, 1.0];
                for (start, delta, limit) in [(start_at[0], dx, wf), (start_at[1], dy, hf)] {
                    if delta == 0.0 {
                        continue;
                    }
                    let (lo, hi) = (start.min(start + delta), start.max(start + delta));
                    let first = (lo.max(0.0) / side).ceil();
                    let last = (hi.min(limit) / side).floor();
                    if !(first.is_finite() && last.is_finite()) || last - first > 4096.0 {
                        continue;
                    }
                    let mut k = first;
                    while k <= last {
                        let t = (k * side - start) / delta;
                        if t > 0.0 && t < 1.0 {
                            cuts.push(t);
                        }
                        k += 1.0;
                    }
                }
                cuts.sort_by(f64::total_cmp);
                for pair in cuts.windows(2) {
                    if let [t0, t1] = *pair
                        && t1 > t0
                    {
                        let t = 0.5 * (t0 + t1);
                        add(
                            start_at[0] + dx * t,
                            start_at[1] + dy * t,
                            (t1 - t0) * length * across * 0.25,
                        );
                    }
                }
            }
        }
        let t = tile as usize;
        let means = sums
            .iter()
            .enumerate()
            .map(|(k, sum)| {
                let (i, j) = (k % columns.max(1), k / columns.max(1));
                let (x0, y0) = (i * t, j * t);
                let (x1, y1) = ((x0 + t).min(w), (y0 + t).min(h));
                let count = (x1.saturating_sub(x0) * y1.saturating_sub(y0)).max(1);
                sum / count as f64
            })
            .collect();
        Self {
            tile,
            columns,
            rows,
            means,
        }
    }

    /// The number of tiles across and down, and their means, row-major.
    pub fn tiles(&self) -> (usize, usize, &[f64]) {
        (self.columns, self.rows, &self.means)
    }

    /// The density at pixel position (`x`, `y`): the tiles' means blended
    /// bilinearly between their centres, clamped at the border.
    pub fn at(&self, x: f64, y: f64) -> f64 {
        if self.columns == 0 || self.rows == 0 || !(x.is_finite() && y.is_finite()) {
            return 0.0;
        }
        let side = f64::from(self.tile);
        let (u, v) = (x / side - 0.5, y / side - 0.5);
        let (i0, j0) = (u.floor(), v.floor());
        let (fu, fv) = (u - i0, v - j0);
        let (last_i, last_j) = ((self.columns - 1) as f64, (self.rows - 1) as f64);
        let mean = |i: f64, j: f64| {
            // Clamped into the grid, so the casts fit.
            let (i, j) = (i.clamp(0.0, last_i) as usize, j.clamp(0.0, last_j) as usize);
            self.means.get(j * self.columns + i).copied().unwrap_or(0.0)
        };
        let top = mean(i0, j0) * (1.0 - fu) + mean(i0 + 1.0, j0) * fu;
        let bottom = mean(i0, j0 + 1.0) * (1.0 - fu) + mean(i0 + 1.0, j0 + 1.0) * fu;
        top * (1.0 - fv) + bottom * fv
    }
}

/// Draws an anti-aliased line of `width` pixels with round ends into an RGB
/// image, `opacity` (0 to 255) opaque. Coordinates may be anything, the
/// image is never indexed outside.
#[allow(clippy::too_many_arguments)]
pub fn draw_line(
    rgb: &mut [u8],
    width: u32,
    height: u32,
    from: [f64; 2],
    to: [f64; 2],
    line_width: f64,
    color: Rgb,
    opacity: u8,
) {
    draw_line_faded(
        rgb,
        width,
        height,
        from,
        to,
        line_width,
        color,
        opacity,
        &|_, _| 1.0,
    );
}

/// [`draw_line`] with the opacity at each pixel also scaled by `fade` of
/// the pixel's centre (0 to 1).
#[allow(clippy::too_many_arguments)]
pub fn draw_line_faded(
    rgb: &mut [u8],
    width: u32,
    height: u32,
    from: [f64; 2],
    to: [f64; 2],
    line_width: f64,
    color: Rgb,
    opacity: u8,
    fade: &dyn Fn(f64, f64) -> f64,
) {
    let w = width as usize;
    cover_line(width, height, from, to, line_width, |index, alpha, _, _| {
        let (px, py) = (index % w.max(1), index / w.max(1));
        let alpha = alpha * f64::from(opacity) / 255.0
            * fade(px as f64 + 0.5, py as f64 + 0.5).clamp(0.0, 1.0);
        if alpha <= 0.0 {
            return;
        }
        let Some(pixel) = rgb.get_mut(index * 3..index * 3 + 3) else {
            return;
        };
        for (channel, &ink) in pixel.iter_mut().zip(&color) {
            let under = f64::from(*channel);
            let blended = under + (f64::from(ink) - under) * alpha;
            *channel = blended.round().clamp(0.0, 255.0) as u8;
        }
    });
}

/// Visits the pixels an anti-aliased line of `line_width` pixels with
/// round ends covers in an image of `width` × `height`, with how much of
/// each it covers (0 to 1, by the distance of the pixel's centre to the
/// segment): `visit(index of the pixel, coverage, that distance, where
/// along the segment the pixel's centre lies, below 0 or above 1 beside
/// an end)`, every pixel once.
fn cover_line(
    width: u32,
    height: u32,
    from: [f64; 2],
    to: [f64; 2],
    line_width: f64,
    mut visit: impl FnMut(usize, f64, f64, f64),
) {
    let finite = from.iter().chain(&to).all(|c| c.is_finite());
    if !finite || !line_width.is_finite() || width == 0 || height == 0 {
        return;
    }
    let half = (line_width * 0.5).clamp(0.35, 64.0);
    let (w, h) = (f64::from(width), f64::from(height));
    // Only what can touch the image.
    let pad = half + 1.0;
    let Some((t0, t1)) = clip_range(from, to, [-pad, -pad], [w + pad, h + pad]) else {
        return;
    };
    let at = |t: f64| {
        [
            from[0] + (to[0] - from[0]) * t,
            from[1] + (to[1] - from[1]) * t,
        ]
    };
    let (a, b) = (at(t0), at(t1));
    // Walk the axis the line runs along most (`u`), one column of pixels at
    // a time, and shade the pixels of that column near the line (`v`) by
    // their distance to the segment. Every pixel is visited once.
    let steep = (b[1] - a[1]).abs() > (b[0] - a[0]).abs();
    let uv = |p: [f64; 2]| if steep { [p[1], p[0]] } else { p };
    let (a, b) = (uv(a), uv(b));
    let (du, dv) = (b[0] - a[0], b[1] - a[1]);
    let length = du.phypot(dv);
    // How far from the line's centre a column can still be inked: the
    // perpendicular reach, seen along the column. A line is at most 45°
    // from its own main axis.
    let slope = if du != 0.0 { dv / du } else { 0.0 };
    let reach = ((half + 1.0) * (1.0 + slope * slope).sqrt()).ceil() as i64 + 1;
    let (u_size, v_size) = if steep {
        (i64::from(height), i64::from(width))
    } else {
        (i64::from(width), i64::from(height))
    };
    // Clipped to the image plus the pad above, so these fit an integer.
    let first = ((a[0].min(b[0]) - pad).floor() as i64).max(0);
    let last = ((a[0].max(b[0]) + pad).floor() as i64).min(u_size - 1);
    for pu in first..=last {
        let su = pu as f64 + 0.5;
        let t = if du != 0.0 {
            ((su - a[0]) / du).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let centre = (a[1] + dv * t).floor() as i64;
        for pv in (centre - reach).max(0)..=(centre + reach).min(v_size - 1) {
            let sv = pv as f64 + 0.5;
            // Distance from the pixel centre to the segment.
            let along = if length > 0.0 {
                (((su - a[0]) * du + (sv - a[1]) * dv) / (length * length)).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let beyond = if length > 0.0 {
                ((su - a[0]) * du + (sv - a[1]) * dv) / (length * length)
            } else {
                0.0
            };
            let distance = (su - (a[0] + du * along)).phypot(sv - (a[1] + dv * along));
            let alpha = (half + 0.5 - distance).clamp(0.0, 1.0);
            if alpha <= 0.0 {
                continue;
            }
            let (px, py) = if steep { (pv, pu) } else { (pu, pv) };
            // Both are within the image: non-negative and below its size.
            visit(
                py as usize * width as usize + px as usize,
                alpha,
                distance,
                // Along the whole segment, not the part clipped above.
                t0 + beyond * (t1 - t0),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft3d_math::Portable;

    /// A sequence of numbers in [0, 1) from a seed, the same every run.
    fn sequence(seed: u64) -> impl FnMut() -> f64 {
        let mut state = seed;
        move || {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (state >> 11) as f64 / (1u64 << 53) as f64
        }
    }

    /// On a plane the reconstruction is exact: a point of the plane is
    /// never hidden by it, and a point behind it by more than the grain's
    /// share of its change and the slack is, wherever in the cell and
    /// however steep.
    #[test]
    fn on_a_plane_the_depth_test_is_exact() {
        let mut next = sequence(7);
        for _ in 0..10_000 {
            let base = 1.0 + next() * 1e5;
            let (gx, gy) = ((next() - 0.5) * 2e3, (next() - 0.5) * 2e3);
            let plane = |x: f64, y: f64| base + gx * x + gy * y;
            let mut z = [[0.0; 4]; 4];
            for (r, row) in z.iter_mut().enumerate() {
                for (k, cell) in row.iter_mut().enumerate() {
                    *cell = plane(k as f64 - 1.0, r as f64 - 1.0);
                }
            }
            let (fu, fv) = (next(), next());
            let depth = plane(fu, fv);
            let slack = base * 1e-6;
            let allowance = gx.abs().max(gy.abs()) * SUBPIXEL_GRAIN + slack;
            let scale = depth.abs() * 1e-9;
            assert!(!surface_hides(&z, fu, fv, depth + allowance - scale, slack));
            assert!(surface_hides(&z, fu, fv, depth + allowance + scale, slack));
        }
    }

    /// Two planes meeting in a crease through the cell, at any angle,
    /// with any slopes on either side (a hollow, a ridge, a change of
    /// slope): the point on the crease is never hidden, since the kink
    /// covers what the reconstruction misses there; and a point far
    /// behind the crease is.
    #[test]
    fn the_kink_covers_a_crease_between_the_samples() {
        let mut next = sequence(11);
        for _ in 0..20_000 {
            let (fu, fv) = (next(), next());
            let angle = next() * std::f64::consts::TAU;
            let (nx, ny) = (angle.pcos(), angle.psin());
            let base = 1.0 + next() * 1e5;
            let along = (next() - 0.5) * 2e3;
            let (s1, s2) = ((next() - 0.5) * 4e3, (next() - 0.5) * 4e3);
            let surface = |x: f64, y: f64| {
                let across = (x - fu) * nx + (y - fv) * ny;
                let beside = -(x - fu) * ny + (y - fv) * nx;
                base + along * beside
                    + if across < 0.0 {
                        s1 * across
                    } else {
                        s2 * across
                    }
            };
            let mut z = [[0.0; 4]; 4];
            for (r, row) in z.iter_mut().enumerate() {
                for (k, cell) in row.iter_mut().enumerate() {
                    *cell = surface(k as f64 - 1.0, r as f64 - 1.0);
                }
            }
            let slack = base * 1e-6;
            assert!(
                !surface_hides(&z, fu, fv, base, slack),
                "a point of the crease at ({fu}, {fv}), slopes {s1} and {s2} at {angle}"
            );
            let far = base + 8.0 * (s1.abs() + s2.abs() + along.abs()) + 1.0;
            assert!(surface_hides(&z, fu, fv, far, slack));
        }
    }

    /// A step to a nearer body between the samples gets no allowance: a
    /// point behind the further level is hidden right up to the step,
    /// while the crease of a hollow of the same size is in sight. The
    /// trade-off pinned here: within the cell the step crosses, the
    /// reconstruction lies between the two levels, so a point *on* the
    /// further level there is hidden too (a line on the far face stops up
    /// to a pixel short of the nearer body's outline); on either sample
    /// the levels themselves are in sight.
    #[test]
    fn a_step_hides_what_a_hollow_shows() {
        let mut step = [[392.0; 4]; 4];
        for row in &mut step {
            row[2] = 362.0;
            row[3] = 362.0;
        }
        assert!(surface_hides(&step, 0.0, 0.0, 402.0, 0.0));
        assert!(surface_hides(&step, 0.9, 0.5, 402.0, 0.0));
        // The levels themselves, at the samples on their side of the step.
        assert!(!surface_hides(&step, 0.0, 0.0, 392.0, 0.0));
        assert!(!surface_hides(&step, 1.0, 0.5, 362.0, 0.0));
        assert!(surface_hides(
            &step,
            1.0,
            0.5,
            362.0 + 30.0 * SUBPIXEL_GRAIN + 1e-9,
            0.0
        ));
        // The far level half-way across the step's cell: hidden, by the
        // trade-off above (the reconstruction is 377 there).
        assert!(surface_hides(&step, 0.5, 0.5, 392.0, 0.0));
        assert!(!surface_hides(
            &step,
            0.5,
            0.5,
            377.0 + 30.0 * SUBPIXEL_GRAIN,
            0.0
        ));
        let mut hollow = [[0.0; 4]; 4];
        for row in &mut hollow {
            for (k, cell) in row.iter_mut().enumerate() {
                // The crease at x = 1.5, between the cell's samples, both
                // sides nearer by 30 per pixel.
                *cell = 392.0 - 30.0 * (k as f64 - 1.5).abs();
            }
        }
        assert!(!surface_hides(&hollow, 0.5, 0.5, 392.0, 0.0));
        let allowance = 30.0 + 30.0 * SUBPIXEL_GRAIN;
        assert!(surface_hides(
            &hollow,
            0.5,
            0.5,
            392.0 + allowance + 1e-9,
            0.0
        ));
    }

    /// Nothing drawn among the four samples around the point hides
    /// nothing; a kink term that needs one is left out; a surface right
    /// at the eye hides everything.
    #[test]
    fn nothing_drawn_hides_nothing() {
        let mut z = [[5.0; 4]; 4];
        assert!(surface_hides(&z, 0.5, 0.5, 6.0, 0.0));
        z[0][0] = f64::INFINITY;
        assert!(surface_hides(&z, 0.5, 0.5, 6.0, 0.0));
        z[1][0] = f64::INFINITY;
        z[1][3] = f64::INFINITY;
        assert!(surface_hides(&z, 0.5, 0.5, 6.0, 0.0));
        z[1][1] = f64::INFINITY;
        assert!(!surface_hides(&z, 0.5, 0.5, 6.0, 0.0));
        z[1][1] = f64::NEG_INFINITY;
        assert!(surface_hides(&z, 0.5, 0.5, 6.0, 0.0));
        let z = [[f64::NAN; 4]; 4];
        assert!(!surface_hides(&z, 0.5, 0.5, 6.0, 0.0));
        let z = [[5.0; 4]; 4];
        assert!(!surface_hides(&z, 0.5, 0.5, f64::NAN, 0.0));
        assert!(!surface_hides(&z, f64::NAN, 0.5, 6.0, 0.0));
    }

    /// A face that goes on from an edge, at any slope and in either
    /// projection, is no step; a surface behind the edge by more than the
    /// allowance is, and so is nothing beside it; a surface in front is
    /// not.
    #[test]
    fn a_face_that_goes_on_is_no_step_and_a_drop_is() {
        let mut next = sequence(5);
        for _ in 0..10_000 {
            let depth = 1.0 + next() * 1e4;
            let slope = (next() - 0.5) * depth;
            let allowance = depth * 1e-6;
            // Orthographic: the distance is affine across the screen.
            let at = |x: f64| depth + slope * x;
            assert!(!surface_steps_back(
                depth,
                Some(at(OUTLINE_NEAR)),
                Some(at(OUTLINE_FAR)),
                false,
                allowance
            ));
            // Perspective: its reciprocal is.
            let k = (next() - 0.5) / depth / 4.0;
            let at = |x: f64| 1.0 / (1.0 / depth + k * x);
            assert!(!surface_steps_back(
                depth,
                Some(at(OUTLINE_NEAR)),
                Some(at(OUTLINE_FAR)),
                true,
                allowance
            ));
        }
        assert!(surface_steps_back(100.0, None, None, true, 1.0));
        assert!(surface_steps_back(
            100.0,
            Some(110.0),
            Some(110.0),
            true,
            1.0
        ));
        assert!(!surface_steps_back(
            100.0,
            Some(100.5),
            Some(100.5),
            true,
            1.0
        ));
        assert!(!surface_steps_back(
            100.0,
            Some(90.0),
            Some(90.0),
            false,
            1.0
        ));
        // The near side only, sloping: its change is counted as slope.
        assert!(!surface_steps_back(100.0, Some(101.0), None, false, 1.0));
        assert!(surface_steps_back(100.0, Some(120.0), None, false, 1.0));
        assert!(!surface_steps_back(f64::NAN, None, None, false, 1.0));
        assert!(!surface_steps_back(100.0, None, None, false, f64::NAN));
    }

    /// An edge fades smoothly by its extent on screen, and in a crowded
    /// tile by the cap over the density, never below the floor.
    #[test]
    fn edges_fade_smoothly_for_being_short_and_by_the_cap_where_they_crowd() {
        let tokens = Tokens::default();
        let (short, long) = (
            f64::from(tokens.edge_fade_short),
            f64::from(tokens.edge_fade_long),
        );
        assert_eq!(fade_short(short, &tokens, 1.0), 0.0);
        assert_eq!(fade_short(long, &tokens, 1.0), 1.0);
        assert_eq!(fade_short(f64::INFINITY, &tokens, 1.0), 1.0);
        assert_eq!(fade_short(2.0 * long, &tokens, 2.0), 1.0);
        let mut last = 0.0;
        for k in 0..=100 {
            let keep = fade_short(short + (long - short) * f64::from(k) / 100.0, &tokens, 1.0);
            assert!(keep >= last && keep - last < 0.02, "a jump at {k}");
            last = keep;
        }
        let floor = f64::from(tokens.edge_fade_floor);
        let cap = f64::from(tokens.edge_dense_cap);
        assert_eq!(edge_fade(1.0, 0.0, &tokens), 1.0);
        assert_eq!(edge_fade(1.0, cap, &tokens), 1.0);
        let half = edge_fade(1.0, 2.0 * cap, &tokens);
        assert!((half - (floor + (1.0 - floor) * 0.5)).abs() < 1e-12);
        assert_eq!(edge_fade(0.0, 0.0, &tokens), floor);
        assert_eq!(edge_fade(1.0, f64::INFINITY, &tokens), floor);
        assert_eq!(edge_fade(f64::NAN, 0.0, &tokens), 1.0);
        assert_eq!(smooth_step(1.0, 1.0, 1.0), 1.0);
        assert_eq!(smooth_step(1.0, 1.0, 0.5), 0.0);
        assert_eq!(smooth_step(0.0, 2.0, f64::NAN), 0.0);
    }

    /// One edge of one pixel across a tile covers a tile's side over its
    /// area of it; the density between tiles is blended, so it changes
    /// smoothly as the edge moves; hidden edges and other lines count for
    /// nothing.
    #[test]
    fn the_density_counts_the_ink_of_the_edges_in_sight_per_tile() {
        let tokens = Tokens::default();
        let edge = |y: f64, hidden: bool| OverlayLine {
            from: [-10.0, y],
            to: [80.0, y],
            kind: LineKind::Edge,
            hidden,
            edge: Some(EdgeInk {
                body: None,
                extent: f64::INFINITY,
                silhouette: false,
                joined: false,
                cut: [false; 2],
                depth: [1.0; 2],
            }),
        };
        let mut overlay = Overlay::default();
        overlay.lines.push(edge(8.0, false));
        overlay.lines.push(edge(40.0, true));
        overlay.lines.push(OverlayLine {
            edge: None,
            kind: LineKind::Sketch,
            ..edge(56.0, false)
        });
        let density = EdgeDensity::new(&overlay, 64, 64, &tokens, 1.0);
        let one = f64::from(tokens.edge_width) / 16.0;
        // At a tile's centre the tile's own mean.
        assert!((density.at(8.0, 8.0) - one).abs() < 1e-9);
        assert!((density.at(40.0, 8.0) - one).abs() < 1e-9);
        assert_eq!(density.at(8.0, 40.0), 0.0);
        assert_eq!(density.at(8.0, 56.0), 0.0);
        // Half-way between the first and the second row of tiles.
        assert!((density.at(8.0, 16.0) - one / 2.0).abs() < 1e-9);
        // Blended between the tiles' centres: no step where the point
        // crosses from one tile into the next, at most the slope of the
        // blend (a tile's mean over its side, per pixel).
        let mut last = density.at(8.0, 0.0);
        for k in 1..=160 {
            let at = density.at(8.0, f64::from(k) * 0.25);
            assert!(
                (at - last).abs() <= one / 16.0 * 0.25 * 1.000_001,
                "a jump at {k}: {last} -> {at}"
            );
            last = at;
        }
        // Nothing to measure, and hostile positions: no density.
        let empty = EdgeDensity::new(&Overlay::default(), 0, 0, &tokens, 1.0);
        assert_eq!(empty.at(1.0, 1.0), 0.0);
        assert_eq!(density.at(f64::NAN, 1.0), 0.0);
        assert!(density.at(-1e300, 1e300).is_finite());
    }

    const BLACK: Rgb = [0, 0, 0];
    const WHITE: Rgb = [255, 255, 255];

    fn canvas(w: u32, h: u32) -> Vec<u8> {
        vec![0; (w * h * 3) as usize]
    }

    fn pixel(rgb: &[u8], w: u32, x: u32, y: u32) -> Rgb {
        let i = ((y * w + x) * 3) as usize;
        [rgb[i], rgb[i + 1], rgb[i + 2]]
    }

    #[test]
    fn a_horizontal_line_covers_its_row_and_nothing_far_from_it() {
        let (w, h) = (40, 20);
        let mut rgb = canvas(w, h);
        // Along the middle of row 10, two pixels wide.
        draw_line(&mut rgb, w, h, [5.0, 10.5], [30.0, 10.5], 2.0, WHITE, 255);
        for x in 6..30 {
            assert_eq!(pixel(&rgb, w, x, 10), WHITE, "x {x}");
            // Half covered above and below: the line is 2 wide about the
            // centre of row 10, so it reaches half-way into rows 9 and 11.
            for y in [9, 11] {
                let [v, ..] = pixel(&rgb, w, x, y);
                assert!((100..=160).contains(&v), "({x}, {y}) is {v}");
            }
            for y in [0, 5, 7, 13, 19] {
                assert_eq!(pixel(&rgb, w, x, y), BLACK, "({x}, {y})");
            }
        }
        // Round ends: nothing more than the half width past them.
        for x in [0, 1, 2, 33, 39] {
            assert_eq!(pixel(&rgb, w, x, 10), BLACK, "x {x}");
        }
    }

    #[test]
    fn a_diagonal_line_is_as_heavy_as_a_straight_one() {
        // The ink on a line is its length times its width, whatever its
        // direction: 20 long, 2 wide, plus the round ends (π · 1²).
        let expected = 20.0 * 2.0 + std::f64::consts::PI;
        let (w, h) = (60, 60);
        let (s, c) = (0.6_f64, 0.8_f64);
        for (from, to) in [
            ([10.0, 30.0], [30.0, 30.0]),
            ([30.0, 10.0], [30.0, 30.0]),
            ([10.0, 10.0], [10.0 + 20.0 * c, 10.0 + 20.0 * s]),
            ([40.0, 10.0], [40.0 - 20.0 * s, 10.0 + 20.0 * c]),
        ] {
            let mut rgb = canvas(w, h);
            draw_line(&mut rgb, w, h, from, to, 2.0, WHITE, 255);
            let ink: f64 = rgb
                .as_chunks::<3>()
                .0
                .iter()
                .map(|p| f64::from(p[0]) / 255.0)
                .sum();
            assert!(
                (ink - expected).abs() < expected * 0.06,
                "{from:?} -> {to:?}: {ink} vs {expected}"
            );
        }
    }

    #[test]
    fn lines_with_hostile_coordinates_draw_what_is_inside_and_never_panic() {
        let (w, h) = (32, 24);
        let ends = [
            -1e300,
            -1e9,
            -1.0,
            0.0,
            12.5,
            31.999,
            1e9,
            1e300,
            f64::NAN,
            f64::INFINITY,
        ];
        let mut rgb = canvas(w, h);
        for &ax in &ends {
            for &ay in &ends {
                for &b in &ends {
                    for width in [0.0, 1.5, 1e9, f64::NAN, -2.0] {
                        draw_line(&mut rgb, w, h, [ax, ay], [b, 12.0], width, WHITE, 255);
                    }
                }
            }
        }
        assert_eq!(rgb.len(), (w * h * 3) as usize);
        // A line from far outside to far outside still crosses the image.
        let mut rgb = canvas(w, h);
        draw_line(&mut rgb, w, h, [-1e9, 12.5], [1e9, 12.5], 1.0, WHITE, 255);
        for x in 0..w {
            assert_eq!(pixel(&rgb, w, x, 12), WHITE, "x {x}");
        }
        // No image, no drawing.
        draw_line(&mut [], 0, 0, [0.0, 0.0], [5.0, 5.0], 1.0, WHITE, 255);
        // A buffer shorter than its claimed size is not indexed past.
        let mut short = vec![0; 10];
        draw_line(&mut short, w, h, [0.0, 0.0], [31.0, 23.0], 3.0, WHITE, 255);
    }

    #[test]
    fn the_axes_are_drawn_from_the_origin_with_x_to_the_right_seen_from_the_front() {
        let mut camera = CameraState::default();
        camera.set_view("front").unwrap();
        let overlay = overlay(None, &camera, 400, 400, None);
        assert_eq!(overlay.lines.len(), 3);
        assert!(overlay.points.is_empty());
        let [x, y, z] = [overlay.lines[0], overlay.lines[1], overlay.lines[2]];
        assert_eq!(
            (x.kind, y.kind, z.kind),
            (LineKind::Axis(0), LineKind::Axis(1), LineKind::Axis(2))
        );
        for line in [x, y, z] {
            assert!((line.from[0] - 200.0).abs() < 1e-9 && (line.from[1] - 200.0).abs() < 1e-9);
        }
        // 6 % of the distance at the target's depth: the viewport shows
        // 2 · tan(15°) of the distance over 400 pixels.
        let length = 0.06 / (2.0 * 15.0_f64.to_radians().ptan()) * 400.0;
        assert!((x.to[0] - (200.0 + length)).abs() < 1e-9, "{x:?}");
        assert!((x.to[1] - 200.0).abs() < 1e-9);
        assert!((z.to[1] - (200.0 - length)).abs() < 1e-9, "{z:?}");
        assert!((z.to[0] - 200.0).abs() < 1e-9);
        // Y points away from the viewer: it stays on the middle, a little
        // foreshortened towards it.
        assert!((y.to[0] - 200.0).abs() < 1e-9 && (y.to[1] - 200.0).abs() < 1e-9);
    }
}

/// A scene whose bodies are placed as rigid copies (an assembly, at
/// rest or at a moment of its motion: [`craft3d_engine::SceneBody::placement`])
/// as parts for the GPU: each body's source mesh and its pose, what a
/// renderer uploads once and then moves ([`craft3d_gpu::Renderer::set_poses`]).
pub struct ScenePartsOf<'a> {
    pub parts: Vec<craft3d_gpu::MeshPart<'a>>,
    pub poses: Vec<craft3d_math::Transform>,
    pub key: PartsKey,
}

/// What a scene's parts were uploaded from: the source meshes (held, so
/// that no other mesh can be where one was), where each body's face ids
/// start, and the face colours. Two scenes with the same key differ only
/// in their poses.
#[derive(Clone, Debug)]
pub struct PartsKey {
    meshes: Vec<(std::sync::Arc<craft3d_tess::Mesh>, u32)>,
    colors: craft3d_render::FaceColors,
}

impl PartialEq for PartsKey {
    fn eq(&self, other: &Self) -> bool {
        self.meshes.len() == other.meshes.len()
            && self
                .meshes
                .iter()
                .zip(&other.meshes)
                .all(|((a, fa), (b, fb))| std::sync::Arc::ptr_eq(a, b) && fa == fb)
            && self.colors == other.colors
    }
}

/// The parts of `scene` when any of its bodies is placed; `None` for a
/// scene of bodies meshed themselves (a part), which is uploaded whole.
/// A body that is not placed is a part of its own mesh at the identity.
pub fn scene_parts(scene: &Scene) -> Option<ScenePartsOf<'_>> {
    if scene.bodies.iter().all(|b| b.placement.is_none()) {
        return None;
    }
    let mut parts = Vec::with_capacity(scene.bodies.len());
    let mut poses = Vec::with_capacity(scene.bodies.len());
    let mut meshes = Vec::with_capacity(scene.bodies.len());
    for body in scene.bodies.iter() {
        let (mesh, pose) = match &body.placement {
            Some(placed) => (&placed.source, placed.pose),
            None => (&body.mesh, craft3d_math::Transform::IDENTITY),
        };
        parts.push(craft3d_gpu::MeshPart {
            mesh,
            first_face_id: body.first_face_id,
            pose,
        });
        poses.push(pose);
        meshes.push((std::sync::Arc::clone(mesh), body.first_face_id));
    }
    Some(ScenePartsOf {
        parts,
        poses,
        key: PartsKey {
            meshes,
            colors: (*scene.face_colors).clone(),
        },
    })
}
