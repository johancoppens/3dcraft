//! An assembly's free joints in the window (docs/design/assemblies.md ›
//! *Joints in the window*): a slider per mate whose main free motion can
//! be set by hand, on the Parameters tab, and dragging a part in the
//! viewport along that motion. Both issue `joint.set`, through the queue
//! the timeline's requests go through (the last value of a joint waits,
//! earlier ones give their place), so the window never waits for a solve;
//! every value after a drag's first is sent with `merge`, so one drag is
//! one undo step.

use super::Shell;
use crate::camera::Projector;
use crate::theme::color;
use craft3d_engine::{JointHandle, Scene, Session};
use craft3d_math::Portable;
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
    /// Where the last frame drew each entry of the mate menu.
    pub(crate) mate_hits: Vec<(&'static str, egui::Rect)>,
}

/// The kinds of mate the menu on two faces offers, as `mate.add` names
/// them, with their words.
pub(crate) const MATE_ENTRIES: [(&str, &str); 6] = [
    ("revolute", "Revolute between these"),
    ("fixed", "Fixed between these"),
    ("slider", "Slider between these"),
    ("cylindrical", "Cylindrical between these"),
    ("planar", "Planar between these"),
    ("ball", "Ball between these"),
];

/// A part dragged along its free motion.
#[derive(Clone, Debug)]
pub(crate) struct PartDrag {
    pub(crate) joint: JointHandle,
    /// Where the drag began: on the joint's plane, from its axis (an
    /// angle), or along its axis (a distance, in `x`).
    pub(crate) start: Vec3,
    /// The angle swept from `start` at the last frame, unwrapped: a drag
    /// goes on past half a turn.
    pub(crate) swept: f64,
    /// A value of this drag was sent: the next ones merge into its undo
    /// step.
    pub(crate) sent: bool,
}

/// The place the ray (`from`, `direction`) gives on a joint: for an
/// angle, the point where it meets the plane through the axis' origin
/// across it, from the axis; for a distance, the point of the axis nearest
/// the ray, as its distance along the axis (in `x`). `None` where the ray
/// runs along the plane or the axis, or (an angle) meets the plane on the
/// axis, within 10⁻⁹ of the assembly's size, where no direction is.
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
        let near = 1e-9 * joint.size.max(f64::MIN_POSITIVE);
        (v.length() > near && v.is_finite()).then_some(v)
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

