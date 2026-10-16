//! Camera maths: the orbit camera of the state as a renderer's camera,
//! moving it with the mouse, framing a box, and projecting model points to
//! pixels the way the CPU renderer does, so what is drawn over its image
//! lands on it.

use crate::state::{CameraState, MAX_EXTENT, MIN_DISTANCE, ProjectionKind};
use craft3d_math::{Bbox3, Vec3};
use craft3d_render::{Camera, Projection};

/// Dragging across one point turns the camera this many degrees.
pub const ORBIT_DEGREES_PER_POINT: f64 = 0.4;
/// Scrolling by one point changes the distance by this factor's power:
/// `distance · exp(-ZOOM_PER_POINT · scroll)`.
pub const ZOOM_PER_POINT: f64 = 0.002;
/// Room left around the model when framing it: the distance is this much
/// larger than the tightest fit.
pub const FIT_MARGIN: f64 = 1.05;

fn vec(v: [f64; 3]) -> Vec3 {
    Vec3::new(v[0], v[1], v[2])
}

impl CameraState {
    fn angles(&self) -> (f64, f64) {
        (self.yaw.to_radians(), self.pitch.to_radians())
    }

    /// The unit vector from the target towards the eye.
    pub fn toward_eye(&self) -> Vec3 {
        let (yaw, pitch) = self.angles();
        Vec3::new(
            pitch.cos() * yaw.cos(),
            pitch.cos() * yaw.sin(),
            pitch.sin(),
        )
    }

    /// The unit vector that points up on the screen: where the eye goes
    /// when the pitch grows. It is defined for a view straight down too,
    /// where "the world's Z" would not be.
    pub fn up(&self) -> Vec3 {
        let (yaw, pitch) = self.angles();
        Vec3::new(
            -pitch.sin() * yaw.cos(),
            -pitch.sin() * yaw.sin(),
            pitch.cos(),
        )
    }

    /// The unit vector that points right on the screen. It has no pitch in
    /// it: turning about the world Z keeps the horizon level.
    pub fn right(&self) -> Vec3 {
        let (yaw, _) = self.angles();
        Vec3::new(-yaw.sin(), yaw.cos(), 0.0)
    }

    pub fn target(&self) -> Vec3 {
        vec(self.target)
    }

    pub fn eye(&self) -> Vec3 {
        self.target() + self.toward_eye() * self.distance
    }

    /// Half the height of what is visible at the target's depth, per unit
    /// of distance.
    fn half_tan(&self) -> f64 {
        (self.fov_y.to_radians() * 0.5).tan()
    }

    /// The camera a renderer takes.
    pub fn to_render(&self) -> Camera {
        Camera {
            eye: self.eye(),
            target: self.target(),
            up: self.up(),
            projection: match self.projection {
                ProjectionKind::Perspective => Projection::Perspective {
                    fov_y_deg: self.fov_y,
                },
                ProjectionKind::Orthographic => Projection::Orthographic {
                    half_height: self.distance * self.half_tan(),
                },
            },
        }
    }

    fn set_target(&mut self, target: Vec3) {
        if target.is_finite() {
            self.target = [
                target.x.clamp(-MAX_EXTENT, MAX_EXTENT),
                target.y.clamp(-MAX_EXTENT, MAX_EXTENT),
                target.z.clamp(-MAX_EXTENT, MAX_EXTENT),
            ];
        }
    }

    /// Turns the camera about the target, as a drag of (`dx`, `dy`) points
    /// does: the model follows the pointer. Dragging right turns the eye to
    /// the left of the model (the yaw shrinks), dragging down lifts it (the
    /// pitch grows, up to looking straight down). The distance stays.
    pub fn orbit(&mut self, dx: f64, dy: f64) {
        if !(dx.is_finite() && dy.is_finite()) {
            return;
        }
        let yaw = self.yaw - dx * ORBIT_DEGREES_PER_POINT;
        let wrapped = yaw.rem_euclid(360.0);
        self.yaw = if wrapped > 180.0 {
            wrapped - 360.0
        } else {
            wrapped
        };
        self.pitch = (self.pitch + dy * ORBIT_DEGREES_PER_POINT).clamp(-90.0, 90.0);
    }

    /// Slides the camera sideways, as a drag of (`dx`, `dy`) points in a
    /// viewport `viewport_height` points high does: the point of the model
    /// under the pointer at the target's depth stays under it.
    pub fn pan(&mut self, dx: f64, dy: f64, viewport_height: f64) {
        if !(dx.is_finite() && dy.is_finite() && viewport_height.is_finite())
            || viewport_height <= 0.0
        {
            return;
        }
        // Model units per point at the target's depth.
        let scale = 2.0 * self.distance * self.half_tan() / viewport_height;
        let target = self.target() - self.right() * (dx * scale) + self.up() * (dy * scale);
        self.set_target(target);
        self.auto_fit = false;
    }

