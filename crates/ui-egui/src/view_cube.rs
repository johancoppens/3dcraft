//! The view cube: a small glass cube in a corner of the viewport that
//! turns with the camera. It is one rounded family of 26 patches with one
//! gap between them all: six faces (top, front, right, ...) as rounded
//! squares, twelve capsules where the edges are and eight circles where
//! the corners are. A capsule is as wide as a circle; a circle lies in its
//! corner's own plane (square to the cube's diagonal), between the round
//! ends of three capsules. Underneath is a cube with its edges and corners
//! cut off ([`Facet::outline`]): every patch sits on one of its facets,
//! and its outline is the cube's silhouette. A click on a patch turns the
//! camera to look at the model from that side: a face gives a standard
//! view, a capsule the view half-way between two of them, a circle the
//! view from a corner. A click on the patch already faced gives the view
//! from the other side, and a drag on the cube orbits the camera as a drag
//! in the viewport does. The cube is always drawn without perspective,
//! whatever the viewport's projection. Beside it a small triad ([`triad`])
//! shows where the world's X, Y and Z point.
//!
//! No egui here: where the patches are and which one a position is on are
//! plain numbers, so the shell paints them and tests ask for them. Their
//! proportions come from the theme ([`style`]). Nothing of it is in a
//! document or in the control protocol: the camera it sets is
//! `UiState::camera`, which `ui.set` sets too.

use crate::state::CameraState;
use crate::theme::Tokens;
use craft3d_math::Vec3;
use craft3d_render::ViewAxes;

/// The proportions of the cube's patches, in units of the cube's half
/// side, so that they scale with it ([`style`] takes them from the theme).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CubeStyle {
    /// How much of an edge is cut off on either side of it: the faces'
    /// flat part reaches 1 - `chamfer` from the middle of a face.
    pub chamfer: f64,
    /// The one gap between any two neighbouring patches.
    pub gap: f64,
    /// The corner radius of a face, as a fraction of `chamfer`.
    pub face_rounding: f64,
    /// A patch that faces the viewer less than this (0: edge-on, 1:
    /// straight on) is not drawn, so no sliver sits on the silhouette, and
    /// a click there goes to the nearest patch that is.
    pub fade: f64,
}

impl CubeStyle {
    /// Numbers that make a cube: finite, the gap narrower than a capsule
    /// is wide. What is not is the theme's own value or the nearest that is.
    fn sane(self) -> Self {
        let pick = |v: f64, lo: f64, hi: f64, or: f64| {
            if v.is_finite() { v.clamp(lo, hi) } else { or }
        };
        let chamfer = pick(self.chamfer, 0.02, 0.45, 0.22);
        Self {
            chamfer,
            gap: pick(self.gap, 0.0, chamfer * 1.2, 0.0),
            face_rounding: pick(self.face_rounding, 0.0, 2.0, 0.9),
            fade: pick(self.fade, 0.0, 0.95, 0.25),
        }
    }
}

/// The cube's proportions as the theme has them.
pub fn style(tokens: &Tokens) -> CubeStyle {
    CubeStyle {
        chamfer: f64::from(tokens.view_cube_chamfer),
        gap: f64::from(tokens.view_cube_gap),
        face_rounding: f64::from(tokens.view_cube_face_rounding),
        fade: f64::from(tokens.view_cube_fade),
    }
}

/// What a patch stands for on the cube.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FacetKind {
    /// A rounded face.
    Face,
    /// A capsule along an edge.
    Edge,
    /// A circle at a corner.
    Corner,
}

/// Where a face's name lies in the picture: the middle of the face and
/// the steps of one half side along the name's right and down, all in the
/// units of [`Facet::outline`]. A point (x, y) of the name, in half sides
/// from its middle, is at `center + x * right + y * down`: the name lies in
/// the face's plane, upright on the side faces and readable from the
/// front on the top and the bottom.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LabelFrame {
    pub center: [f64; 2],
    pub right: [f64; 2],
    pub down: [f64; 2],
    /// One half side of the cube seen straight on, in the same units.
    pub half_side: f64,
}

/// One patch of the view cube as a camera sees it.
#[derive(Clone, Debug, PartialEq)]
pub struct Facet {
    /// The side of the model it stands for: -1, 0 or 1 along X, Y and Z.
    pub side: [i8; 3],
    /// The facet of the cut cube underneath, in the picture of the cube:
    /// from the cube's centre, x to the right, y down, in units of the
    /// picture's radius (everything of the cube is within the unit
    /// circle). The outlines of the facets that face the viewer tile the
    /// cube's silhouette without gaps.
    pub outline: Vec<[f64; 2]>,
    /// The patch that is painted, a convex polygon in the same units: a
    /// rounded square, a capsule or a circle, seen without perspective.
    pub patch: Vec<[f64; 2]>,
    /// How much it faces the viewer, 0 (edge-on) to 1.
    pub toward: f64,
    /// Where its name lies, for a face.
    pub label_frame: Option<LabelFrame>,
}

impl Facet {
    pub fn kind(&self) -> FacetKind {
        kind_of(self.side)
    }

