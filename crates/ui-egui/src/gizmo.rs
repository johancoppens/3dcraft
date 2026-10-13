//! The orbit gizmo: a small picture of the world axes in a corner of the
//! viewport that turns with the camera. Its six ends are the six standard
//! views (a click on +X looks at the model from +X, and a click on the
//! end already faced gives the view from the other side), and a drag on it
//! orbits the camera as a drag in the viewport does.
//!
//! No egui here: where the ends are and which one a position is on are
//! plain numbers, so the shell paints them and tests ask for them. Nothing
//! of it is in a document or in the control protocol: the camera it sets
//! is `UiState::camera`, which `ui.set` sets too.

use crate::state::{CameraState, VIEWS};
use craft3d_math::Vec3;

/// One end of an axis of the gizmo.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GizmoEnd {
    /// 0 is X, 1 is Y, 2 is Z.
    pub axis: usize,
    /// The end the axis points to; false for the opposite one.
    pub positive: bool,
    /// Where it is drawn, from the gizmo's centre in units of its arm:
    /// x to the right, y down. Within the unit circle.
    pub offset: [f64; 2],
    /// How far towards the viewer it is, -1 to 1: an end facing the viewer
    /// is at 1, in the middle of the gizmo.
    pub toward: f64,
    /// The standard view a click on it gives: the model seen from that
    /// side.
    pub view: &'static str,
}

impl GizmoEnd {
    /// "X", "-Y", ...
    pub fn label(&self) -> &'static str {
        match (self.axis, self.positive) {
            (0, true) => "X",
            (0, false) => "-X",
            (1, true) => "Y",
            (1, false) => "-Y",
            (2, true) => "Z",
            _ => "-Z",
        }
    }

    /// The view from the other side.
    pub fn opposite_view(&self) -> &'static str {
        view_of(self.axis, !self.positive)
    }
}

/// The view from the `positive` or the other end of `axis`: the eye is on
/// that side of the model.
fn view_of(axis: usize, positive: bool) -> &'static str {
    match (axis, positive) {
        (0, true) => "right",
        (0, false) => "left",
        (1, true) => "back",
        (1, false) => "front",
        (2, true) => "top",
        _ => "bottom",
    }
}

/// The six ends as `camera` sees them, the furthest from the viewer first:
/// the order to paint them in. Nothing for a camera that looks nowhere.
pub fn ends(camera: &CameraState) -> Vec<GizmoEnd> {
    let Ok(axes) = camera.to_render().axes() else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(6);
    for (axis, direction) in [Vec3::X, Vec3::Y, Vec3::Z].into_iter().enumerate() {
        for positive in [true, false] {
            let d = if positive { direction } else { -direction };
            let offset = [d.dot(axes.right), -d.dot(axes.up)];
            let toward = -d.dot(axes.forward);
            if offset.iter().chain([&toward]).all(|c| c.is_finite()) {
                out.push(GizmoEnd {
                    axis,
                    positive,
                    offset,
                    toward,
                    view: view_of(axis, positive),
                });
            }
        }
    }
    out.sort_by(|a, b| a.toward.total_cmp(&b.toward));
    out
}

/// The end at `at` (from the gizmo's centre in units of its arm, x right
/// and y down) when the ends are discs of radius `reach` in those units:
/// the one nearest to the viewer of those the position is on.
pub fn end_at(camera: &CameraState, at: [f64; 2], reach: f64) -> Option<GizmoEnd> {
    ends(camera)
        .into_iter()
        .rev()
        .find(|end| (end.offset[0] - at[0]).hypot(end.offset[1] - at[1]) <= reach)
}

/// True when `camera` looks from the side `view` names.
pub fn faces(camera: &CameraState, view: &str) -> bool {
    VIEWS.iter().any(|&(name, yaw, pitch)| {
        name == view
            && (camera.pitch - pitch).abs() < 1e-6
            // Straight down or up, the yaw only turns the picture.
            && (pitch.abs() >= 90.0 || ((camera.yaw - yaw + 180.0).rem_euclid(360.0) - 180.0).abs() < 1e-6)
    })
}

/// What a click on `end` does to `camera`: the view from that end, or
/// from the other side when the camera faces that end already. The target
/// and the distance stay (the shell frames the model afterwards, as its
/// view buttons did). Returns the view that was set.
pub fn click(camera: &mut CameraState, end: &GizmoEnd) -> &'static str {
    let view = if faces(camera, end.view) {
        end.opposite_view()
    } else {
        end.view
    };
    // The names are the table's own.
    let _ = camera.set_view(view);
    view
}

#[cfg(test)]
mod tests {
    use super::*;

    fn looking(view: &str) -> CameraState {
        let mut camera = CameraState::default();
        camera.set_view(view).unwrap();
        camera
    }

