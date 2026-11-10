//! An assembly's mates in the window (docs/design/assemblies.md › *Mate
//! markers*): the markers in the viewport ([`crate::mates`]), drawn while
//! `show.mates` is on; pointing at a mate's row on the Assembly tab or at
//! its marker highlights the marker and tints its two parts
//! (`sidePanel.mateHover`), the marker names its mate, and a click on a
//! row or a marker chooses the mate (`sidePanel.mate`). Under each mate
//! row whose joint can be set by hand is its slider
//! ([`Shell::joint_slider`], the Parameters tab's).

use super::Shell;
use crate::mates::{self, Marker};
use crate::state::CameraState;
use crate::theme::color;
use crate::viewport::Marks;
use craft3d_engine::Scene;
use craft3d_render::Highlight;

/// Where the pointer pointed at a mate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Pointed {
    Row,
    Marker,
}

/// The mates' view state that is not [`crate::state::UiState`].
#[derive(Clone, Debug, Default)]
pub(crate) struct MateView {
    /// What set `sidePanel.mateHover`, when the pointer did (a row or a
    /// marker) rather than `ui.set`: it goes when the pointer leaves that.
    pub(crate) hover_by: Option<Pointed>,
    /// The mate the pointer set it to.
    pub(crate) hover_name: Option<String>,
    /// Where the last frame drew each mate's row, by its name.
    pub(crate) rows: Vec<(String, egui::Rect)>,
    /// A point of each marker on screen at the last frame (the middle of
    /// its longest line there), in points, by its mate.
    pub(crate) markers: Vec<(String, egui::Pos2)>,
    /// What the tooltip over the markers says, a line per mate under the
    /// pointer; empty without one.
    pub(crate) tip: Vec<String>,
    /// The document the chosen and pointed-at mates are of.
    pub(crate) document: Option<u64>,
}

impl Shell {
    /// The mates whose markers are drawn highlighted: the one chosen and
    /// the one pointed at.
    fn hot_mates(&self) -> [Option<&str>; 2] {
        [
            self.state.side_panel.mate.as_deref(),
            self.state.side_panel.mate_hover.as_deref(),
        ]
    }

    /// The markers drawn for `camera` on an image of `size` pixels at
    /// `scale` pixels per point: none for a part, while `show.mates` is
    /// off, or when the camera looks nowhere.
    fn mate_markers_for(&self, camera: &CameraState, size: [u32; 2], scale: f32) -> Vec<Marker> {
        let Some(outline) = self.outline.as_ref().filter(|_| self.state.show.mates) else {
            return Vec::new();
        };
        let [width, height] = size;
        let Some(length) = mates::world_size(camera, width, height, scale, self.tokens.mate_size)
        else {
            return Vec::new();
        };
        mates::markers(&outline.mates, length)
    }

    /// The marks of the markers for `camera` on an image of `size` pixels
    /// at `scale` pixels per point: the highlighted ones apart.
    pub(crate) fn mate_marks_for(&self, camera: &CameraState, size: [u32; 2], scale: f32) -> Marks {
        let hot = self.hot_mates();
        let mut marks = Marks::default();
        for marker in self.mate_markers_for(camera, size, scale) {
            if hot.contains(&Some(marker.name.as_str())) {
                marks.mates_hot.extend(marker.lines);
            } else {
                marks.mates.extend(marker.lines);
            }
        }
        marks
    }

    /// The mate markers as the viewport draws them now (its camera, size
    /// and pixels per point): what both paths of the viewport draw.
    pub fn mate_marks(&self) -> Marks {
        self.mate_marks_for(&self.state.camera, self.size, self.pixels_per_point)
    }