    /// The name of a face, as the standard views are called; nothing for
    /// an edge or a corner.
    pub fn label(&self) -> Option<&'static str> {
        label_of(self.side)
    }

    /// The axis a face is across (0 X, 1 Y, 2 Z).
    pub fn axis(&self) -> Option<usize> {
        (self.kind() == FacetKind::Face)
            .then(|| self.side.iter().position(|&c| c != 0))
            .flatten()
    }

    /// Its direction in the world, of length 1: how a light lights it.
    pub fn normal(&self) -> [f64; 3] {
        direction(self.side).map_or([0.0; 3], |d| [d.x, d.y, d.z])
    }

    /// The middle of its patch: on the patch, since the patch is convex.
    pub fn center(&self) -> [f64; 2] {
        let n = self.patch.len().max(1) as f64;
        let sum = self
            .patch
            .iter()
            .fold([0.0, 0.0], |acc, p| [acc[0] + p[0], acc[1] + p[1]]);
        [sum[0] / n, sum[1] / n]
    }
}

pub fn kind_of(side: [i8; 3]) -> FacetKind {
    match side.iter().filter(|&&c| c != 0).count() {
        1 => FacetKind::Face,
        2 => FacetKind::Edge,
        _ => FacetKind::Corner,
    }
}

pub fn label_of(side: [i8; 3]) -> Option<&'static str> {
    match side {
        [1, 0, 0] => Some("right"),
        [-1, 0, 0] => Some("left"),
        [0, 1, 0] => Some("back"),
        [0, -1, 0] => Some("front"),
        [0, 0, 1] => Some("top"),
        [0, 0, -1] => Some("bottom"),
        _ => None,
    }
}

/// Every side a patch stands for: 26 of them.
pub fn sides() -> impl Iterator<Item = [i8; 3]> {
    (-1..=1_i8)
        .flat_map(|x| (-1..=1_i8).flat_map(move |y| (-1..=1_i8).map(move |z| [x, y, z])))
        .filter(|side| *side != [0, 0, 0])
}

fn direction(side: [i8; 3]) -> Option<Vec3> {
    let [x, y, z] = side.map(f64::from);
    Vec3::new(x, y, z).normalized()
}

/// The point with `values` on the axes they name (0 X, 1 Y, 2 Z).
fn point(values: [(usize, f64); 3]) -> Vec3 {
    let mut c = [0.0; 3];
    for (axis, value) in values {
        if let Some(slot) = c.get_mut(axis) {
            *slot = value;
        }
    }
    let [x, y, z] = c;
    Vec3::new(x, y, z)
}

/// The unit vector along `axis`.
fn unit(axis: usize) -> Vec3 {
    point([(axis, 1.0), (3, 0.0), (3, 0.0)])
}

/// The facet for `side` of the cube from -1 to 1 with its edges and
/// corners cut off `chamfer` deep: its corners in space.
fn outline(side: [i8; 3], chamfer: f64) -> Vec<Vec3> {
    let k = 1.0 - chamfer;
    let s = side.map(f64::from);
    let free: Vec<usize> = (0..3).filter(|&a| side.get(a) == Some(&0)).collect();
    let fixed: Vec<usize> = (0..3).filter(|&a| side.get(a) != Some(&0)).collect();
    let of = |axis: usize| s.get(axis).copied().unwrap_or(0.0);
    match (fixed.as_slice(), free.as_slice()) {
        // A face: the square left of it.
        (&[a], &[b, c]) => [(1.0, 1.0), (-1.0, 1.0), (-1.0, -1.0), (1.0, -1.0)]
            .into_iter()
            .map(|(u, v)| point([(a, of(a)), (b, u * k), (c, v * k)]))
            .collect(),
        // Where an edge was: a strip between two faces.
        (&[a, b], &[f]) => [(1.0, k, 1.0), (k, 1.0, 1.0), (k, 1.0, -1.0), (1.0, k, -1.0)]
            .into_iter()
            .map(|(u, v, w)| point([(a, of(a) * u), (b, of(b) * v), (f, w * k)]))
            .collect(),
        // Where a corner was: a triangle between three faces.
        _ => [(1.0, k, k), (k, 1.0, k), (k, k, 1.0)]
            .into_iter()
            .map(|(u, v, w)| point([(0, of(0) * u), (1, of(1) * v), (2, of(2) * w)]))
            .collect(),
    }
}

/// How many straight pieces a quarter of a round corner, a capsule's end
/// and a whole circle are drawn with.
const QUARTER: usize = 8;
const END: usize = 12;
const ROUND: usize = 40;

/// One patch in space, before a camera sees it.
struct Patch {
    side: [i8; 3],
    normal: Vec3,
    outline: Vec<Vec3>,
    patch: Vec<Vec3>,
    /// For a face: its middle and the directions of its name's right and
    /// down.
    label: Option<[Vec3; 3]>,
}

