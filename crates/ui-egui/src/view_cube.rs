//! The view cube: a small cut gem in a corner of the viewport that turns
//! with the camera. It is a cube with its edges and corners cut off, so it
//! has 26 facets: six faces (top, front, right, ...), twelve facets where
//! an edge was and eight where a corner was. They are drawn as separate
//! patches with a thin gap between them ([`facets_gapped`]). A click on a
//! facet turns the camera to look at the model from that side: a face
//! gives a standard view, an edge facet the view half-way between two of
//! them, a corner facet the view from a corner. A click on the facet
//! already faced gives the view from the other side, and a drag on the
//! cube orbits the camera as a drag in the viewport does. The gem is drawn
//! the way the viewport is: with a perspective camera its far side is
//! smaller, with an orthographic one it is not. Above it a small triad
//! ([`triad`]) shows where the world's X, Y and Z point.
//!
//! No egui here: where the facets are and which one a position is on are
//! plain numbers, so the shell paints them and tests ask for them. Nothing
//! of it is in a document or in the control protocol: the camera it sets
//! is `UiState::camera`, which `ui.set` sets too.

use crate::state::{CameraState, ProjectionKind};
use craft3d_math::Vec3;
use craft3d_render::ViewAxes;

/// How far along a face of the cube its flat part reaches, of 1: the rest
/// is cut off into the facets of the edges and the corners.
pub const CUT: f64 = 0.6;

/// How far the eye of a perspective picture of the cube is from its
/// centre, in radii of the cube: near, so that the perspective shows.
pub const EYE: f64 = 3.2;

/// What a facet was on the cube.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FacetKind {
    Face,
    Edge,
    Corner,
}

/// One facet of the view cube as a camera sees it.
#[derive(Clone, Debug, PartialEq)]
pub struct Facet {
    /// The side of the model it stands for: -1, 0 or 1 along X, Y and Z.
    pub side: [i8; 3],
    /// Its outline in the picture of the cube, from the cube's centre: x
    /// to the right, y down, in units of the picture's radius (every
    /// corner is within the unit circle). The facets' outlines tile the
    /// picture without gaps: a position is on the facet whose outline it
    /// is in ([`facet_at`]).
    pub outline: Vec<[f64; 2]>,
    /// The patch that is painted: the outline with every side moved
    /// inwards by half the gap, on the cube, so that two neighbouring
    /// patches are a gap apart. The outline itself without a gap.
    pub patch: Vec<[f64; 2]>,
    /// How much it faces the viewer, 0 (edge-on) to 1.
    pub toward: f64,
}

impl Facet {
    pub fn kind(&self) -> FacetKind {
        kind_of(self.side)
    }

    /// The name of a face, as the standard views are called; nothing for
    /// the facet of an edge or a corner.
    pub fn label(&self) -> Option<&'static str> {
        label_of(self.side)
    }

    /// The axis a face is across (0 X, 1 Y, 2 Z), for its colour.
    pub fn axis(&self) -> Option<usize> {
        (self.kind() == FacetKind::Face)
            .then(|| self.side.iter().position(|&c| c != 0))
            .flatten()
    }

    /// The middle of its outline.
    pub fn center(&self) -> [f64; 2] {
        let n = self.outline.len().max(1) as f64;
        let sum = self
            .outline
            .iter()
            .fold([0.0, 0.0], |acc, p| [acc[0] + p[0], acc[1] + p[1]]);
        [sum[0] / n, sum[1] / n]
    }

    /// The room a label in the middle of the patch has: the lengths of
    /// the horizontal and the vertical chord of the patch through its
    /// middle, in the units of the outline. A face seen nearly edge-on has
    /// little room one way or the other, and its label is made smaller or
    /// left out.
    pub fn room(&self) -> [f64; 2] {
        let c = self.center();
        [chord(&self.patch, c, 0), chord(&self.patch, c, 1)]
    }
}