    /// Moves the eye towards the target (`scroll` positive: the wheel
    /// turned away from the user) or back.
    pub fn zoom(&mut self, scroll: f64) {
        if !scroll.is_finite() || scroll == 0.0 {
            return;
        }
        let factor = (-ZOOM_PER_POINT * scroll).clamp(-20.0, 20.0).exp();
        self.distance = (self.distance * factor).clamp(MIN_DISTANCE, MAX_EXTENT);
        self.auto_fit = false;
    }

    /// Zooms as [`Self::zoom`] does, but about `point` instead of the
    /// target: the whole camera (eye and target) is scaled about `point`
    /// by the factor the distance changes by, so `point` stays on the
    /// same pixel, in perspective (the direction from the eye to it and
    /// the field of view do not change) and orthographic (its offset from
    /// the eye and the visible height scale alike). The eye moves along
    /// the line towards `point` and never reaches or passes it; the
    /// distance keeps the limits of [`Self::zoom`]. A point that is not
    /// finite zooms about the target.
    pub fn zoom_about(&mut self, scroll: f64, point: Vec3) {
        if !scroll.is_finite() || scroll == 0.0 {
            return;
        }
        if !point.is_finite() {
            self.zoom(scroll);
            return;
        }
        let before = self.distance;
        self.zoom(scroll);
        if !(before > 0.0 && before.is_finite()) {
            return;
        }
        // The factor actually applied, after the limits.
        let factor = self.distance / before;
        if !factor.is_finite() {
            return;
        }
        self.set_target(point + (self.target() - point) * factor);
    }

    /// The point of the plane through the target, square to the view,
    /// that shows at pixel (`x`, `y`) of a viewport `width` × `height`
    /// pixels: what a zoom at that pixel keeps in place when there is no
    /// model under it. `None` for a camera that looks nowhere or a pixel
    /// that is not a number.
    pub fn target_plane_point(&self, x: f64, y: f64, width: f64, height: f64) -> Option<Vec3> {
        if !(x.is_finite() && y.is_finite()) {
            return None;
        }
        let projector = Projector::new(&self.to_render(), width, height)?;
        let (start, direction) = projector.ray(x, y);
        // The direction's step along the view is 1, and the target is
        // `distance` beyond the eye plane: for a perspective camera from
        // the eye, for an orthographic one from the start on the eye plane.
        let point = start + direction * self.distance;
        point.is_finite().then_some(point)
    }

    /// Frames `bbox` from the direction the camera already looks from: the
    /// target becomes the centre of the box and the distance the smallest
    /// at which every corner is in view, times [`FIT_MARGIN`]. `aspect` is
    /// the viewport's width over its height.
    ///
    /// An empty box changes nothing; a box that is a single point only
    /// moves the target.
    pub fn fit(&mut self, bbox: &Bbox3, aspect: f64) {
        let (Some(center), Some((lo, hi))) = (bbox.center(), bbox.corners()) else {
            return;
        };
        if !(center.is_finite() && lo.is_finite() && hi.is_finite()) {
            return;
        }
        self.set_target(center);
        let half_y = self.half_tan();
        let half_x = half_y * aspect;
        if !(half_y.is_finite() && half_y > 0.0 && half_x.is_finite() && half_x > 0.0) {
            return;
        }
        let (dir, right, up) = (self.toward_eye(), self.right(), self.up());
        // A corner at offset `rel` from the centre is `along = rel·dir`
        // nearer to the eye than the target, `|rel·right|` off to the side
        // and `|rel·up|` up or down.
        let mut distance = 0.0_f64;
        for x in [lo.x, hi.x] {
            for y in [lo.y, hi.y] {
                for z in [lo.z, hi.z] {
                    let rel = Vec3::new(x, y, z) - center;
                    let along = rel.dot(dir);
                    let (side, lift) = (rel.dot(right).abs(), rel.dot(up).abs());
                    let needed = match self.projection {
                        // In view when the offsets fit the frustum at the
                        // corner's own depth, `distance - along`.
                        ProjectionKind::Perspective => {
                            (along + side / half_x).max(along + lift / half_y)
                        }
                        // In view when the offsets fit the slab, and the
                        // eye is in front of the corner.
                        ProjectionKind::Orthographic => {
                            (side / half_x).max(lift / half_y).max(along)
                        }
                    };
                    distance = distance.max(needed);
                }
            }
        }
        if distance.is_finite() && distance > 0.0 {
            self.distance = (distance * FIT_MARGIN).clamp(MIN_DISTANCE, MAX_EXTENT);
        }
    }
}

/// Where a model point is in an image: pixel coordinates from the top-left
/// corner (x right, y down), and its depth along the view direction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Projected {
    pub x: f64,
    pub y: f64,
    pub depth: f64,
}