/// The 26 patches of the cube, in space, in the order of [`sides`].
fn patches(style: &CubeStyle) -> Vec<Patch> {
    let style = style.sane();
    let (c, g) = (style.chamfer, style.gap);
    let k = 1.0 - c;
    // A capsule is as wide as the strip it lies on less the gap, and a
    // corner's circle as wide as a capsule.
    let r = ((c * std::f64::consts::SQRT_2 - g) * 0.5).max(1e-6);
    // The middle of a corner's triangle, along each axis, and the middle
    // of a strip across it.
    let m = (1.0 + 2.0 * k) / 3.0;
    let mid = (1.0 + k) * 0.5;
    // A capsule's end is the gap away from the circle of each corner it
    // points at (centre to centre: two radii and the gap).
    let reach = 2.0 * r + g;
    let left = reach * reach - 2.0 * (mid - m) * (mid - m);
    let end = if left > 0.0 { m - left.sqrt() } else { m }.clamp(0.0, k);
    let mut out = Vec::new();
    for side in sides() {
        let Some(normal) = direction(side) else {
            continue;
        };
        let s = side.map(f64::from);
        let of = |axis: usize| s.get(axis).copied().unwrap_or(0.0);
        let fixed: Vec<usize> = (0..3).filter(|&a| side.get(a) != Some(&0)).collect();
        let free: Vec<usize> = (0..3).filter(|&a| side.get(a) == Some(&0)).collect();
        let (patch, label) = match (fixed.as_slice(), free.as_slice()) {
            (&[a], &[u, v]) => {
                // A rounded square, the gap's half in from the face's
                // flat part on every side.
                let half = k - g * 0.5;
                let rho = (style.face_rounding * c).clamp(0.0, half.max(0.0));
                let inner = half - rho;
                let mut points = Vec::new();
                for (cu, cv, start) in [
                    (inner, -inner, -90.0_f64),
                    (inner, inner, 0.0),
                    (-inner, inner, 90.0),
                    (-inner, -inner, 180.0),
                ] {
                    for j in 0..=QUARTER {
                        let angle = (start + 90.0 * j as f64 / QUARTER as f64).to_radians();
                        points.push(point([
                            (a, of(a)),
                            (u, cu + rho * angle.cos()),
                            (v, cv + rho * angle.sin()),
                        ]));
                    }
                }
                // The name: on the top and the bottom its right is +X and
                // its top towards the back (the top) or the front (the
                // bottom), so both read from the front; on a side face it
                // stands upright.
                let (right, down) = if a == 2 {
                    (Vec3::X, unit(1) * -of(2))
                } else {
                    (Vec3::Z.cross(normal), -Vec3::Z)
                };
                (points, Some([normal, right, down]))
            }
            (&[a, b], &[f]) => {
                // A capsule along the edge: its centre line in the middle
                // of the strip, round ends `end` from the edge's middle.
                let along = unit(f);
                let across = (unit(a) * of(a) - unit(b) * of(b)) * std::f64::consts::FRAC_1_SQRT_2;
                let centre = |t: f64| point([(a, of(a) * mid), (b, of(b) * mid), (f, t)]);
                let mut points = Vec::new();
                for (t, start) in [(end, -90.0_f64), (-end, 90.0)] {
                    for j in 0..=END {
                        let angle = (start + 180.0 * j as f64 / END as f64).to_radians();
                        points.push(
                            centre(t) + along * (r * angle.cos()) + across * (r * angle.sin()),
                        );
                    }
                }
                (points, None)
            }
            _ => {
                // A circle in the corner's own plane, in the middle of its
                // triangle.
                let centre = point([(0, of(0) * m), (1, of(1) * m), (2, of(2) * m)]);
                let (Some(t1), Some(t2)) = (
                    normal.cross(Vec3::Z).normalized(),
                    normal.cross(normal.cross(Vec3::Z)).normalized(),
                ) else {
                    continue;
                };
                let points = (0..ROUND)
                    .map(|j| {
                        let angle = std::f64::consts::TAU * j as f64 / ROUND as f64;
                        centre + t1 * (r * angle.cos()) + t2 * (r * angle.sin())
                    })
                    .collect();
                (points, None)
            }
        };
        out.push(Patch {
            side,
            normal,
            outline: outline(side, c),
            patch,
            label,
        });
    }
    out
}

/// How a camera sees the cube: without perspective, in units of the
/// picture's radius (the furthest any patch or facet reaches from the
/// cube's centre).
struct Picture {
    axes: ViewAxes,
    radius: f64,
}

impl Picture {
    fn new(camera: &CameraState, patches: &[Patch]) -> Option<Self> {
        let axes = camera.to_render().axes().ok()?;
        let radius = patches
            .iter()
            .flat_map(|p| p.patch.iter().chain(&p.outline))
            .map(|p| p.length())
            .fold(1.0_f64, f64::max);
        radius.is_finite().then_some(Self { axes, radius })
    }

    fn place(&self, p: Vec3) -> [f64; 2] {
        [
            p.dot(self.axes.right) / self.radius,
            -p.dot(self.axes.up) / self.radius,
        ]
    }

    /// A direction in space as a step in the picture.
    fn step(&self, d: Vec3) -> [f64; 2] {
        self.place(d)
    }

    /// How much a patch with `normal` faces the viewer.
    fn toward(&self, normal: Vec3) -> f64 {
        -normal.dot(self.axes.forward)
    }
}

/// Every patch that faces `camera` at all, with how much: the patches
/// shown and the ones faded out.
fn seen(camera: &CameraState, style: &CubeStyle) -> Vec<Facet> {
    let patches = patches(style);
    let Some(picture) = Picture::new(camera, &patches) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for p in patches {
        let toward = picture.toward(p.normal);
        // The cube is convex: a facet shows exactly when it faces the
        // viewer. A number that is not one shows nothing.
        if toward <= 1e-9 || toward.is_nan() {
            continue;
        }
        let place = |points: &[Vec3]| -> Vec<[f64; 2]> {
            points.iter().map(|&q| picture.place(q)).collect()
        };
        let facet = Facet {
            side: p.side,
            outline: place(&p.outline),
            patch: place(&p.patch),
            toward,
            label_frame: p.label.map(|[centre, right, down]| LabelFrame {
                center: picture.place(centre),
                right: picture.step(right),
                down: picture.step(down),
                half_side: 1.0 / picture.radius,
            }),
        };
        let finite = facet
            .outline
            .iter()
            .chain(&facet.patch)
            .flatten()
            .all(|c| c.is_finite());
        if finite {
            out.push(facet);
        }
    }
    out.sort_by(|a, b| a.toward.total_cmp(&b.toward).then(a.side.cmp(&b.side)));
    out
}