/// The length of the chord of the convex polygon `outline` through `at`
/// along axis `along` (0: horizontal, 1: vertical).
fn chord(outline: &[[f64; 2]], at: [f64; 2], along: usize) -> f64 {
    let along = along.min(1);
    let across = 1 - along;
    let (Some(&level), n) = (at.get(across), outline.len()) else {
        return 0.0;
    };
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    for k in 0..n {
        let (Some(a), Some(b)) = (outline.get(k), outline.get((k + 1) % n)) else {
            continue;
        };
        let (Some(&a_across), Some(&b_across), Some(&a_along), Some(&b_along)) =
            (a.get(across), b.get(across), a.get(along), b.get(along))
        else {
            continue;
        };
        // Where the side crosses the line through `at`, if it does.
        let span = b_across - a_across;
        if span == 0.0 || !span.is_finite() {
            continue;
        }
        let t = (level - a_across) / span;
        if (0.0..=1.0).contains(&t) {
            let x = a_along + (b_along - a_along) * t;
            lo = lo.min(x);
            hi = hi.max(x);
        }
    }
    if hi > lo { hi - lo } else { 0.0 }
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

/// The corners of the facet for `side` in space, on a cube from -1 to 1.
fn corners(side: [i8; 3]) -> Vec<Vec3> {
    let s = side.map(f64::from);
    let free: Vec<usize> = (0..3).filter(|&k| side.get(k) == Some(&0)).collect();
    let fixed: Vec<usize> = (0..3).filter(|&k| side.get(k) != Some(&0)).collect();
    let point = |values: [(usize, f64); 3]| {
        let mut c = [0.0; 3];
        for (axis, value) in values {
            if let Some(slot) = c.get_mut(axis) {
                *slot = value;
            }
        }
        let [x, y, z] = c;
        Vec3::new(x, y, z)
    };
    let of = |axis: usize| s.get(axis).copied().unwrap_or(0.0);
    match (fixed.as_slice(), free.as_slice()) {
        // A face: the square left of it.
        (&[a], &[b, c]) => [(1.0, 1.0), (-1.0, 1.0), (-1.0, -1.0), (1.0, -1.0)]
            .into_iter()
            .map(|(u, v)| point([(a, of(a)), (b, u * CUT), (c, v * CUT)]))
            .collect(),
        // Where an edge was: a strip between two faces.
        (&[a, b], &[f]) => [
            (1.0, CUT, 1.0),
            (CUT, 1.0, 1.0),
            (CUT, 1.0, -1.0),
            (1.0, CUT, -1.0),
        ]
        .into_iter()
        .map(|(u, v, w)| point([(a, of(a) * u), (b, of(b) * v), (f, w * CUT)]))
        .collect(),
        // Where a corner was: a triangle between three faces.
        _ => [(1.0, CUT, CUT), (CUT, 1.0, CUT), (CUT, CUT, 1.0)]
            .into_iter()
            .map(|(u, v, w)| point([(0, of(0) * u), (1, of(1) * v), (2, of(2) * w)]))
            .collect(),
    }
}

/// `points`, the corners of a flat convex facet with outward `normal`,
/// with every side moved inwards by `by` within the facet's plane. A
/// facet too small for that, or numbers that are not, keep their corners.
fn inset(points: &[Vec3], normal: Vec3, by: f64) -> Vec<Vec3> {
    let n = points.len();
    if !(by > 0.0 && by.is_finite()) || n < 3 {
        return points.to_vec();
    }
    let middle = points.iter().fold(Vec3::ZERO, |acc, &p| acc + p) * (1.0 / n as f64);
    let moved: Option<Vec<Vec3>> = (0..n)
        .map(|k| {
            let prev = *points.get((k + n - 1) % n)?;
            let here = *points.get(k)?;
            let next = *points.get((k + 1) % n)?;
            // The inward normals of the two sides at this corner, in the
            // facet's plane.
            let inward = |a: Vec3, b: Vec3| {
                let side = (b - a).normalized()?;
                let m = normal.cross(side).normalized()?;
                Some(if m.dot(middle - a) < 0.0 { -m } else { m })
            };
            let (m1, m2) = (inward(prev, here)?, inward(here, next)?);
            let cos = m1.dot(m2);
            // The corner moves along the bisector, by / cos(half the angle).
            (cos > -0.9).then(|| here + (m1 + m2) * (by / (1.0 + cos)))
        })
        .collect();
    match moved {
        // Still the same way round: the inset did not fold the facet over.
        Some(moved)
            if moved
                .iter()
                .zip(points)
                .all(|(m, p)| (*m - middle).dot(*p - middle) > 0.0) =>
        {
            moved
        }
        _ => points.to_vec(),
    }
}

/// Every side a facet stands for: 26 of them.
pub fn sides() -> impl Iterator<Item = [i8; 3]> {
    (-1..=1_i8)
        .flat_map(|x| (-1..=1_i8).flat_map(move |y| (-1..=1_i8).map(move |z| [x, y, z])))
        .filter(|side| *side != [0, 0, 0])
}

fn direction(side: [i8; 3]) -> Option<Vec3> {
    let [x, y, z] = side.map(f64::from);
    Vec3::new(x, y, z).normalized()
}

/// The furthest corner of the cut cube from its centre.
fn radius() -> f64 {
    (1.0 + 2.0 * CUT * CUT).sqrt()
}

/// The picture of the cube: where a point of it lands, in units of the
/// picture's radius, the way the viewport's camera projects.
struct Picture {
    axes: ViewAxes,
    perspective: bool,
}

impl Picture {
    fn new(camera: &CameraState) -> Option<Self> {
        Some(Self {
            axes: camera.to_render().axes().ok()?,
            perspective: camera.projection == ProjectionKind::Perspective,
        })
    }

    /// The eye of the cube's own picture, in the cube's units.
    fn eye(&self) -> Vec3 {
        -self.axes.forward * (EYE * radius())
    }

    fn place(&self, p: Vec3) -> [f64; 2] {
        let r = radius();
        let (x, y) = (p.dot(self.axes.right), -p.dot(self.axes.up));
        if self.perspective {
            // Seen from EYE radii away: what is nearer is larger. The
            // sphere around the cube then reaches EYE / sqrt(EYE² - 1)
            // radii from the middle of the picture, which is the unit.
            let depth = EYE * r + p.dot(self.axes.forward);
            let reach = EYE / (EYE * EYE - 1.0).sqrt();
            let scale = EYE / (depth * reach);
            [x * scale, y * scale]
        } else {
            [x / r, y / r]
        }
    }

    /// How much a facet with `normal` through `on` faces the viewer.
    fn toward(&self, normal: Vec3, on: Vec3) -> f64 {
        if self.perspective {
            (self.eye() - on)
                .normalized()
                .map_or(0.0, |to_eye| normal.dot(to_eye))
        } else {
            -normal.dot(self.axes.forward)
        }
    }
}

/// The facets `camera` sees (the ones that face it), the least facing
/// first, without gaps between their patches. Nothing for a camera that
/// looks nowhere.
pub fn facets(camera: &CameraState) -> Vec<Facet> {
    facets_gapped(camera, 0.0)
}

/// [`facets`] with a gap of `gap` between neighbouring patches, in units
/// of the cube's half side (the theme's `view_cube_gap`): it scales with
/// the cube, and is foreshortened with the facets.
pub fn facets_gapped(camera: &CameraState, gap: f64) -> Vec<Facet> {
    let half = if gap.is_finite() {
        (gap * 0.5).clamp(0.0, 0.1)
    } else {
        0.0
    };
    let Some(picture) = Picture::new(camera) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for side in sides() {
        let (Some(normal), points) = (direction(side), corners(side)) else {
            continue;
        };
        let Some(&on) = points.first() else {
            continue;
        };
        let toward = picture.toward(normal, on);
        // The cube is convex: a facet shows exactly when it faces the
        // viewer. A number that is not one shows nothing.
        if toward <= 1e-9 || toward.is_nan() {
            continue;
        }
        let patch: Vec<[f64; 2]> = inset(&points, normal, half)
            .into_iter()
            .map(|p| picture.place(p))
            .collect();
        let outline: Vec<[f64; 2]> = points.into_iter().map(|p| picture.place(p)).collect();
        if outline
            .iter()
            .chain(&patch)
            .flatten()
            .all(|c| c.is_finite())
        {
            out.push(Facet {
                side,
                outline,
                patch,
                toward,
            });
        }
    }
    out.sort_by(|a, b| a.toward.total_cmp(&b.toward).then(a.side.cmp(&b.side)));
    out
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

/// The world axes as `camera` sees them, for the triad above the cube:
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

/// The facet under `at` (in the units of [`Facet::outline`]).
pub fn facet_at(camera: &CameraState, at: [f64; 2]) -> Option<Facet> {
    if !at.iter().all(|c| c.is_finite()) {
        return None;
    }
    facets(camera).into_iter().find(|facet| {
        // Inside a convex outline: on the same side of every edge.
        let n = facet.outline.len();
        let mut sign = 0.0_f64;
        (0..n).all(|k| {
            let (Some(a), Some(b)) = (facet.outline.get(k), facet.outline.get((k + 1) % n)) else {
                return false;
            };
            let cross = (b[0] - a[0]) * (at[1] - a[1]) - (b[1] - a[1]) * (at[0] - a[0]);
            if cross == 0.0 {
                return true;
            }
            let same = sign == 0.0 || (sign > 0.0) == (cross > 0.0);
            sign = cross;
            same
        })
    })
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
    use crate::state::VIEWS;

    fn cameras() -> [CameraState; 2] {
        [
            CameraState::default(),
            CameraState {
                projection: ProjectionKind::Orthographic,
                ..CameraState::default()
            },
        ]
    }

    #[test]
    fn the_cube_has_26_facets_and_shows_the_ones_that_face_the_camera() {
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
            // From the default corner: three faces, and the facets between
            // and around them. All within the unit circle.
            let seen = facets(&camera);
            let faces: Vec<_> = seen.iter().filter_map(Facet::label).collect();
            assert_eq!(faces.len(), 3, "{faces:?}");
            for name in ["top", "front", "right"] {
                assert!(faces.contains(&name), "{faces:?}");
            }
            assert!(seen.len() >= 7, "{}", seen.len());
            for facet in &seen {
                let corners = if facet.kind() == FacetKind::Corner {
                    3
                } else {
                    4
                };
                assert_eq!(facet.outline.len(), corners);
                assert!(facet.outline.iter().all(|p| p[0].hypot(p[1]) <= 1.0 + 1e-9));
                assert!(facet.toward > 0.0 && facet.toward <= 1.0 + 1e-12);
                // The facets of a convex solid do not overlap in a picture:
                // the middle of each is on that facet and on no other.
                assert_eq!(facet_at(&camera, facet.center()).unwrap().side, facet.side);
            }
            // Straight on, one face is in the middle with its neighbours
            // around it: X to the right and Z up seen from the front.
            let mut front = camera;
            front.set_view("front").unwrap();
            assert_eq!(facet_at(&front, [0.0, 0.0]).unwrap().label(), Some("front"));
            let middle = |side: [i8; 3]| {
                facets(&front)
                    .into_iter()
                    .find(|f| f.side == side)
                    .unwrap()
                    .center()
            };
            let (right, above, corner) =
                (middle([1, -1, 0]), middle([0, -1, 1]), middle([1, -1, 1]));
            assert!(right[0] > 0.45 && right[1].abs() < 1e-9, "{right:?}");
            assert!(above[1] < -0.45 && above[0].abs() < 1e-9, "{above:?}");
            assert!(corner[0] > 0.45 && corner[1] < -0.45, "{corner:?}");
            for at in [right, above, corner] {
                assert!(facet_at(&front, at).is_some());
            }
            assert_eq!(facet_at(&front, [3.0, 0.0]), None);
            assert_eq!(facet_at(&front, [f64::NAN, 0.0]), None);
            // Without a gap the patch is the outline.
            for facet in facets(&front) {
                assert_eq!(facet.patch, facet.outline);
            }
        }
        // In perspective the face in front is larger than the same face in
        // an orthographic picture is, relative to the cube's outline: the
        // cube shows which camera the viewport has.
        let width = |camera: &CameraState| {
            let mut front = *camera;
            front.set_view("front").unwrap();
            let face = facet_at(&front, [0.0, 0.0]).unwrap();
            let xs: Vec<f64> = face.outline.iter().map(|p| p[0]).collect();
            xs.iter().copied().fold(f64::MIN, f64::max)
                - xs.iter().copied().fold(f64::MAX, f64::min)
        };
        let [perspective, orthographic] = cameras();
        assert!(width(&perspective) > width(&orthographic) * 1.15);
        let broken = CameraState {
            fov_y: f64::NAN,
            ..CameraState::default()
        };
        assert!(facets(&broken).is_empty() && facet_at(&broken, [0.0, 0.0]).is_none());
        assert!(triad(&broken).is_none() && facets_gapped(&broken, 0.05).is_empty());
    }

    /// Twice the distance from `at` to the nearest side of the convex
    /// `outline`, negative outside.
    fn inside_by(outline: &[[f64; 2]], at: [f64; 2]) -> f64 {
        let n = outline.len();
        let area: f64 = (0..n)
            .map(|k| {
                let (a, b) = (outline[k], outline[(k + 1) % n]);
                a[0] * b[1] - b[0] * a[1]
            })
            .sum();
        (0..n)
            .map(|k| {
                let (a, b) = (outline[k], outline[(k + 1) % n]);
                let cross = (b[0] - a[0]) * (at[1] - a[1]) - (b[1] - a[1]) * (at[0] - a[0]);
                cross.signum() * area.signum() * cross.abs() / (b[0] - a[0]).hypot(b[1] - a[1])
            })
            .fold(f64::INFINITY, f64::min)
    }

    /// The patches are the facets shrunk by half the gap on every side:
    /// each is inside its own outline, no two overlap, the gap between two
    /// neighbours is the gap asked for (seen straight on), and the
    /// position in a gap still belongs to a facet, so nothing between the
    /// patches is dead to a click.
    #[test]
    fn the_patches_have_a_gap_between_them_and_the_gaps_are_not_dead() {
        let gap = 0.06;
        for camera in cameras() {
            for view in ["iso", "front", "top"] {
                let mut camera = camera;
                camera.set_view(view).unwrap();
                let seen = facets_gapped(&camera, gap);
                let plain = facets(&camera);
                assert_eq!(seen.len(), plain.len(), "{view}");
                for (facet, whole) in seen.iter().zip(&plain) {
                    assert_eq!(facet.side, whole.side);
                    assert_eq!(facet.outline, whole.outline);
                    assert_eq!(facet.patch.len(), facet.outline.len());
                    // Every corner of the patch is strictly inside the
                    // outline.
                    for &p in &facet.patch {
                        assert!(
                            inside_by(&facet.outline, p) > 1e-6,
                            "{view} {:?}",
                            facet.side
                        );
                    }
                    // And outside every other patch.
                    for other in seen.iter().filter(|o| o.side != facet.side) {
                        for &p in &facet.patch {
                            assert!(inside_by(&other.patch, p) < 0.0, "{view}");
                        }
                    }
                }
                // A position on the outline between two patches is on a
                // facet: the one whose outline holds it.
                for facet in &plain {
                    let (a, b) = (facet.outline[0], facet.outline[1]);
                    let c = facet.center();
                    let near_side = [
                        (a[0] + b[0]) * 0.5 * 0.995 + c[0] * 0.005,
                        (a[1] + b[1]) * 0.5 * 0.995 + c[1] * 0.005,
                    ];
                    if facet.toward > 0.2 {
                        assert!(facet_at(&camera, near_side).is_some(), "{view}");
                    }
                }
            }
        }
        // Straight on, in an orthographic picture, the gap between the
        // front face and the edge facet to its right is the gap asked for,
        // in the picture's units (the cube's half side is 1 / radius).
        let mut front = cameras()[1];
        front.set_view("front").unwrap();
        let seen = facets_gapped(&front, gap);
        let face = seen.iter().find(|f| f.side == [0, -1, 0]).unwrap();
        let strip = seen.iter().find(|f| f.side == [1, -1, 0]).unwrap();
        let right_of_face = face.patch.iter().map(|p| p[0]).fold(f64::MIN, f64::max);
        let left_of_strip = strip.patch.iter().map(|p| p[0]).fold(f64::MAX, f64::min);
        let unit = 1.0 / (1.0 + 2.0 * CUT * CUT).sqrt();
        // The face's side moves in by half the gap; the strip's, seen at
        // 45°, by half the gap times cos 45°.
        let want = gap * 0.5 * (1.0 + std::f64::consts::FRAC_1_SQRT_2) * unit;
        assert!(
            ((left_of_strip - right_of_face) - want).abs() < 1e-9,
            "{} against {want}",
            left_of_strip - right_of_face
        );
        // A gap that is not a number, or one too large, is no gap or the
        // largest there is; never a broken patch.
        for odd in [f64::NAN, f64::INFINITY, -1.0, 5.0] {
            for facet in facets_gapped(&front, odd) {
                assert!(facet.patch.iter().flatten().all(|c| c.is_finite()));
                for &p in &facet.patch {
                    assert!(inside_by(&facet.outline, p) >= -1e-12);
                }
            }
        }
        // The room for a label: the front face seen straight on has room
        // both ways; seen nearly edge-on it has little room across.
        let face = facet_at(&front, [0.0, 0.0]).unwrap();
        let [w, h] = face.room();
        assert!((w - h).abs() < 1e-9 && w > 0.5, "{w} {h}");
        let mut grazing = front;
        grazing.yaw += 80.0;
        let face = facets(&grazing)
            .into_iter()
            .find(|f| f.side == [0, -1, 0])
            .unwrap();
        let [w, h] = face.room();
        assert!(w < 0.2 && h > 0.5, "{w} {h}");
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