    /// The faces of the parts of the chosen mate that are not held to the
    /// world, tinted `mate_part`: also of a mate that failed (its parts are still shown
    /// where they are, which says what it joins); the world has none.
    /// `None` when no mate is pointed at or chosen, or its parts show
    /// nothing.
    pub(super) fn mate_part_highlight(&self, scene: Option<&Scene>) -> Option<Highlight> {
        let outline = self.outline.as_ref()?;
        let scene = scene?;
        // Only the chosen mate's parts: pointing at one lights its marker
        // alone (the developer, 2026-11-06: subtle).
        let [chosen, _] = self.hot_mates();
        let name = chosen?;
        let mate = outline.mates.iter().find(|m| m.name == name)?;
        // A part that is a flexible subassembly has no bodies of its own:
        // its instances (`Sub/Inner`) are tinted.
        let of_part = |name: &str| {
            mate.parts.iter().any(|p| {
                name == p
                    || name
                        .strip_prefix(p.as_str())
                        .is_some_and(|rest| rest.starts_with('/'))
            })
        };
        let features: Vec<&String> = outline
            .instances
            .iter()
            // A part held to the world (a housing, a ring gear) is not
            // tinted: it is not what the joint moves.
            .filter(|i| of_part(&i.name) && !i.grounded)
            .flat_map(|i| &i.features)
            .collect();
        if features.is_empty() {
            return None;
        }
        let ids = scene
            .bodies
            .iter()
            .filter(|b| features.contains(&&b.feature))
            .flat_map(|b| {
                let count = u32::try_from(b.facts.face_names.len()).unwrap_or(0);
                (0..count).filter_map(move |k| b.first_face_id.checked_add(k))
            });
        let (highlight, _) = Highlight::new(self.tokens.mate_part, ids);
        Some(highlight)
    }

    /// The document shown is `document`: a mate chosen or pointed at in
    /// another one (another tab, a document opened or closed) goes.
    /// No document known (a frame drawn while a command runs elsewhere)
    /// changes nothing.
    pub(super) fn mates_follow_document(&mut self, document: Option<u64>) {
        if document.is_some() && self.mate_view.document != document {
            self.mate_view.document = document;
            self.state.side_panel.mate = None;
            self.state.side_panel.mate_hover = None;
            self.mate_view.hover_by = None;
            self.mate_view.tip.clear();
        }
    }

    /// What the tooltip over the markers under the pointer says, a line
    /// per mate (overlapping markers, such as two coaxial joints, are
    /// named together); empty when no marker is under it.
    pub fn mate_tip(&self) -> &[String] {
        &self.mate_view.tip
    }

    /// Where the last frame drew each mate's row on the Assembly tab.
    pub fn mate_row_hits(&self) -> &[(String, egui::Rect)] {
        &self.mate_view.rows
    }

    /// A point of each mate's marker on screen at the last frame, in
    /// points (where a pointer names it).
    pub fn mate_marker_hits(&self) -> &[(String, egui::Pos2)] {
        &self.mate_view.markers
    }