/// The patches `camera` sees with the theme's proportions: [`facets_styled`].
pub fn facets(camera: &CameraState) -> Vec<Facet> {
    facets_styled(camera, &style(&Tokens::default()))
}

/// The patches `camera` sees and that are drawn (they face it at least
/// `style.fade`), the least facing first: the order to paint them in.
/// Nothing for a camera that looks nowhere.
pub fn facets_styled(camera: &CameraState, style: &CubeStyle) -> Vec<Facet> {
    let fade = style.sane().fade;
    seen(camera, style)
        .into_iter()
        .filter(|f| f.toward >= fade)
        .collect()
}

/// The cube's silhouette as `camera` sees it: a convex polygon in the
/// units of [`Facet::outline`], anticlockwise on screen (y down).
pub fn silhouette(camera: &CameraState, style: &CubeStyle) -> Vec<[f64; 2]> {
    let points: Vec<[f64; 2]> = seen(camera, style)
        .into_iter()
        .flat_map(|f| f.outline)
        .collect();
    hull(points)
}

/// The convex hull of `points` (Andrew's monotone chain), without
/// repeated or collinear corners.
fn hull(mut points: Vec<[f64; 2]>) -> Vec<[f64; 2]> {
    points.retain(|p| p.iter().all(|c| c.is_finite()));
    points.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    points.dedup();
    if points.len() < 3 {
        return points;
    }
    let turn = |o: [f64; 2], a: [f64; 2], b: [f64; 2]| {
        (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
    };
    let mut lower: Vec<[f64; 2]> = Vec::new();
    for &p in &points {
        while let [.., a, b] = lower.as_slice() {
            if turn(*a, *b, p) <= 0.0 {
                lower.pop();
            } else {
                break;
            }
        }
        lower.push(p);
    }
    let mut upper: Vec<[f64; 2]> = Vec::new();
    for &p in points.iter().rev() {
        while let [.., a, b] = upper.as_slice() {
            if turn(*a, *b, p) <= 0.0 {
                upper.pop();
            } else {
                break;
            }
        }
        upper.push(p);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

/// How far `at` is inside the convex polygon `polygon` (negative:
/// outside), in its units; nothing for a polygon with no area.
pub fn inside_by(polygon: &[[f64; 2]], at: [f64; 2]) -> Option<f64> {
    let n = polygon.len();
    let area: f64 = (0..n)
        .filter_map(|k| {
            let (a, b) = (polygon.get(k)?, polygon.get((k + 1) % n)?);
            Some(a[0] * b[1] - b[0] * a[1])
        })
        .sum();
    if area == 0.0 || !area.is_finite() {
        return None;
    }
    (0..n)
        .filter_map(|k| {
            let (a, b) = (polygon.get(k)?, polygon.get((k + 1) % n)?);
            let length = (b[0] - a[0]).hypot(b[1] - a[1]);
            (length > 0.0).then(|| {
                let cross = (b[0] - a[0]) * (at[1] - a[1]) - (b[1] - a[1]) * (at[0] - a[0]);
                cross * area.signum() / length
            })
        })
        .reduce(f64::min)
}

/// How opaque a face's name is: nothing on a face seen more nearly
/// edge-on than `hide`, fully a little above it.
pub fn label_opacity(toward: f64, hide: f64) -> f64 {
    if !(toward.is_finite() && hide.is_finite()) || toward < hide {
        return 0.0;
    }
    ((toward - hide) / 0.12).clamp(0.0, 1.0)
}

/// One arm of the axis triad ([`triad`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TriadArm {
    /// The world axis: 0 X, 1 Y, 2 Z.
    pub axis: usize,
    /// Where the arm's tip is, from the triad's origin: x to the right,
    /// y down, the arm's full length 1 (an arm seen end-on is shorter).
    pub tip: [f64; 2],
    /// How much it points at the viewer, -1 (away) to 1 (towards).
    pub toward: f64,
}

/// The world axes as `camera` sees them, for the triad beside the cube:
/// the three arms from a common origin, drawn orthographically (only the
/// directions matter), the one that points most away first so that the
/// nearer ones are painted over it. Nothing for a camera that looks
/// nowhere.
pub fn triad(camera: &CameraState) -> Option<[TriadArm; 3]> {
    let axes = camera.to_render().axes().ok()?;
    let arm = |axis: usize, d: Vec3| TriadArm {
        axis,
        tip: [d.dot(axes.right), -d.dot(axes.up)],
        toward: -d.dot(axes.forward),
    };
    let mut arms = [arm(0, Vec3::X), arm(1, Vec3::Y), arm(2, Vec3::Z)];
    if !arms
        .iter()
        .all(|a| a.tip.iter().all(|c| c.is_finite()) && a.toward.is_finite())
    {
        return None;
    }
    arms.sort_by(|a, b| a.toward.total_cmp(&b.toward).then(a.axis.cmp(&b.axis)));
    Some(arms)
}

/// The patch under `at` (in the units of [`Facet::outline`]) with the
/// theme's proportions: [`facet_at_styled`].
pub fn facet_at(camera: &CameraState, at: [f64; 2]) -> Option<Facet> {
    facet_at_styled(camera, &style(&Tokens::default()), at)
}

/// The patch under `at`: the one drawn there, or, in a gap between
/// patches or over a patch that is faded out, the nearest one drawn, so
/// that nothing on the cube is dead to a click. Nothing outside the cube's
/// silhouette.
pub fn facet_at_styled(camera: &CameraState, style: &CubeStyle, at: [f64; 2]) -> Option<Facet> {
    if !at.iter().all(|c| c.is_finite()) {
        return None;
    }
    let shown = facets_styled(camera, style);
    // Painted last is on top.
    if let Some(on) = shown
        .iter()
        .rev()
        .find(|f| inside_by(&f.patch, at).is_some_and(|d| d >= 0.0))
    {
        return Some(on.clone());
    }
    if !inside_by(&silhouette(camera, style), at).is_some_and(|d| d >= 0.0) {
        return None;
    }
    shown
        .into_iter()
        .filter_map(|f| Some((-inside_by(&f.patch, at)?, f)))
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, f)| f)
}

