//! An assembly's free joints in the window (docs/design/assemblies.md ›
//! *Joints in the window*): a slider per mate whose main free motion can
//! be set by hand, on the Parameters tab, and dragging a part in the
//! viewport along that motion. Both issue `joint.set`, through the queue
//! the timeline's requests go through (the last value of a joint waits,
//! earlier ones give their place), so the window never waits for a solve.

use super::Shell;
use crate::camera::Projector;
use crate::theme::color;
use craft3d_engine::{JointHandle, Scene, Session};
use craft3d_math::Vec3;
use serde_json::json;

/// The range of a slider for an angle without limits: one turn either
/// way of zero.
const TURN: f64 = std::f64::consts::PI;

/// The joints' view state.
#[derive(Clone, Debug, Default)]
pub(crate) struct Joints {
    /// The slider being dragged: its mate and the value it shows.
    pub(crate) dragging: Option<(String, f64)>,
    /// Where the last frame drew each slider, by its mate.
    pub(crate) hits: Vec<(String, egui::Rect)>,
    /// A part being dragged in the viewport: its joint as the drag began
    /// and where the drag began on the joint's plane or axis.
    pub(crate) part: Option<PartDrag>,
    /// The joint each shown body's drag moves, by its feature (`F3`), for
    /// the scene revision they were found for.
    pub(crate) handles: Option<(u64, Vec<(String, JointHandle)>)>,
}

/// A part dragged along its free motion.
#[derive(Clone, Debug)]
pub(crate) struct PartDrag {
    pub(crate) joint: JointHandle,
    /// Where the drag began: on the joint's plane, from its axis (an
    /// angle), or along its axis (a distance, in `x`).
    pub(crate) start: Vec3,
}

/// The place the ray (`from`, `direction`) gives on a joint: for an
/// angle, the point where it meets the plane through the axis' origin
/// across it, from the axis; for a distance, the point of the axis nearest
/// the ray, as its distance along the axis (in `x`). `None` where the ray
/// runs along the plane or the axis.
pub(crate) fn on_joint(joint: &JointHandle, from: Vec3, direction: Vec3) -> Option<Vec3> {
    let origin = Vec3::new(joint.origin[0], joint.origin[1], joint.origin[2]);
    let axis = Vec3::new(joint.axis[0], joint.axis[1], joint.axis[2]).normalized()?;
    let direction = direction.normalized()?;
    if joint.angle {
        let across = direction.dot(axis);
        if across.abs() < 1e-6 {
            return None;
        }
        let t = (origin - from).dot(axis) / across;
        let p = from + direction * t;
        let v = p - origin;
        let v = v - axis * v.dot(axis);
        (v.length() > 1e-12 && v.is_finite()).then_some(v)
    } else {
        // The nearest points of two lines.
        let w = from - origin;
        let b = axis.dot(direction);
        let denominator = 1.0 - b * b;
        if denominator.abs() < 1e-9 {
            return None;
        }
        let s = (axis.dot(w) - b * direction.dot(w)) / denominator;
        s.is_finite().then_some(Vec3::new(s, 0.0, 0.0))
    }
}

/// The joint's value with the drag from `start` to `now`: the angle swept
/// about the axis (within half a turn either way), or the distance moved
/// along it, added to the value the drag began at; held within its limits.
pub(crate) fn dragged_value(drag: &PartDrag, now: Vec3) -> Option<f64> {
    let joint = &drag.joint;
    let delta = if joint.angle {
        let axis = Vec3::new(joint.axis[0], joint.axis[1], joint.axis[2]).normalized()?;
        let sin = drag.start.cross(now).dot(axis);
        let cos = drag.start.dot(now);
        sin.atan2(cos)
    } else {
        now.x - drag.start.x
    };
    let value = joint.value + delta;
    let value = match joint.limits {
        Some((lo, hi)) => value.clamp(lo, hi),
        None => value,
    };
    value.is_finite().then_some(value)
}

/// `value` written for `joint.set`: an angle in degrees, a distance in
/// the document's unit.
pub(crate) fn written(value: f64, angle: bool, unit: &str) -> String {
    if angle {
        format!("{} deg", value.to_degrees())
    } else {
        format!("{value} {unit}")
    }
}

impl Shell {
    /// Finds, once per scene, the joint each shown body of an assembly is
    /// dragged along (`Session::assembly_joint_of`).
    pub(super) fn sync_joint_handles(&mut self, session: &mut Session, scene: &Scene) {
        if self.outline.is_none() {
            self.joints.handles = None;
            return;
        }
        if self
            .joints
            .handles
            .as_ref()
            .is_some_and(|(revision, _)| *revision == scene.revision)
        {
            return;
        }
        let handles = scene
            .features
            .iter()
            .filter_map(|f| Some((f.id.clone(), session.assembly_joint_of(&f.id)?)))
            .collect();
        self.joints.handles = Some((scene.revision, handles));
    }