    #[test]
    fn the_ends_turn_with_the_camera() {
        // From the front (the eye on -Y, Z up): X to the right, Z up, and
        // -Y towards the viewer in the middle.
        let front = ends(&looking("front"));
        assert_eq!(front.len(), 6);
        let of =
            |ends: &[GizmoEnd], label: &str| *ends.iter().find(|e| e.label() == label).unwrap();
        let close = |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).hypot(a[1] - b[1]) < 1e-9;
        assert!(close(of(&front, "X").offset, [1.0, 0.0]));
        assert!(close(of(&front, "-X").offset, [-1.0, 0.0]));
        assert!(close(of(&front, "Z").offset, [0.0, -1.0]));
        assert!(close(of(&front, "-Y").offset, [0.0, 0.0]));
        assert!((of(&front, "-Y").toward - 1.0).abs() < 1e-9);
        assert!((of(&front, "Y").toward + 1.0).abs() < 1e-9);
        // Painted back to front.
        assert_eq!(front.first().unwrap().label(), "Y");
        assert_eq!(front.last().unwrap().label(), "-Y");
        // From the top (the eye on +Z, the picture's up is +Y): X right, Y up.
        let top = ends(&looking("top"));
        assert!(close(of(&top, "X").offset, [1.0, 0.0]));
        assert!(close(of(&top, "Y").offset, [0.0, -1.0]));
        assert!((of(&top, "Z").toward - 1.0).abs() < 1e-9);
        // From the default corner all three positive ends face the viewer.
        for end in ends(&CameraState::default()) {
            assert_eq!(end.toward > 0.0, end.positive == (end.axis != 1), "{end:?}");
            assert!(end.offset[0].hypot(end.offset[1]) <= 1.0 + 1e-12);
        }
    }

    #[test]
    fn a_click_on_an_end_looks_from_there_and_again_from_the_other_side() {
        for (label, view, other) in [
            ("X", "right", "left"),
            ("-X", "left", "right"),
            ("Y", "back", "front"),
            ("-Y", "front", "back"),
            ("Z", "top", "bottom"),
            ("-Z", "bottom", "top"),
        ] {
            let mut camera = CameraState {
                yaw: 33.0,
                pitch: 12.0,
                distance: 77.0,
                target: [1.0, 2.0, 3.0],
                ..CameraState::default()
            };
            let end = *ends(&camera).iter().find(|e| e.label() == label).unwrap();
            assert_eq!(click(&mut camera, &end), view);
            assert!(faces(&camera, view) && !faces(&camera, other), "{label}");
            assert_eq!(
                camera,
                CameraState {
                    ..looking_from(view, &camera)
                }
            );
            // The eye is on that side of the target.
            let toward_eye = camera.eye() - camera.target();
            let side = [toward_eye.x, toward_eye.y, toward_eye.z][end.axis];
            assert!((side.abs() - 77.0).abs() < 1e-9 && (side > 0.0) == end.positive);
            // The end now faces the viewer, in the middle of the gizmo:
            // a click there is a click on it, and gives the other side.
            let faced = end_at(&camera, [0.0, 0.0], 0.2).unwrap();
            assert_eq!(faced.label(), label);
            assert_eq!(click(&mut camera, &faced), other);
            assert!(faces(&camera, other));
            assert_eq!((camera.distance, camera.target), (77.0, [1.0, 2.0, 3.0]));
        }
    }

    fn looking_from(view: &str, like: &CameraState) -> CameraState {
        let mut camera = *like;
        camera.set_view(view).unwrap();
        camera
    }

    #[test]
    fn a_position_is_on_the_end_nearest_to_the_viewer_or_on_none() {
        let camera = CameraState::default();
        for end in ends(&camera) {
            let found = end_at(&camera, end.offset, 0.2).unwrap();
            // Two ends can overlap; the one in front is found.
            assert!(found.toward >= end.toward);
        }
        assert_eq!(end_at(&camera, [0.0, 0.0], 0.05), None);
        assert_eq!(end_at(&camera, [3.0, 3.0], 0.2), None);
        for bad in [f64::NAN, f64::INFINITY] {
            assert_eq!(end_at(&camera, [bad, 0.0], 0.2), None);
            assert_eq!(
                end_at(&camera, [0.0, 0.0], bad).map(|_| ()),
                end_at(&camera, [0.0, 0.0], bad).map(|_| ())
            );
        }
        // A camera that is no camera has no gizmo.
        let broken = CameraState {
            fov_y: f64::NAN,
            ..CameraState::default()
        };
        assert!(ends(&broken).is_empty());
        assert_eq!(end_at(&broken, [0.0, 0.0], 1.0), None);
    }
}