    /// The pointer over the viewport `rect` and the markers: hovering one
    /// names its mate and highlights it, a click chooses it. True when
    /// this frame's click was on a marker (and so is not a pick).
    pub(super) fn mate_pointer(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        rect: egui::Rect,
        scene: Option<&Scene>,
    ) -> bool {
        let [width, height] = self.size;
        let markers = self.mate_markers_for(&self.state.camera, self.size, self.pixels_per_point);
        let projector = crate::camera::Projector::new(
            &self.state.camera.to_render(),
            f64::from(width),
            f64::from(height),
        );
        let to_pixels = |at: egui::Pos2| {
            let across = |from: f32, to: f32, at: f32, pixels: u32| {
                f64::from(at - from) / f64::from((to - from).max(f32::MIN_POSITIVE))
                    * f64::from(pixels)
            };
            (
                across(rect.left(), rect.right(), at.x, width),
                across(rect.top(), rect.bottom(), at.y, height),
            )
        };
        let to_points = |x: f64, y: f64| {
            egui::pos2(
                rect.left() + (x / f64::from(width.max(1))) as f32 * rect.width(),
                rect.top() + (y / f64::from(height.max(1))) as f32 * rect.height(),
            )
        };
        self.mate_view.markers = match &projector {
            Some(projector) => markers
                .iter()
                .filter_map(|m| {
                    let (a, b) = m
                        .lines
                        .iter()
                        .filter_map(|&[a, b]| projector.segment(a, b))
                        .max_by(|(a, b), (c, d)| {
                            let l = |p: &crate::camera::Projected, q: &crate::camera::Projected| {
                                (q.x - p.x).abs() + (q.y - p.y).abs()
                            };
                            l(a, b).total_cmp(&l(c, d))
                        })?;
                    // Toward the far end of the axis, out of the body it
                    // starts on.
                    Some((
                        m.name.clone(),
                        to_points(a.x + (b.x - a.x) * 0.8, a.y + (b.y - a.y) * 0.8),
                    ))
                })
                .collect(),
            None => Vec::new(),
        };
        let reach = f64::from(self.tokens.mate_hit) * f64::from(self.pixels_per_point);
        let camera = self.state.camera;
        let section = self.section();
        let length = mates::world_size(
            &camera,
            width,
            height,
            self.pixels_per_point,
            self.tokens.mate_size,
        )
        .unwrap_or(0.0);
        // The markers under the pointer where they are in sight: a marker
        // behind the face there (drawn dimmed) leaves the pointer to the
        // face, so faces around a joint are still picked. Nearest first,
        // as near in the mates' order; those as near as the nearest (one
        // pixel) overlap and are named together.
        let under = |at: egui::Pos2| -> Vec<String> {
            let Some(projector) = projector.as_ref() else {
                return Vec::new();
            };
            let (x, y) = to_pixels(at);
            let near = mates::markers_at(&markers, projector, x, y, reach);
            if near.is_empty() {
                return Vec::new();
            }
            let front = scene
                .and_then(|scene| {
                    scene
                        .pick_cut(&camera.to_render(), width, height, x, y, section.as_ref())
                        .ok()
                })
                .and_then(|(face, cap)| face.map(|f| f.point).or(cap.map(|c| c.point)))
                .map(|p| projector.depth(p))
                .filter(|d| d.is_finite());
            let slack = 0.05 * length;
            let seen: Vec<(&str, f64)> = near
                .into_iter()
                .filter(|(_, _, depth)| front.is_none_or(|f| *depth <= f + slack))
                .map(|(name, d, _)| (name, d))
                .collect();
            let Some(&(_, best)) = seen.first() else {
                return Vec::new();
            };
            seen.into_iter()
                .filter(|(_, d)| *d <= best + 1.0)
                .map(|(name, _)| name.to_string())
                .collect()
        };
        // Not while the camera or a part is being dragged.
        let idle = !response.dragged() && !ui.input(|i| i.pointer.any_down());
        let pointed = response
            .hover_pos()
            .filter(|_| idle)
            .map(under)
            .unwrap_or_default();
        match pointed.first() {
            Some(name) => {
                if self.state.side_panel.mate_hover.as_ref() != Some(name) {
                    ui.ctx().request_repaint();
                }
                self.state.side_panel.mate_hover = Some(name.clone());
                self.mate_view.hover_by = Some(Pointed::Marker);
                self.mate_view.hover_name = Some(name.clone());
                let kind = |name: &str| {
                    self.outline
                        .as_ref()
                        .and_then(|o| o.mates.iter().find(|m| m.name == name))
                        .map_or("", |m| m.kind)
                };
                let tip: Vec<String> = pointed
                    .iter()
                    .map(|name| match kind(name) {
                        "" => name.clone(),
                        kind => format!("{name} · {kind}"),
                    })
                    .collect();
                self.mate_view.tip = tip.clone();
                response.clone().on_hover_text_at_pointer(tip.join("\n"));
            }
            None => {
                self.mate_view.tip.clear();
                self.pointer_left(Pointed::Marker);
            }
        }
        let clicked = response.clicked()
            && response
                .interact_pointer_pos()
                .and_then(|at| under(at).into_iter().next())
                .map(|name| {
                    self.state.side_panel.mate = Some(name);
                    self.state.panels.side_tab = crate::state::SideTab::Assembly;
                    ui.ctx().request_repaint();
                })
                .is_some();
        // A click elsewhere in the viewport lets the chosen mate go.
        if response.clicked() && !clicked {
            self.state.side_panel.mate = None;
        }
        clicked
    }

    /// The pointer is on no mate's row (or marker): a hover it made there
    /// goes, one set from outside or from the other stays.
    fn pointer_left(&mut self, from: Pointed) {
        if self.mate_view.hover_by != Some(from) {
            return;
        }
        // Only while the hover is still the one the pointer set: one set
        // from outside since (`ui.set`) stays.
        if self.state.side_panel.mate_hover == self.mate_view.hover_name {
            self.state.side_panel.mate_hover = None;
        }
        self.mate_view.hover_by = None;
        self.mate_view.hover_name = None;
    }