/// The yaw and pitch, in degrees, of a camera that looks at the model
/// from `side`. From straight above or below the picture's up is +Y, as
/// in the standard views.
pub fn angles(side: [i8; 3]) -> Option<(f64, f64)> {
    let d = direction(side)?;
    let pitch = d.z.clamp(-1.0, 1.0).asin().to_degrees();
    let yaw = if side[0] == 0 && side[1] == 0 {
        -90.0
    } else {
        d.y.atan2(d.x).to_degrees()
    };
    Some((yaw, pitch))
}

/// True when `camera` looks at the model from `side`.
pub fn looks_from(camera: &CameraState, side: [i8; 3]) -> bool {
    let (Some(d), Some(eye)) = (
        direction(side),
        (camera.eye() - camera.target()).normalized(),
    ) else {
        return false;
    };
    d.dot(eye) > 1.0 - 1e-9
}

/// The side a click on `facet` turns `camera` to: its own, or the
/// opposite one when the camera looks from there already.
pub fn clicked(camera: &CameraState, facet: &Facet) -> [i8; 3] {
    if looks_from(camera, facet.side) {
        facet.side.map(|c| -c)
    } else {
        facet.side
    }
}

/// `camera` turned to look from `side`; the target and the distance stay.
pub fn turned(camera: &CameraState, side: [i8; 3]) -> CameraState {
    let mut out = *camera;
    if let Some((yaw, pitch)) = angles(side) {
        out.yaw = yaw;
        out.pitch = pitch;
    }
    out
}