/// Projects model points into an image of a given size exactly as
/// `craft3d_render::render` does with the same camera and size.
#[derive(Clone, Copy, Debug)]
pub struct Projector {
    eye: Vec3,
    right: Vec3,
    up: Vec3,
    forward: Vec3,
    perspective: bool,
    /// tan(fov / 2) for a perspective camera, half the visible height for
    /// an orthographic one.
    half: f64,
    width: f64,
    height: f64,
}

impl Projector {
    /// `None` for a camera that looks nowhere (eye on the target, up along
    /// the view direction, a field of view outside (0°, 180°)) or an image
    /// without area.
    pub fn new(camera: &Camera, width: f64, height: f64) -> Option<Self> {
        let forward = (camera.target - camera.eye).normalized()?;
        // An up vector along the view direction leaves "right" to rounding:
        // that is no camera, however long the cross product happens to be.
        let sideways = forward.cross(camera.up);
        let up_length = camera.up.length();
        if !(up_length.is_finite() && sideways.length() > 1e-9 * up_length) {
            return None;
        }
        let right = sideways.normalized()?;
        let up = right.cross(forward);
        let (perspective, half) = match camera.projection {
            Projection::Perspective { fov_y_deg } => {
                if !(fov_y_deg > 0.0 && fov_y_deg < 180.0) {
                    return None;
                }
                (true, (fov_y_deg.to_radians() * 0.5).tan())
            }
            Projection::Orthographic { half_height } => (false, half_height),
        };
        let usable = half.is_finite()
            && half > 0.0
            && camera.eye.is_finite()
            && width.is_finite()
            && height.is_finite()
            && width > 0.0
            && height > 0.0;
        usable.then_some(Self {
            eye: camera.eye,
            right,
            up,
            forward,
            perspective,
            half,
            width,
            height,
        })
    }

    /// Distance of `p` in front of the eye, along the view direction.
    pub fn depth(&self, p: Vec3) -> f64 {
        (p - self.eye).dot(self.forward)
    }

    /// The nearest depth a perspective camera can draw: a point closer to
    /// the eye plane has no place on the screen.
    pub fn near(&self) -> f64 {
        if self.perspective {
            1e-9
        } else {
            f64::NEG_INFINITY
        }
    }

    /// `None` for a point at or behind the eye plane of a perspective
    /// camera, and for a point that is not finite.
    pub fn project(&self, p: Vec3) -> Option<Projected> {
        let rel = p - self.eye;
        let (cx, cy, depth) = (rel.dot(self.right), rel.dot(self.up), rel.dot(self.forward));
        let scale = if self.perspective {
            if depth.is_nan() || depth <= self.near() {
                return None;
            }
            1.0 / (depth * self.half)
        } else {
            1.0 / self.half
        };
        let aspect = self.width / self.height;
        let x = (cx * scale / aspect * 0.5 + 0.5) * self.width;
        let y = (0.5 - cy * scale * 0.5) * self.height;
        (x.is_finite() && y.is_finite() && depth.is_finite()).then_some(Projected { x, y, depth })
    }

    /// The ray through the image point (`x`, `y`) (pixel coordinates, as
    /// [`Projector::project`] gives them): its start and its direction,
    /// whose component along the view direction is 1, so that a point
    /// `start + direction * t` lies at depth `t` beyond the start's. For a
    /// perspective camera the start is the eye; for an orthographic one it
    /// is the point of the eye plane under the pixel.
    pub fn ray(&self, x: f64, y: f64) -> (Vec3, Vec3) {
        let aspect = self.width / self.height;
        let nx = (x / self.width - 0.5) * 2.0;
        let ny = (0.5 - y / self.height) * 2.0;
        let across = self.right * (nx * aspect * self.half) + self.up * (ny * self.half);
        if self.perspective {
            (self.eye, self.forward + across)
        } else {
            (self.eye + across, self.forward)
        }
    }

    /// The depth at the fraction `t` of the straight line on the image
    /// from `a` to `b`, the projections of the ends of a segment in space.
    /// Along a line on the screen of a perspective camera it is the
    /// reciprocal of the depth that changes evenly, not the depth.
    pub fn depth_between(&self, a: &Projected, b: &Projected, t: f64) -> f64 {
        if self.perspective && a.depth > 0.0 && b.depth > 0.0 {
            let inverse = 1.0 / a.depth + (1.0 / b.depth - 1.0 / a.depth) * t;
            if inverse > 0.0 {
                return 1.0 / inverse;
            }
        }
        a.depth + (b.depth - a.depth) * t
    }