    /// An assembly's mates under its instances, a section that folds with
    /// its count: each a kit row with its state as its icon, its name, and
    /// its kind and value at the right (or why it fails under it), then
    /// its joint's slider when it can be set by hand. Pointing at a row
    /// highlights its marker; a click chooses the mate (again: lets it
    /// go).
    pub(super) fn mates_ui(
        &mut self,
        ui: &mut egui::Ui,
        kit: &crate::kit::Kit,
        outline: &craft3d_engine::AssemblyOutline,
        open: &mut bool,
    ) -> Vec<egui::Rect> {
        let mut statuses = Vec::new();
        let t = kit.t;
        let s = kit.skin;
        self.mate_view.rows.clear();
        self.joints.hits.clear();
        kit.fold_header(ui, open, "Mates", Some(outline.mates.len()), None);
        if !*open {
            return statuses;
        }
        if outline.mates.is_empty() {
            ui.label(
                egui::RichText::new("No mates yet.")
                    .font(kit.small_font())
                    .color(color(s.text_dim)),
            );
        }
        let size = self.joint_range_size();
        let mut hovered = None;
        let mut released = false;
        for mate in &outline.mates {
            let (glyph, ink) = match mate.status {
                "ok" => (crate::kit::Glyph::Check, s.ok),
                "unsolved" => (crate::kit::Glyph::Warning, s.attention),
                _ => (crate::kit::Glyph::Warning, s.failed),
            };
            let value = match mate.value {
                Some((v, true)) => format!(" · {}°", crate::kit::slider::fixed(v.to_degrees(), 1)),
                Some((v, false)) => format!(" · {}", crate::kit::slider::fixed(v, 3)),
                None => String::new(),
            };
            let driven = if mate.driven { " · driven" } else { "" };
            let status = format!("{}{value}{driven}", mate.kind);
            let hover = mate
                .error
                .clone()
                .unwrap_or_else(|| format!("{} · {}", mate.kind, mate.status));
            let chosen = self.state.side_panel.mate.as_deref() == Some(mate.name.as_str());
            let hits = kit.list_row_hits(
                ui,
                crate::kit::ListRow::new(&mate.name)
                    .icon(glyph)
                    .icon_ink(ink)
                    .value(&status)
                    .selected(chosen)
                    .failed(mate.status != "ok" && mate.status != "unsolved"),
            );
            let tip = hits.tip(&mate.name, &status, &hover);
            let shown = hits.response.on_hover_text(tip);
            statuses.push(shown.rect);
            self.mate_view.rows.push((mate.name.clone(), shown.rect));
            if shown.hovered() {
                hovered = Some(mate.name.clone());
            }
            if shown.clicked() {
                self.state.side_panel.mate = (!chosen).then(|| mate.name.clone());
                ui.ctx().request_repaint();
            }
            if let Some(error) = &mate.error {
                ui.indent(("craft3d-mate-error", &mate.name), |ui| {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(error)
                                .color(color(ink))
                                .size(t.secondary_font),
                        )
                        .wrap(),
                    );
                });
            }
            // Its joint, when it can be set by hand: the slider of the
            // Parameters tab, named by its motion.
            if mate.settable && mate.value.is_some() {
                let motion = motion_label(mate);
                ui.indent(("craft3d-mate-joint", &mate.name), |ui| {
                    released |= self.joint_slider(ui, kit, mate, &outline.unit, size, &motion);
                });
            } else if drivable(mate) {
                // Driven by a formula, or following another: held by hand
                // for the view only (`view.drive`).
                let motion = motion_label(mate);
                ui.indent(("craft3d-mate-joint", &mate.name), |ui| {
                    released |= self.drive_slider(ui, kit, mate, &outline.unit, size, &motion);
                });
            }
        }
        if released {
            self.joints.dragging = None;
        }
        match hovered {
            Some(name) => {
                if self.state.side_panel.mate_hover.as_ref() != Some(&name) {
                    ui.ctx().request_repaint();
                }
                self.state.side_panel.mate_hover = Some(name.clone());
                self.mate_view.hover_by = Some(Pointed::Row);
                self.mate_view.hover_name = Some(name);
            }
            None => self.pointer_left(Pointed::Row),
        }
        statuses
    }
}

/// A joint's slider's label: its main free motion (`Angle`, `Distance`).
fn motion_label(mate: &craft3d_engine::OutlineMate) -> String {
    mate.variables
        .first()
        .and_then(|v| v.strip_prefix(mate.name.as_str()))
        .and_then(|v| v.strip_prefix('_'))
        .map_or_else(|| "Value".to_string(), capitalised)
}

/// True when a mate's joint can be held by hand for the view
/// (`view.drive`): solved, with one value to hold (a revolute's angle, a
/// slider's distance, a cylindrical's angle), and not settable as it is.
pub(super) fn drivable(mate: &craft3d_engine::OutlineMate) -> bool {
    !mate.settable
        && mate.status == "ok"
        && mate.value.is_some()
        && matches!(mate.kind, "revolute" | "slider" | "cylindrical")
}

/// `word` with its first letter in capitals (`angle` → `Angle`).
fn capitalised(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}