/// The camera a fraction `t` (0 to 1) of the way from `from` to `to`: the
/// yaw the short way round, everything else evenly, eased at both ends.
pub fn between(from: &CameraState, to: &CameraState, t: f64) -> CameraState {
    // A fraction that is not a number is the end.
    if t >= 1.0 || t.is_nan() {
        return *to;
    }
    let t = t.max(0.0);
    let s = t * t * (3.0 - 2.0 * t);
    let mix = |a: f64, b: f64| a + (b - a) * s;
    let turn = (to.yaw - from.yaw + 180.0).rem_euclid(360.0) - 180.0;
    let mut out = *to;
    out.yaw = from.yaw + turn * s;
    out.pitch = mix(from.pitch, to.pitch);
    out.distance = mix(from.distance, to.distance);
    for ((slot, a), b) in out.target.iter_mut().zip(from.target).zip(to.target) {
        *slot = mix(a, b);
    }
    out.validated().unwrap_or(*to)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{ProjectionKind, VIEWS};

    fn cameras() -> [CameraState; 2] {
        [
            CameraState::default(),
            CameraState {
                projection: ProjectionKind::Orthographic,
                ..CameraState::default()
            },
        ]
    }

    fn looking(view: &str) -> CameraState {
        let mut camera = CameraState::default();
        camera.set_view(view).unwrap();
        camera
    }

    #[test]
    fn the_cube_has_26_patches_and_shows_the_ones_that_face_the_camera() {
        assert_eq!(sides().count(), 26);
        let count = |kind: FacetKind| sides().filter(|&side| kind_of(side) == kind).count();
        assert_eq!(
            (
                count(FacetKind::Face),
                count(FacetKind::Edge),
                count(FacetKind::Corner)
            ),
            (6, 12, 8)
        );
        for camera in cameras() {
            // From the default corner: three faces, and the patches
            // between and around them, all within the unit circle.
            let seen = facets(&camera);
            let faces: Vec<_> = seen.iter().filter_map(Facet::label).collect();
            assert_eq!(faces.len(), 3, "{faces:?}");
            for name in ["top", "front", "right"] {
                assert!(faces.contains(&name), "{faces:?}");
            }
            assert!(seen.len() >= 7, "{}", seen.len());
            for facet in &seen {
                assert!(
                    facet
                        .outline
                        .iter()
                        .chain(&facet.patch)
                        .all(|p| p[0].hypot(p[1]) <= 1.0 + 1e-9)
                );
                assert!(facet.toward >= 0.25 && facet.toward <= 1.0 + 1e-12);
                assert_eq!(facet.label_frame.is_some(), facet.kind() == FacetKind::Face);
            }
        }
        let broken = CameraState {
            fov_y: f64::NAN,
            ..CameraState::default()
        };
        assert!(facets(&broken).is_empty() && facet_at(&broken, [0.0, 0.0]).is_none());
        assert!(triad(&broken).is_none());
    }

    /// The shapes: a face is a rounded square, an edge a capsule as wide
    /// as a corner's circle, and the gap between a face and a capsule, a
    /// capsule and a circle, is the one of the style. Seen straight on from
    /// the front, without perspective, the numbers are in the picture.
    #[test]
    fn faces_capsules_and_circles_with_one_gap_between_them() {
        let style = style(&Tokens::dark());
        let (c, g) = (style.chamfer, style.gap);
        let front = looking("front");
        let seen = facets_styled(&front, &style);
        let find = |side: [i8; 3]| seen.iter().find(|f| f.side == side).unwrap();
        let (face, capsule, circle) = (find([0, -1, 0]), find([1, -1, 0]), find([1, -1, 1]));
        // The face, the four capsules and the four circles around it.
        assert_eq!(
            seen.len(),
            9,
            "{:?}",
            seen.iter().map(|f| f.side).collect::<Vec<_>>()
        );
        let unit = {
            // One half side in the picture: the face's flat part is 2(1 - c)
            // wide less the gap.
            let xs: Vec<f64> = face.patch.iter().map(|p| p[0]).collect();
            let width = xs.iter().copied().fold(f64::MIN, f64::max)
                - xs.iter().copied().fold(f64::MAX, f64::min);
            width / (2.0 * (1.0 - c) - g)
        };
        let span = |f: &Facet, axis: usize| {
            let v: Vec<f64> = f.patch.iter().map(|p| p[axis]).collect();
            (
                v.iter().copied().fold(f64::MAX, f64::min),
                v.iter().copied().fold(f64::MIN, f64::max),
            )
        };
        // The capsule to the right of the face, seen at 45°: its width
        // across is foreshortened, its length is not.
        let r = (c * std::f64::consts::SQRT_2 - g) * 0.5;
        let (left, right) = span(capsule, 0);
        assert!(((right - left) - 2.0 * r * std::f64::consts::FRAC_1_SQRT_2 * unit).abs() < 1e-9);
        // The gap between face and capsule, along the strip: half a gap on
        // the face's side, half a gap at 45° on the capsule's.
        let face_right = span(face, 0).1;
        let gap = (left - face_right) / unit;
        let want =
            g * 0.5 + (c * std::f64::consts::SQRT_2 * 0.5 - r) * std::f64::consts::FRAC_1_SQRT_2;
        assert!((gap - want).abs() < 1e-9, "{gap} against {want}");
        // The capsule's round end and the circle's rim are a gap apart in
        // space (centre to centre two radii and the gap).
        let (top, _) = span(capsule, 1);
        assert!(top > span(circle, 1).1, "the capsule ends below the circle");
        // The face's corners are rounded: its outline has no corner of
        // the face's square.
        let k = (1.0 - c) * unit;
        assert!(
            face.patch
                .iter()
                .all(|p| p[0].hypot(p[1]) < k * std::f64::consts::SQRT_2 - 1e-3)
        );
        // Every patch is convex, and no two overlap.
        for f in &seen {
            for other in seen.iter().filter(|o| o.side != f.side) {
                for &p in &f.patch {
                    assert!(
                        inside_by(&other.patch, p).unwrap() < 0.0,
                        "{:?} in {:?}",
                        f.side,
                        other.side
                    );
                }
            }
        }
        // Odd proportions give a cube still: finite, never a broken patch.
        for odd in [f64::NAN, f64::INFINITY, -1.0, 5.0] {
            let odd = CubeStyle {
                chamfer: odd,
                gap: odd,
                face_rounding: odd,
                fade: odd,
            };
            for facet in facets_styled(&front, &odd) {
                assert!(facet.patch.iter().flatten().all(|c| c.is_finite()));
                assert!(facet.patch.len() >= 3);
            }
        }
    }

    /// A click anywhere on the cube is on a patch: on a face, a capsule or
    /// a circle it is that one, in the gap between patches the nearest,
    /// over a patch faded out (seen nearly edge-on) the nearest that is
    /// drawn; outside the silhouette nothing. Every kind is hit in the
    /// standard views and in between.
    #[test]
    fn every_face_capsule_and_circle_is_hit_and_the_gaps_are_not_dead() {
        let style = style(&Tokens::dark());
        let mut kinds = [0; 3];
        let mut views: Vec<CameraState> = crate::state::VIEWS
            .iter()
            .map(|(name, ..)| looking(name))
            .collect();
        for (yaw, pitch) in [(17.0, 33.0), (-120.0, -40.0), (75.0, 5.0), (200.0, 70.0)] {
            views.push(CameraState {
                yaw,
                pitch,
                ..CameraState::default()
            });
        }
        for camera in views {
            let seen = facets_styled(&camera, &style);
            for facet in &seen {
                let hit = facet_at_styled(&camera, &style, facet.center()).unwrap();
                assert_eq!(hit.side, facet.side, "{camera:?}");
                kinds[facet.kind() as usize] += 1;
                // Just inside its rim, too.
                let rim = facet.patch[0];
                let c = facet.center();
                let near = [rim[0] * 0.97 + c[0] * 0.03, rim[1] * 0.97 + c[1] * 0.03];
                assert_eq!(
                    facet_at_styled(&camera, &style, near).unwrap().side,
                    facet.side
                );
            }
            // Positions all over the silhouette find a patch drawn; the
            // nearest when they are in a gap.
            let outline = silhouette(&camera, &style);
            for i in -20..=20 {
                for j in -20..=20 {
                    let at = [f64::from(i) / 20.0, f64::from(j) / 20.0];
                    let found = facet_at_styled(&camera, &style, at);
                    // In the silhouette, or on a corner's circle where it
                    // reaches past the cut corner.
                    let inside = inside_by(&outline, at).unwrap() >= 0.0
                        || seen.iter().any(|f| inside_by(&f.patch, at).unwrap() >= 0.0);
                    assert_eq!(found.is_some(), inside, "{at:?}");
                    if let Some(found) = found {
                        assert!(found.toward >= style.fade);
                        let distance = |f: &Facet| (-inside_by(&f.patch, at).unwrap()).max(0.0);
                        let best = seen.iter().map(distance).fold(f64::MAX, f64::min);
                        assert!(distance(&found) <= best + 1e-12, "{at:?}");
                    }
                }
            }
            assert_eq!(facet_at_styled(&camera, &style, [3.0, 0.0]), None);
            assert_eq!(facet_at_styled(&camera, &style, [f64::NAN, 0.0]), None);
        }
        assert!(kinds.iter().all(|&n| n > 4), "{kinds:?}");
        // Straight on, the face is in the middle and its neighbours
        // around it: X to the right and Z up seen from the front.
        let front = looking("front");
        assert_eq!(facet_at(&front, [0.0, 0.0]).unwrap().label(), Some("front"));
        let middle = |side: [i8; 3]| {
            facets(&front)
                .into_iter()
                .find(|f| f.side == side)
                .unwrap()
                .center()
        };
        let (right, above, corner) = (middle([1, -1, 0]), middle([0, -1, 1]), middle([1, -1, 1]));
        assert!(right[0] > 0.45 && right[1].abs() < 1e-9, "{right:?}");
        assert!(above[1] < -0.45 && above[0].abs() < 1e-9, "{above:?}");
        assert!(corner[0] > 0.45 && corner[1] < -0.45, "{corner:?}");
        // The top face is edge-on from the front and is not drawn: a click
        // above the front face's capsule finds that capsule.
        assert!(facets(&front).iter().all(|f| f.side != [0, 0, 1]));
    }

    /// The cube is drawn without perspective, whatever the viewport's
    /// projection: the same patches for both.
    #[test]
    fn the_cube_is_the_same_in_perspective_and_orthographic() {
        for (yaw, pitch) in [(-45.0, 29.5), (10.0, -60.0), (133.0, 12.0)] {
            let [perspective, orthographic] = cameras().map(|c| CameraState { yaw, pitch, ..c });
            assert_ne!(perspective.projection, orthographic.projection);
            assert_eq!(facets(&perspective), facets(&orthographic));
        }
    }

    /// A face's name lies in its plane: on a face seen straight on it is
    /// upright and unforeshortened, on the top seen from the iso corner it
    /// is sheared as the face is, and on a face seen nearly edge-on it is
    /// left out.
    #[test]
    fn names_lie_in_their_face_and_are_hidden_edge_on() {
        let hide = f64::from(Tokens::dark().view_cube_label_hide);
        let front = looking("front");
        let face = facets(&front)
            .into_iter()
            .find(|f| f.side == [0, -1, 0])
            .unwrap();
        let frame = face.label_frame.unwrap();
        assert!(frame.center[0].abs() < 1e-12 && frame.center[1].abs() < 1e-12);
        assert!(frame.right[0] > 0.0 && frame.right[1].abs() < 1e-12);
        assert!(frame.down[1] > 0.0 && frame.down[0].abs() < 1e-12);
        assert!((frame.right[0] - frame.down[1]).abs() < 1e-12);
        assert_eq!(label_opacity(face.toward, hide), 1.0);
        // From the top, the name of the top reads with +X to the right and
        // its top towards +Y (up in the picture).
        let top = looking("top");
        let frame = facets(&top)
            .into_iter()
            .find(|f| f.side == [0, 0, 1])
            .unwrap()
            .label_frame
            .unwrap();
        assert!(frame.right[0] > 0.0 && frame.down[1] > 0.0);
        // From the iso corner, the top's name is sheared: its right and
        // down are not square in the picture.
        let iso = looking("iso");
        let frame = facets(&iso)
            .into_iter()
            .find(|f| f.side == [0, 0, 1])
            .unwrap()
            .label_frame
            .unwrap();
        let dot = frame.right[0] * frame.down[0] + frame.right[1] * frame.down[1];
        assert!(dot.abs() > 0.01, "{frame:?}");
        // Turned until the front is nearly edge-on: no name on it.
        for yaw in [-90.0 + 80.0, -90.0 + 84.0, -90.0 + 88.0] {
            let grazing = CameraState { yaw, ..front };
            let toward = seen(&grazing, &style(&Tokens::dark()))
                .into_iter()
                .find(|f| f.side == [0, -1, 0])
                .map_or(0.0, |f| f.toward);
            assert!(toward < hide, "{yaw}: {toward}");
            assert_eq!(label_opacity(toward, hide), 0.0);
        }
        assert_eq!(label_opacity(f64::NAN, hide), 0.0);
    }

    /// The triad shows the world axes the way the viewport does: from
    /// the front X points right and Z up with Y going into the screen,
    /// from the top X right and Y up, from the right Y right, and from
    /// the iso corner all three show, X to the lower right, Y to the
    /// upper right... as the named views have them.
    #[test]
    fn the_triad_points_the_world_axes_as_the_standard_views_show_them() {
        let arm = |camera: &CameraState, axis: usize| {
            triad(camera)
                .unwrap()
                .into_iter()
                .find(|a| a.axis == axis)
                .unwrap()
        };
        let near =
            |p: [f64; 2], q: [f64; 2]| (p[0] - q[0]).abs() < 1e-9 && (p[1] - q[1]).abs() < 1e-9;
        for camera in cameras() {
            let look = |view: &str| {
                let mut c = camera;
                c.set_view(view).unwrap();
                c
            };
            let front = look("front");
            assert!(near(arm(&front, 0).tip, [1.0, 0.0]));
            assert!(near(arm(&front, 2).tip, [0.0, -1.0]));
            assert!(near(arm(&front, 1).tip, [0.0, 0.0]));
            assert!((arm(&front, 1).toward + 1.0).abs() < 1e-9, "Y points away");
            let top = look("top");
            assert!(near(arm(&top, 0).tip, [1.0, 0.0]));
            assert!(near(arm(&top, 1).tip, [0.0, -1.0]));
            assert!(
                (arm(&top, 2).toward - 1.0).abs() < 1e-9,
                "Z points at the viewer"
            );
            let right = look("right");
            assert!(near(arm(&right, 1).tip, [1.0, 0.0]));
            assert!(near(arm(&right, 2).tip, [0.0, -1.0]));
            let back = look("back");
            assert!(near(arm(&back, 0).tip, [-1.0, 0.0]));
            let iso = look("iso");
            let (x, y, z) = (arm(&iso, 0), arm(&iso, 1), arm(&iso, 2));
            assert!(x.tip[0] > 0.3 && x.tip[1] > 0.1, "{x:?}");
            assert!(y.tip[0] > 0.3 && y.tip[1] < -0.1, "{y:?}");
            assert!(z.tip[0].abs() < 1e-9 && z.tip[1] < -0.5, "{z:?}");
            // Painted far to near: the order is by how much they point at
            // the viewer.
            let arms = triad(&iso).unwrap();
            assert!(arms.windows(2).all(|w| w[0].toward <= w[1].toward));
            // The same for a camera that is not at a standard view: the
            // tips are the camera's own axes.
            let turned = CameraState {
                yaw: 117.0,
                pitch: -33.0,
                ..camera
            };
            let axes = turned.to_render().axes().unwrap();
            for (k, d) in [Vec3::X, Vec3::Y, Vec3::Z].into_iter().enumerate() {
                let a = arm(&turned, k);
                assert!(near(a.tip, [d.dot(axes.right), -d.dot(axes.up)]));
                assert!((a.tip[0].hypot(a.tip[1]).powi(2) + a.toward.powi(2) - 1.0).abs() < 1e-9);
            }
        }
    }

    #[test]
    fn the_faces_are_the_standard_views_and_the_other_facets_lie_between_them() {
        for (name, yaw, pitch) in VIEWS {
            if name == "iso" {
                continue;
            }
            let side = sides().find(|&side| label_of(side) == Some(name)).unwrap();
            assert_eq!(angles(side), Some((yaw, pitch)), "{name}");
            for camera in cameras() {
                let camera = turned(&camera, side);
                assert!(looks_from(&camera, side) && !looks_from(&camera, side.map(|c| -c)));
                // The face is then in the middle of the cube, and a click
                // on it gives the other side.
                let faced = facet_at(&camera, [0.0, 0.0]).unwrap();
                assert_eq!(faced.side, side);
                assert_eq!(clicked(&camera, &faced), side.map(|c| -c));
            }
        }
        // An edge: half-way between front and right. A corner: all three.
        assert_eq!(angles([1, -1, 0]), Some((-45.0, 0.0)));
        let (yaw, pitch) = angles([0, -1, 1]).unwrap();
        assert!(yaw == -90.0 && (pitch - 45.0).abs() < 1e-12);
        let (yaw, pitch) = angles([1, -1, 1]).unwrap();
        assert!((yaw + 45.0).abs() < 1e-12 && (pitch - 35.264_389_682_754_654).abs() < 1e-9);
        for side in sides() {
            let camera = turned(
                &CameraState {
                    distance: 50.0,
                    target: [1.0, 2.0, 3.0],
                    ..CameraState::default()
                },
                side,
            );
            assert!(looks_from(&camera, side), "{side:?}");
            let eye = camera.eye() - camera.target();
            for (k, c) in [eye.x, eye.y, eye.z].into_iter().enumerate() {
                assert_eq!(c.abs() > 1e-9, side[k] != 0, "{side:?}");
                assert!(c * f64::from(side[k]) >= 0.0, "{side:?}");
            }
        }
    }

    #[test]
    fn a_turn_goes_the_short_way_and_ends_exactly() {
        let from = CameraState {
            yaw: 170.0,
            pitch: 10.0,
            distance: 100.0,
            ..CameraState::default()
        };
        let to = CameraState {
            yaw: -170.0,
            pitch: 30.0,
            distance: 50.0,
            target: [4.0, 0.0, 0.0],
            ..from
        };
        assert_eq!(between(&from, &to, 1.0), to);
        assert_eq!(between(&from, &to, 7.0), to);
        assert_eq!(between(&from, &to, f64::NAN), to);
        let half = between(&from, &to, 0.5);
        // Through 180, not back through 0.
        assert!((half.yaw.abs() - 180.0).abs() < 1e-9, "{}", half.yaw);
        assert_eq!(
            (half.pitch, half.distance, half.target[0]),
            (20.0, 75.0, 2.0)
        );
        let start = between(&from, &to, 0.0);
        assert_eq!(
            (start.yaw, start.pitch, start.distance),
            (170.0, 10.0, 100.0)
        );
    }
}