    /// An Alt+drag that began on a part of an assembly moves the part along
    /// its free motion, as `joint.set` (the crank turns, the linkage
    /// follows). True while such a drag goes on: the camera stays.
    pub(super) fn part_drag(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        rect: egui::Rect,
        scene: Option<&Scene>,
    ) -> bool {
        let [width, height] = self.size;
        let ray_at = |shell: &Shell, at: egui::Pos2| -> Option<(Vec3, Vec3)> {
            let x = f64::from(at.x - rect.left()) / f64::from(rect.width().max(f32::MIN_POSITIVE))
                * f64::from(width);
            let y = f64::from(at.y - rect.top()) / f64::from(rect.height().max(f32::MIN_POSITIVE))
                * f64::from(height);
            let projector = Projector::new(
                &shell.state.camera.to_render(),
                f64::from(width),
                f64::from(height),
            )?;
            Some(projector.ray(x, y))
        };
        if response.drag_started() {
            self.joints.part = None;
            let (alt, primary) = ui.input(|i| {
                (
                    i.modifiers.alt,
                    i.pointer.button_down(egui::PointerButton::Primary),
                )
            });
            let from = ui.input(|i| i.pointer.press_origin());
            if !(alt && primary && self.outline.is_some()) {
                return false;
            }
            let Some(at) = from.or_else(|| response.interact_pointer_pos()) else {
                return false;
            };
            let x = f64::from(at.x - rect.left()) / f64::from(rect.width().max(f32::MIN_POSITIVE))
                * f64::from(width);
            let y = f64::from(at.y - rect.top()) / f64::from(rect.height().max(f32::MIN_POSITIVE))
                * f64::from(height);
            let Ok(found) = self.pick(scene, x, y, None, None, false) else {
                return false;
            };
            let Some(feature) = found.face.as_ref().map(|f| f.feature.clone()) else {
                return false;
            };
            let Some(joint) = self.joints.handles.as_ref().and_then(|(_, handles)| {
                handles
                    .iter()
                    .find(|(f, _)| *f == feature)
                    .map(|(_, j)| j.clone())
            }) else {
                return false;
            };
            let Some(start) = ray_at(self, at).and_then(|(o, d)| on_joint(&joint, o, d)) else {
                return false;
            };
            self.joints.part = Some(PartDrag { joint, start });
            return true;
        }
        let Some(drag) = self.joints.part.clone() else {
            return false;
        };
        if !response.dragged() {
            self.joints.part = None;
            return false;
        }
        if let Some(at) = response.interact_pointer_pos()
            && let Some(now) = ray_at(self, at).and_then(|(o, d)| on_joint(&drag.joint, o, d))
            && let Some(value) = dragged_value(&drag, now)
        {
            let unit = drag.joint.unit.clone();
            self.timeline.queue(
                "joint.set",
                json!({"mate": drag.joint.mate, "value": written(value, drag.joint.angle, &unit)}),
            );
            ui.ctx().request_repaint();
        }
        true
    }

    /// True when the document shown is an assembly.
    pub(super) fn is_assembly_shown(&self) -> bool {
        self.outline.is_some()
    }

    /// True when the assembly shown has a joint that can be set by hand.
    pub(super) fn has_joint_sliders(&self) -> bool {
        self.outline
            .as_ref()
            .is_some_and(|o| o.mates.iter().any(|m| m.settable && m.value.is_some()))
    }

    /// Where the last frame drew each joint's slider, by its mate.
    pub fn joint_slider_hits(&self) -> &[(String, egui::Rect)] {
        &self.joints.hits
    }

    /// The joints of an assembly, a slider each: dragging it sets the
    /// joint (`joint.set`), the assembly follows.
    pub(super) fn joints_ui(&mut self, ui: &mut egui::Ui) {
        let tokens = self.tokens;
        let dim = color(tokens.text_dim);
        self.joints.hits.clear();
        let Some(outline) = self.outline.clone() else {
            return;
        };
        ui.label(egui::RichText::new("Joints").strong());
        let size = self
            .shown_scene
            .as_ref()
            .and_then(|s| s.bbox.corners())
            .map_or(100.0, |(lo, hi)| (hi - lo).length().max(1e-9));
        let mut released = false;
        for mate in outline.mates.iter().filter(|m| m.settable) {
            let Some((value, angle)) = mate.value else {
                continue;
            };
            let (lo, hi) = mate
                .limits
                .unwrap_or(if angle { (-TURN, TURN) } else { (-size, size) });
            let mut shown = match &self.joints.dragging {
                Some((name, v)) if *name == mate.name => *v,
                _ => value,
            };
            // A slider of degrees for an angle, of the document's unit
            // otherwise.
            let (mut on_slider, range, suffix) = if angle {
                (
                    shown.to_degrees(),
                    lo.to_degrees()..=hi.to_degrees(),
                    "°".to_string(),
                )
            } else {
                (shown, lo..=hi, format!(" {}", outline.unit))
            };
            let response = ui
                .horizontal(|ui| {
                    ui.label(&mate.name);
                    ui.add(
                        egui::Slider::new(&mut on_slider, range)
                            .suffix(suffix)
                            .clamping(egui::SliderClamping::Never),
                    )
                })
                .inner
                .on_hover_text(format!("{} · joint.set {}", mate.kind, mate.name));
            self.joints.hits.push((mate.name.clone(), response.rect));
            if response.changed() && on_slider.is_finite() {
                shown = if angle {
                    on_slider.to_radians()
                } else {
                    on_slider
                };
                self.joints.dragging = Some((mate.name.clone(), shown));
                self.timeline.queue(
                    "joint.set",
                    json!({"mate": mate.name, "value": written(shown, angle, &outline.unit)}),
                );
                ui.ctx().request_repaint();
            }
            if response.drag_stopped() || (response.changed() && !response.dragged()) {
                released = true;
            }
        }
        if released {
            self.joints.dragging = None;
        }
        if outline.mates.iter().any(|m| m.driven && !m.settable) {
            ui.add(
                egui::Label::new(
                    egui::RichText::new(
                        "Joints driven by a formula (of t) follow the timeline; the player plays them.",
                    )
                    .color(dim)
                    .small(),
                )
                .wrap(),
            );
        }
        ui.separator();
    }
}
