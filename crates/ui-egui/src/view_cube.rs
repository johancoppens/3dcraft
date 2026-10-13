//! The view cube: a small cut gem in a corner of the viewport that turns
//! with the camera. It is a cube with its edges and corners cut off, so it
//! has 26 facets: six faces (top, front, right, ...), twelve facets where
//! an edge was and eight where a corner was. A click on a facet turns the
//! camera to look at the model from that side: a face gives a standard
//! view, an edge facet the view half-way between two of them, a corner
//! facet the view from a corner. A click on the facet already faced gives
//! the view from the other side, and a drag on the cube orbits the camera
//! as a drag in the viewport does. The three axes stick out of the faces
//! they point through, in their colours. The gem is drawn the way the
//! viewport is: with a perspective camera its far side is smaller, with an
//! orthographic one it is not, and a click on the ring around it switches
//! between the two.
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
    /// corner is within the unit circle).
    pub outline: Vec<[f64; 2]>,
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
/// first. Nothing for a camera that looks nowhere.
pub fn facets(camera: &CameraState) -> Vec<Facet> {
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
        let outline: Vec<[f64; 2]> = points.into_iter().map(|p| picture.place(p)).collect();
        if outline.iter().flatten().all(|c| c.is_finite()) {
            out.push(Facet {
                side,
                outline,
                toward,
            });
        }
    }
    out.sort_by(|a, b| a.toward.total_cmp(&b.toward).then(a.side.cmp(&b.side)));
    out
}

/// The stick of axis `axis` (0 X, 1 Y, 2 Z) in the picture of the cube:
/// from the middle of its face to its end, in the units of
/// [`Facet::outline`], and whether it points towards the viewer.
pub fn axis_stick(camera: &CameraState, axis: usize) -> Option<([f64; 2], [f64; 2], bool)> {
    let picture = Picture::new(camera)?;
    let d = *[Vec3::X, Vec3::Y, Vec3::Z].get(axis)?;
    let ends = (picture.place(d), picture.place(d * (radius() * 1.12)));
    let finite = ends.0.iter().chain(&ends.1).all(|c| c.is_finite());
    finite.then_some((ends.0, ends.1, picture.toward(d, d) > 0.0))
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
            // The X stick points right, away from the middle.
            let (from, to, _) = axis_stick(&front, 0).unwrap();
            assert!(to[0] > from[0] && from[0] > 0.3 && to[1].abs() < 1e-9);
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
        assert!(axis_stick(&broken, 0).is_none() && axis_stick(&perspective, 7).is_none());
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