/// The joint's value with the drag from `start` to `now`, and the angle
/// swept (for the next frame's `swept`): the angle swept about the axis,
/// taken within half a turn of the last frame's (so a drag goes round as
/// far as it is dragged), or the distance moved along it, times the
/// joint's `sign`, added to the value the drag began at; held within its
/// limits.
pub(crate) fn dragged_value(drag: &PartDrag, now: Vec3) -> Option<(f64, f64)> {
    let joint = &drag.joint;
    let delta = if joint.angle {
        let axis = Vec3::new(joint.axis[0], joint.axis[1], joint.axis[2]).normalized()?;
        let sin = drag.start.cross(now).dot(axis);
        let cos = drag.start.dot(now);
        let within = sin.patan2(cos);
        // The turn nearest the last frame's.
        let tau = std::f64::consts::TAU;
        let turns = ((drag.swept - within) / tau).round();
        if !turns.is_finite() {
            return None;
        }
        within + turns * tau
    } else {
        now.x - drag.start.x
    };
    let value = joint.value + joint.sign * delta;
    let value = match joint.limits {
        Some((lo, hi)) => value.clamp(lo, hi),
        None => value,
    };
    (value.is_finite() && delta.is_finite()).then_some((value, delta))
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
            self.joints.part = Some(PartDrag {
                joint,
                start,
                swept: 0.0,
                sent: false,
            });
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
            && let Some((value, swept)) = dragged_value(&drag, now)
        {
            let unit = drag.joint.unit.clone();
            self.timeline.queue(
                "joint.set",
                json!({"mate": drag.joint.mate, "value": written(value, drag.joint.angle, &unit),
                       "merge": drag.sent}),
            );
            if let Some(part) = self.joints.part.as_mut() {
                part.swept = swept;
                part.sent = true;
            }
            ui.ctx().request_repaint();
        }
        true
    }

    /// The two faces selected, as a mate names them (`Instance/face`):
    /// when the document shown is an assembly and exactly two faces of two
    /// of its own instances are selected.
    pub(crate) fn mate_pair(&self) -> Option<(String, String)> {
        let outline = self.outline.as_ref()?;
        let items = &self.select.view.items;
        let [a, b] = items.as_slice() else {
            return None;
        };
        let reference = |item: &craft3d_engine::SelectionItem| -> Option<(String, String)> {
            if item.kind != craft3d_engine::SelectionKind::Face {
                return None;
            }
            let instance = outline
                .instances
                .iter()
                .find(|i| i.own && i.features.contains(&item.feature))?;
            Some((
                instance.name.clone(),
                format!("{}/{}", instance.name, item.name),
            ))
        };
        let ((first, a), (second, b)) = (reference(a)?, reference(b)?);
        (first != second).then_some((a, b))
    }

    /// Where the last frame drew each entry of the mate menu, by its kind.
    pub fn mate_entry_hits(&self) -> &[(&'static str, egui::Rect)] {
        &self.joints.mate_hits
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

    /// The model's size: the diagonal of the shown scene's box (what a
    /// distance without limits may be set to either way).
    pub(super) fn joint_range_size(&self) -> f64 {
        self.shown_scene
            .as_ref()
            .and_then(|s| s.bbox.corners())
            .map_or(100.0, |(lo, hi)| (hi - lo).length().max(1e-9))
    }

    /// The slider of one joint, `label` before its rail: the kit's one
    /// slider in a row, in degrees for an angle (typed values as they are)
    /// and in the document's `unit` otherwise, between the mate's limits
    /// or a turn either way (a distance: `size` either way). Dragging it
    /// queues `joint.set`, every value after a drag's first with `merge`
    /// (one drag is one undo step). True when this slider's drag ended.
    pub(super) fn joint_slider(
        &mut self,
        ui: &mut egui::Ui,
        kit: &crate::kit::Kit,
        mate: &craft3d_engine::OutlineMate,
        unit: &str,
        size: f64,
        label: &str,
    ) -> bool {
        let Some((value, angle)) = mate.value else {
            return false;
        };
        let (lo, hi) = mate
            .limits
            .unwrap_or(if angle { (-TURN, TURN) } else { (-size, size) });
        let mut shown = match &self.joints.dragging {
            Some((name, v)) if *name == mate.name => *v,
            _ => value,
        };
        let (mut on_slider, range, shown_unit) = if angle {
            (shown.to_degrees(), [lo.to_degrees(), hi.to_degrees()], "°")
        } else {
            (shown, [lo, hi], unit)
        };
        let label_width = (ui.available_width() * 0.3).clamp(48.0, 120.0);
        let value_width = 72.0;
        let decimals = if angle { 1 } else { 2 };
        let slider = ui
            .push_id(("craft3d-joint", &mate.name), |ui| {
                kit.slider_with(
                    ui,
                    crate::kit::Slider::new(label, &mut on_slider, range[0]..=range[1])
                        .widths(label_width, value_width)
                        .shown_as(1.0, decimals, shown_unit)
                        // Typed values as they are (a joint's slider
                        // never clamped them); no default to go back
                        // to: a joint has no rest value of its own.
                        .typed_free(true),
                    None,
                )
            })
            .inner;
        let response = slider
            .response
            .clone()
            .on_hover_text(format!("{} · joint.set {}", mate.kind, mate.name));
        self.joints.hits.push((mate.name.clone(), response.rect));
        if slider.changed && on_slider.is_finite() {
            shown = if angle {
                on_slider.to_radians()
            } else {
                on_slider
            };
            // A drag's later values merge into its first's undo step.
            let merge = matches!(&self.joints.dragging, Some((name, _)) if *name == mate.name);
            self.joints.dragging = Some((mate.name.clone(), shown));
            self.timeline.queue(
                "joint.set",
                json!({"mate": mate.name, "value": written(shown, angle, unit),
                       "merge": merge}),
            );
            ui.ctx().request_repaint();
        }
        let mine = matches!(&self.joints.dragging, Some((name, _)) if *name == mate.name);
        mine && !slider.dragging
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
        let kit = crate::kit::Kit::panel(&tokens);
        kit.section_header(ui, "Joints");
        let size = self.joint_range_size();
        let mut released = false;
        for mate in outline.mates.iter().filter(|m| m.settable) {
            released |= self.joint_slider(ui, &kit, mate, &outline.unit, size, &mate.name);
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
                    .size(tokens.secondary_font),
                )
                .wrap(),
            );
        }
        kit.divider(ui);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use craft3d_math::Portable;

    fn joint(sign: f64) -> JointHandle {
        JointHandle {
            mate: "Drive".into(),
            angle: true,
            origin: [0.0; 3],
            axis: [0.0, 0.0, 1.0],
            value: 0.0,
            sign,
            limits: None,
            unit: "mm".into(),
            size: 100.0,
        }
    }

    fn at(degrees: f64) -> Vec3 {
        let a = degrees.to_radians();
        Vec3::new(a.pcos(), a.psin(), 0.0)
    }

    /// A drag that goes on past half a turn keeps turning: each frame's
    /// angle is taken within half a turn of the last frame's.
    #[test]
    fn a_drag_goes_on_past_half_a_turn() {
        let mut drag = PartDrag {
            joint: joint(1.0),
            start: at(0.0),
            swept: 0.0,
            sent: false,
        };
        for degrees in [60.0, 120.0, 170.0, 200.0, 260.0, 330.0, 400.0] {
            let (value, swept) = dragged_value(&drag, at(degrees)).unwrap();
            assert!(
                (value.to_degrees() - degrees).abs() < 1e-9,
                "{degrees}: {}",
                value.to_degrees()
            );
            drag.swept = swept;
        }
    }

    /// On side a of its joint the value goes the other way.
    #[test]
    fn side_a_turns_the_value_the_other_way() {
        let drag = PartDrag {
            joint: joint(-1.0),
            start: at(0.0),
            swept: 0.0,
            sent: false,
        };
        let (value, _) = dragged_value(&drag, at(30.0)).unwrap();
        assert!((value.to_degrees() + 30.0).abs() < 1e-9);
    }

    /// Near the axis is measured against the model's size: a large model's
    /// point a micrometre off the axis still has a direction.
    #[test]
    fn near_the_axis_is_relative_to_the_model() {
        let mut big = joint(1.0);
        big.size = 1e6;
        let mut small = joint(1.0);
        small.size = 1e-3;
        // A ray straight down at (x, 0).
        let down = Vec3::new(0.0, 0.0, -1.0);
        assert!(on_joint(&big, Vec3::new(1e-2, 0.0, 5.0), down).is_some());
        assert!(on_joint(&big, Vec3::new(1e-4, 0.0, 5.0), down).is_none());
        assert!(on_joint(&small, Vec3::new(1e-9, 0.0, 5.0), down).is_some());
    }
}