    /// The part of the segment `a`–`b` in front of the eye, projected. A
    /// segment that crosses the eye plane is cut there rather than dropped.
    pub fn segment(&self, a: Vec3, b: Vec3) -> Option<(Projected, Projected)> {
        if !self.perspective {
            return Some((self.project(a)?, self.project(b)?));
        }
        // A little in front of the eye plane, relative to the segment's own
        // depth range, so the cut end projects to a finite pixel.
        let (da, db) = (self.depth(a), self.depth(b));
        if !(da.is_finite() && db.is_finite()) {
            return None;
        }
        let near = (da.max(db) * 1e-4).max(self.near() * 2.0);
        if da < near && db < near {
            return None;
        }
        let cut = |from: Vec3, d_from: f64, to: Vec3, d_to: f64| {
            if d_from >= near {
                from
            } else {
                from.lerp(to, (near - d_from) / (d_to - d_from))
            }
        };
        let (a2, b2) = (cut(a, da, b, db), cut(b, db, a, da));
        Some((self.project(a2)?, self.project(b2)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: Vec3, b: Vec3) -> bool {
        a.distance(b) <= 1e-12
    }

    fn looking(view: &str) -> CameraState {
        let mut camera = CameraState::default();
        camera.set_view(view).unwrap();
        camera
    }

    #[test]
    fn the_named_views_have_the_screen_axes_a_drawing_has() {
        // (view, towards the eye, right on the screen, up on the screen)
        let cases = [
            ("front", -Vec3::Y, Vec3::X, Vec3::Z),
            ("back", Vec3::Y, -Vec3::X, Vec3::Z),
            ("right", Vec3::X, Vec3::Y, Vec3::Z),
            ("left", -Vec3::X, -Vec3::Y, Vec3::Z),
            // From above X still runs to the right and Y away from the
            // viewer, as on a map; from below Y comes towards the viewer.
            ("top", Vec3::Z, Vec3::X, Vec3::Y),
            ("bottom", -Vec3::Z, Vec3::X, -Vec3::Y),
        ];
        for (view, toward, right, up) in cases {
            let camera = looking(view);
            assert!(near(camera.toward_eye(), toward), "{view}");
            assert!(near(camera.right(), right), "{view}");
            assert!(near(camera.up(), up), "{view}");
        }
        // Whatever the angles: three unit vectors at right angles, right
        // handed with the eye direction pointing out of the screen.
        for yaw in [-170.0, -45.0, 0.0, 33.0, 180.0] {
            for pitch in [-90.0, -30.0, 0.0, 29.5, 90.0] {
                let camera = CameraState {
                    yaw,
                    pitch,
                    ..CameraState::default()
                };
                let (d, r, u) = (camera.toward_eye(), camera.right(), camera.up());
                for v in [d, r, u] {
                    assert!((v.length() - 1.0).abs() < 1e-12);
                }
                assert!(near(r.cross(u), d), "yaw {yaw} pitch {pitch}");
                assert!(r.z.abs() < 1e-12, "the horizon is level");
            }
        }
    }

    #[test]
    fn the_render_camera_is_the_orbit_camera() {
        let state = CameraState {
            target: [10.0, 20.0, 30.0],
            distance: 50.0,
            ..looking("front")
        };
        let camera = state.to_render();
        assert!(near(camera.eye, Vec3::new(10.0, -30.0, 30.0)));
        assert!(near(camera.target, Vec3::new(10.0, 20.0, 30.0)));
        assert!(near(camera.up, Vec3::Z));
        assert_eq!(
            camera.projection,
            Projection::Perspective { fov_y_deg: 30.0 }
        );
        // The orthographic camera shows the same height at the target:
        // 50 · tan(15°) above and below it.
        let ortho = CameraState {
            projection: ProjectionKind::Orthographic,
            ..state
        };
        let Projection::Orthographic { half_height } = ortho.to_render().projection else {
            panic!("not orthographic");
        };
        assert!((half_height - 50.0 * 15.0_f64.to_radians().tan()).abs() < 1e-12);
    }

    #[test]
    fn orbiting_keeps_the_distance_and_the_target() {
        let mut camera = CameraState {
            target: [3.0, -4.0, 5.0],
            distance: 77.0,
            ..looking("front")
        };
        // 25 points to the right, 10 down: 0.4° per point.
        camera.orbit(25.0, 10.0);
        assert!((camera.yaw - (-100.0)).abs() < 1e-12, "{}", camera.yaw);
        assert!((camera.pitch - 4.0).abs() < 1e-12, "{}", camera.pitch);
        assert_eq!(camera.target, [3.0, -4.0, 5.0]);
        assert_eq!(camera.distance, 77.0);
        assert!((camera.eye().distance(camera.target()) - 77.0).abs() < 1e-9);
        assert!(camera.auto_fit, "turning does not stop the automatic fit");
        // Dragging right moves the eye to the model's left: for the front
        // view that is towards -X.
        assert!(camera.eye().x < 3.0);
        // Dragging down lifts the eye.
        assert!(camera.eye().z > 5.0);

        // The pitch stops at straight down and straight up; the yaw wraps.
        camera.orbit(0.0, 1e6);
        assert_eq!(camera.pitch, 90.0);
        camera.orbit(0.0, -1e7);
        assert_eq!(camera.pitch, -90.0);
        let mut camera = looking("left");
        camera.orbit(-25.0, 0.0);
        assert!((camera.yaw - (-170.0)).abs() < 1e-9, "{}", camera.yaw);
        for _ in 0..1000 {
            camera.orbit(123.0, 0.0);
            assert!(camera.yaw > -180.0 && camera.yaw <= 180.0);
        }
        // Numbers that are not finite move nothing.
        let before = camera;
        camera.orbit(f64::NAN, 1.0);
        camera.orbit(1.0, f64::INFINITY);
        assert_eq!(camera, before);
    }

    #[test]
    fn panning_keeps_the_point_under_the_pointer() {
        // Front view, 100 away, 30° field of view, 400 points high: the
        // viewport shows 2 · 100 · tan(15°) = 53.59 units top to bottom.
        let mut camera = CameraState {
            distance: 100.0,
            ..looking("front")
        };
        let per_point = 2.0 * 100.0 * 15.0_f64.to_radians().tan() / 400.0;
        camera.pan(40.0, 10.0, 400.0);
        // The model moved right and down with the pointer, so the camera
        // looks at a point further left (-X) and higher (+Z).
        assert!((camera.target[0] - (-40.0 * per_point)).abs() < 1e-12);
        // Not exactly 0: the cosine of -90° is 6e-17 in floating point.
        assert!(camera.target[1].abs() < 1e-12);
        assert!((camera.target[2] - 10.0 * per_point).abs() < 1e-12);
        assert_eq!(camera.distance, 100.0);
        assert!(
            !camera.auto_fit,
            "placing the camera by hand keeps it there"
        );
        // The point that was under the pointer before is under it now.
        let projector = Projector::new(&camera.to_render(), 800.0, 400.0).unwrap();
        let moved = projector.project(Vec3::ZERO).unwrap();
        assert!((moved.x - 440.0).abs() < 1e-9 && (moved.y - 210.0).abs() < 1e-9);

        let before = camera;
        camera.pan(1.0, 1.0, 0.0);
        camera.pan(f64::NAN, 1.0, 400.0);
        camera.pan(1.0, 1.0, f64::NAN);
        assert_eq!(camera, before);
        // However far, the target stays a usable number.
        camera.pan(1e300, 1e300, 1.0);
        assert!(camera.target.iter().all(|c| c.abs() <= MAX_EXTENT));
    }

    #[test]
    fn zooming_scales_the_distance() {
        let mut camera = CameraState::default();
        camera.zoom(100.0);
        assert!((camera.distance - 100.0 * (-0.2_f64).exp()).abs() < 1e-12);
        camera.zoom(-100.0);
        assert!((camera.distance - 100.0).abs() < 1e-9);
        assert!(!camera.auto_fit);
        camera.zoom(1e9);
        assert!(camera.distance >= MIN_DISTANCE);
        camera.zoom(-1e9);
        camera.zoom(-1e9);
        assert!(camera.distance <= MAX_EXTENT);
        let before = camera;
        camera.zoom(f64::NAN);
        camera.zoom(0.0);
        assert_eq!(camera, before);
    }

    /// Zooming at the pointer: the point under it before is under it
    /// after, in both projections, about a point of the model (a hit, in
    /// front of or behind the target) and about the target plane (no
    /// hit), in and out, and at the limits nothing breaks.
    #[test]
    fn zooming_at_the_pointer_keeps_the_point_under_it() {
        let (w, h) = (800.0, 500.0);
        for projection in [ProjectionKind::Perspective, ProjectionKind::Orthographic] {
            let start = CameraState {
                projection,
                distance: 200.0,
                target: [10.0, -5.0, 3.0],
                yaw: -30.0,
                pitch: 25.0,
                ..CameraState::default()
            };
            for (px, py) in [(100.0, 80.0), (400.0, 250.0), (790.0, 490.0)] {
                let plane = start.target_plane_point(px, py, w, h).unwrap();
                // On the plane through the target, square to the view.
                assert!((plane - start.target()).dot(start.toward_eye()).abs() < 1e-9);
                let projector = Projector::new(&start.to_render(), w, h).unwrap();
                let (from, dir) = projector.ray(px, py);
                // A hit nearer than the target and one beyond it.
                let hits = [plane, from + dir * 120.0, from + dir * 350.0];
                for point in hits {
                    let shown = projector.project(point).unwrap();
                    assert!((shown.x - px).abs() < 1e-6 && (shown.y - py).abs() < 1e-6);
                    for scroll in [120.0, -120.0, 40.0, 1500.0, -1500.0] {
                        let mut camera = start;
                        camera.zoom_about(scroll, point);
                        let want = start.distance * (-ZOOM_PER_POINT * scroll).exp();
                        assert!((camera.distance - want).abs() < 1e-9 * want);
                        assert!(!camera.auto_fit, "a zoom by hand stops the automatic fit");
                        let after = Projector::new(&camera.to_render(), w, h)
                            .unwrap()
                            .project(point)
                            .unwrap();
                        let context = format!("{projection:?} {px},{py} {scroll} {point:?}");
                        assert!((after.x - px).abs() < 1e-6, "{context}: {after:?}");
                        assert!((after.y - py).abs() < 1e-6, "{context}: {after:?}");
                        // The eye never reaches the point: it stays in
                        // front of it, on the same side.
                        if projection == ProjectionKind::Perspective {
                            assert!(after.depth > 0.0, "{context}");
                        }
                        // Turned as before.
                        assert_eq!((camera.yaw, camera.pitch), (start.yaw, start.pitch));
                    }
                }
            }
            // At the limits: as close as it goes and back out, never a
            // number that is not one, never through the point.
            let point = start.target_plane_point(200.0, 100.0, w, h).unwrap();
            let mut camera = start;
            for _ in 0..200 {
                camera.zoom_about(1e6, point);
            }
            assert!(camera.distance >= MIN_DISTANCE);
            assert!(camera.target.iter().all(|c| c.is_finite()));
            for _ in 0..200 {
                camera.zoom_about(-1e6, point);
            }
            assert!(camera.distance <= MAX_EXTENT && camera.distance.is_finite());
            assert!(
                camera
                    .target
                    .iter()
                    .all(|c| c.is_finite() && c.abs() <= MAX_EXTENT)
            );
            // Nothing that is not a number moves it, and a point that is
            // not one zooms about the target.
            let before = camera;
            camera.zoom_about(f64::NAN, point);
            camera.zoom_about(0.0, point);
            assert_eq!(camera, before);
            let mut plain = start;
            plain.zoom(50.0);
            let mut about = start;
            about.zoom_about(50.0, Vec3::new(f64::NAN, 0.0, 0.0));
            assert_eq!(about, plain);
            assert_eq!(start.target_plane_point(f64::NAN, 1.0, w, h), None);
            assert_eq!(start.target_plane_point(1.0, 1.0, 0.0, h), None);
        }
    }

    fn corners(lo: Vec3, hi: Vec3) -> Vec<Vec3> {
        let mut out = Vec::new();
        for x in [lo.x, hi.x] {
            for y in [lo.y, hi.y] {
                for z in [lo.z, hi.z] {
                    out.push(Vec3::new(x, y, z));
                }
            }
        }
        out
    }

    /// From the top, a box of 40 × 30 × 10 in a viewport of 800 × 400:
    /// the height decides. Its top corners are 5 above the centre and 15
    /// up the screen, so they need a distance of 5 + 15 / tan(15°); the
    /// width would need only 5 + 20 / (2 · tan(15°)).
    #[test]
    fn fit_frames_a_box_seen_from_the_top() {
        let (lo, hi) = (Vec3::new(10.0, 20.0, 0.0), Vec3::new(50.0, 50.0, 10.0));
        let bbox = Bbox3::from_points([lo, hi]);
        let mut camera = looking("top");
        camera.fit(&bbox, 2.0);
        let tan = 15.0_f64.to_radians().tan();
        let expected = (5.0 + 15.0 / tan) * 1.05;
        assert!(
            (camera.distance - expected).abs() < 1e-9,
            "{} vs {expected}",
            camera.distance
        );
        assert_eq!(camera.target, [30.0, 35.0, 5.0]);
        assert_eq!((camera.yaw, camera.pitch), (-90.0, 90.0), "the view stays");

        let projector = Projector::new(&camera.to_render(), 800.0, 400.0).unwrap();
        let mut top = f64::INFINITY;
        for corner in corners(lo, hi) {
            let p = projector.project(corner).unwrap();
            assert!((0.0..=800.0).contains(&p.x), "{corner:?} -> {p:?}");
            assert!((0.0..=400.0).contains(&p.y), "{corner:?} -> {p:?}");
            top = top.min(p.y);
        }
        // The nearest corners are at depth `expected - 5`, 15 up the
        // screen: 200 · (1 - 15 / (depth · tan 15°)) pixels from the top.
        let by_hand = 200.0 * (1.0 - 15.0 / ((expected - 5.0) * tan));
        assert!((top - by_hand).abs() < 1e-9, "{top} vs {by_hand}");
        // Framed, not lost in the distance: under 5 % of the height spare.
        assert!(top > 0.0 && top < 20.0, "{top}");
        // X runs to the right and Y up the screen, as on a map.
        let origin = projector.project(Vec3::new(10.0, 20.0, 10.0)).unwrap();
        let along_x = projector.project(Vec3::new(50.0, 20.0, 10.0)).unwrap();
        let along_y = projector.project(Vec3::new(10.0, 50.0, 10.0)).unwrap();
        assert!(along_x.x > origin.x && (along_x.y - origin.y).abs() < 1e-9);
        assert!(along_y.y < origin.y && (along_y.x - origin.x).abs() < 1e-9);
    }

    /// The same box, orthographic: the visible half height at any depth is
    /// distance · tan(15°), and must be 15.
    #[test]
    fn fit_frames_a_box_orthographically() {
        let (lo, hi) = (Vec3::new(10.0, 20.0, 0.0), Vec3::new(50.0, 50.0, 10.0));
        let mut camera = CameraState {
            projection: ProjectionKind::Orthographic,
            ..looking("top")
        };
        camera.fit(&Bbox3::from_points([lo, hi]), 2.0);
        let tan = 15.0_f64.to_radians().tan();
        assert!((camera.distance - 15.0 / tan * 1.05).abs() < 1e-9);
        let projector = Projector::new(&camera.to_render(), 800.0, 400.0).unwrap();
        for corner in corners(lo, hi) {
            let p = projector.project(corner).unwrap();
            assert!((0.0..=800.0).contains(&p.x) && (0.0..=400.0).contains(&p.y));
            assert!(p.depth > 0.0, "the eye is outside the box");
        }
        // 15 up the screen is 200 / 1.05 pixels from the middle.
        let p = projector.project(Vec3::new(30.0, 50.0, 0.0)).unwrap();
        assert!((p.y - (200.0 - 200.0 / 1.05)).abs() < 1e-9, "{p:?}");
        // A narrow viewport: the width of 40 decides. Half of it must fit
        // in half the height times the aspect ratio of 0.5.
        camera.fit(&Bbox3::from_points([lo, hi]), 0.5);
        assert!((camera.distance - 20.0 / (0.5 * tan) * 1.05).abs() < 1e-9);
    }

    #[test]
    fn fit_keeps_every_corner_in_view_from_any_direction() {
        let boxes = [
            (Vec3::new(-1.0, -2.0, -3.0), Vec3::new(4.0, 5.0, 6.0)),
            (Vec3::new(0.0, 0.0, 0.0), Vec3::new(1e-3, 2e-3, 5e-4)),
            (Vec3::new(-1e6, 2e5, 0.0), Vec3::new(1e6, 3e5, 4e5)),
            // Flat: a sketch on a plane.
            (Vec3::new(0.0, 0.0, 5.0), Vec3::new(40.0, 30.0, 5.0)),
        ];
        for (lo, hi) in boxes {
            for projection in [ProjectionKind::Perspective, ProjectionKind::Orthographic] {
                for (yaw, pitch) in [(-45.0, 29.5), (120.0, -60.0), (-90.0, 90.0), (0.0, 0.0)] {
                    for (w, h) in [(800.0, 400.0), (300.0, 900.0)] {
                        let mut camera = CameraState {
                            yaw,
                            pitch,
                            projection,
                            ..CameraState::default()
                        };
                        camera.fit(&Bbox3::from_points([lo, hi]), w / h);
                        // How far the corners reach from the middle of the
                        // viewport, as a fraction of its half size, and
                        // how near the nearest one is.
                        let measure = |camera: &CameraState| {
                            let projector = Projector::new(&camera.to_render(), w, h).unwrap();
                            let (mut reach, mut nearest) = (0.0_f64, f64::INFINITY);
                            for corner in corners(lo, hi) {
                                let depth = projector.depth(corner);
                                nearest = nearest.min(depth);
                                if let Some(p) = projector.project(corner) {
                                    reach = reach
                                        .max((p.x / w * 2.0 - 1.0).abs())
                                        .max((p.y / h * 2.0 - 1.0).abs());
                                }
                            }
                            (reach, nearest)
                        };
                        let what = format!("{lo:?} {yaw} {pitch} {projection:?} {w}x{h}");
                        let (reach, nearest) = measure(&camera);
                        assert!(reach < 1.0, "{what}: out of view, {reach}");
                        assert!(nearest > 0.0, "{what}: the eye is in the box");
                        // Tight: without the margin a corner is exactly on
                        // the viewport's edge. (An orthographic camera may
                        // instead be as close as it can get without
                        // entering the box.)
                        let tightest = CameraState {
                            distance: camera.distance / FIT_MARGIN,
                            ..camera
                        };
                        let (reach, nearest) = measure(&tightest);
                        let on_the_edge = (reach - 1.0).abs() < 1e-6;
                        let at_the_box = projection == ProjectionKind::Orthographic
                            && nearest.abs() <= 1e-6 * camera.distance
                            && reach <= 1.0;
                        assert!(on_the_edge || at_the_box, "{what}: reach {reach}");
                    }
                }
            }
        }
    }

    #[test]
    fn fit_survives_boxes_that_cannot_be_framed() {
        let start = CameraState::default();
        let mut camera = start;
        camera.fit(&Bbox3::EMPTY, 1.5);
        assert_eq!(camera, start, "nothing to frame");
        // A single point: look at it, from where the camera was.
        camera.fit(&Bbox3::from_points([Vec3::new(7.0, 8.0, 9.0)]), 1.5);
        assert_eq!(camera.target, [7.0, 8.0, 9.0]);
        assert_eq!(camera.distance, start.distance);
        for aspect in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let mut camera = start;
            camera.fit(
                &Bbox3::from_points([Vec3::ZERO, Vec3::new(1.0, 1.0, 1.0)]),
                aspect,
            );
            assert!(camera.validated().is_ok(), "aspect {aspect}");
        }
        let mut camera = start;
        camera.fit(
            &Bbox3::from_points([Vec3::new(f64::NAN, 0.0, 0.0), Vec3::new(1.0, 1.0, 1.0)]),
            1.0,
        );
        assert!(camera.validated().is_ok());
        let mut camera = start;
        camera.fit(
            &Bbox3::from_points([Vec3::new(-1e300, 0.0, 0.0), Vec3::new(1e300, 1.0, 1.0)]),
            1.0,
        );
        assert!(camera.validated().is_ok());
        assert!(camera.distance <= MAX_EXTENT);
    }

    #[test]
    fn a_segment_through_the_eye_plane_is_cut_not_dropped() {
        // Front view from y = -10: the eye plane is y = -10.
        let camera = CameraState {
            distance: 10.0,
            ..looking("front")
        };
        let projector = Projector::new(&camera.to_render(), 200.0, 200.0).unwrap();
        // Entirely in front: both ends as they project.
        let (a, b) = projector
            .segment(Vec3::new(-1.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0))
            .unwrap();
        assert!((a.depth - 10.0).abs() < 1e-12 && (b.depth - 10.0).abs() < 1e-12);
        assert!(a.x < 100.0 && b.x > 100.0 && (a.y - 100.0).abs() < 1e-9);
        // Entirely behind: nothing.
        assert!(
            projector
                .segment(Vec3::new(0.0, -20.0, 0.0), Vec3::new(1.0, -30.0, 0.0))
                .is_none()
        );
        assert!(projector.project(Vec3::new(0.0, -20.0, 0.0)).is_none());
        // From in front to behind, along the view axis and 1 above it: the
        // visible part still starts where the segment starts, and rises
        // towards the top of the screen as it nears the eye.
        for (from, to) in [
            (Vec3::new(0.0, 0.0, 1.0), Vec3::new(0.0, -20.0, 1.0)),
            (Vec3::new(0.0, -20.0, 1.0), Vec3::new(0.0, 0.0, 1.0)),
        ] {
            let (a, b) = projector.segment(from, to).unwrap();
            let (far, close) = if a.depth > b.depth { (a, b) } else { (b, a) };
            assert!((far.depth - 10.0).abs() < 1e-12);
            assert!(close.depth > 0.0 && close.depth < 0.01);
            assert!(close.y < far.y && close.y.is_finite());
        }
        // An orthographic camera has no eye plane to cut at.
        let ortho = CameraState {
            projection: ProjectionKind::Orthographic,
            ..camera
        };
        let projector = Projector::new(&ortho.to_render(), 200.0, 200.0).unwrap();
        assert!(
            projector
                .segment(Vec3::new(0.0, 0.0, 1.0), Vec3::new(0.0, -20.0, 1.0))
                .is_some()
        );
    }

    #[test]
    fn a_camera_that_looks_nowhere_gives_no_projector() {
        let good = CameraState::default().to_render();
        assert!(Projector::new(&good, 100.0, 100.0).is_some());
        for (w, h) in [(0.0, 100.0), (100.0, 0.0), (f64::NAN, 100.0), (-5.0, 5.0)] {
            assert!(Projector::new(&good, w, h).is_none());
        }
        let on_target = Camera {
            eye: good.target,
            ..good
        };
        assert!(Projector::new(&on_target, 100.0, 100.0).is_none());
        let up_along_view = Camera {
            up: good.target - good.eye,
            ..good
        };
        assert!(Projector::new(&up_along_view, 100.0, 100.0).is_none());
        for fov_y_deg in [0.0, 180.0, -3.0, f64::NAN] {
            let camera = Camera {
                projection: Projection::Perspective { fov_y_deg },
                ..good
            };
            assert!(Projector::new(&camera, 100.0, 100.0).is_none());
        }
        let projector = Projector::new(&good, 100.0, 100.0).unwrap();
        assert!(projector.project(Vec3::new(f64::NAN, 0.0, 0.0)).is_none());
    }
}
