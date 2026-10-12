//! The shell: viewport, feature tree and status bar over one engine
//! session. Thin: it reads the scene and draws it, and owns nothing but
//! view state. It never edits a document; that takes a command
//! (`Session::execute`), from whoever drives the session.

use crate::frames::{Mode, Scheduler, Ticket};
use crate::state::{CameraState, ProjectionKind, UiState, VIEWS};
use crate::theme::{Tokens, color, color_alpha};
use crate::viewport::{self, Composed, MAX_SIDE, MIN_SIDE, Overlay, Quality};
use craft3d_engine::{DocumentStatus, FeatureState, Scene, Session};
use craft3d_math::Bbox3;
use std::sync::Arc;

/// The viewport's size before the window has laid it out, in pixels: what
/// `ui.screenshot` renders in a host without a window.
pub const DEFAULT_VIEWPORT: [u32; 2] = [960, 640];

/// While the camera moves the image is rendered this much smaller per side
/// and without supersampling, so orbiting stays fluid on the CPU renderer.
pub const MOVING_DIVISOR: u32 = 2;
/// Supersampling per axis of the image at rest, when the budget allows.
pub const REST_SSAA: u32 = 2;
/// Most samples (pixels times supersampling squared) in one image. The
/// CPU renderer's time is nearly all per sample (measured: about 30 ns per
/// sample, whatever the triangle count up to a few hundred thousand), so
/// these bound a frame to roughly 120 ms at rest and 20 ms while moving on
/// the developer's machine. A viewport with more pixels than the budget
/// gets a smaller image of the same shape, stretched over it.
pub const REST_SAMPLES: u64 = 4_000_000;
pub const MOVING_SAMPLES: u64 = 600_000;

/// One row of the feature tree, as it is shown.
#[derive(Clone, Debug, PartialEq)]
pub struct TreeRow {
    /// `F3`.
    pub id: String,
    pub name: String,
    /// `block`, `sketch`, …
    pub kind: &'static str,
    pub state: FeatureState,
    /// `ok`, `failed`, `suppressed`, `rolled back`; for a sketch that
    /// solved, how far it is constrained.
    pub status: String,
    /// Why the feature failed.
    pub error: Option<String>,
    pub selected: bool,
}

/// What the status bar says about the last command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandReport {
    /// The command id, or the protocol method.
    pub label: String,
    pub ok: bool,
    /// A short form of the result, or the error.
    pub text: String,
}

/// How the image in the viewport was last rendered.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderStats {
    pub width: u32,
    pub height: u32,
    pub ssaa: u32,
    pub triangles: usize,
    /// How long rendering it took, on whichever thread did it. `None`
    /// where there is no clock (a browser).
    pub milliseconds: Option<f64>,
    /// False for the quick preview shown while the camera moves and until
    /// the sharp image is ready.
    pub sharp: bool,
}

/// What the image in the viewport is made from: another image is made
/// only when one of these changes.
#[derive(Clone, Debug, PartialEq)]
struct FrameKey {
    revision: Option<u64>,
    camera: CameraState,
    /// The viewport, in pixels.
    size: [u32; 2],
    /// The backend's largest texture side.
    largest: usize,
    selected: Option<String>,
    tokens: Tokens,
}

/// One image to make.
struct RenderJob {
    scene: Option<Arc<Scene>>,
    camera: CameraState,
    quality: Quality,
    tokens: Tokens,
    selected: Option<String>,
    /// Asked to repaint when the image is ready.
    #[cfg(not(target_arch = "wasm32"))]
    wake: Option<egui::Context>,
}

/// An image that was made.
struct Rendered {
    composed: Composed,
    quality: Quality,
    milliseconds: Option<f64>,
}

impl RenderJob {
    fn render(&self) -> Result<Rendered, String> {
        let elapsed = stopwatch();
        let composed = viewport::compose(
            self.scene.as_deref(),
            &self.camera,
            self.quality,
            &self.tokens,
            self.selected.as_deref(),
        )?;
        Ok(Rendered {
            composed,
            quality: self.quality,
            milliseconds: elapsed(),
        })
    }
}

/// A sharp image that arrived, or why it did not.
type Arrived = (Ticket, Result<Rendered, String>);

#[cfg(not(target_arch = "wasm32"))]
type RenderWorker = crate::frames::Worker<RenderJob, Rendered>;

pub struct Shell {
    pub state: UiState,
    pub tokens: Tokens,
    pub last_command: Option<CommandReport>,
    /// Viewport size in pixels, and pixels per point.
    size: [u32; 2],
    pixels_per_point: f32,
    /// Where the viewport is in the window, in points.
    rect: Option<egui::Rect>,
    texture: Option<egui::TextureHandle>,
    /// What is drawn over the texture, in the pixels of its image.
    overlay: Overlay,
    /// The size of that image.
    overlay_size: [u32; 2],
    /// Which image is on screen and which is being made.
    scheduler: Scheduler<FrameKey>,
    /// The thread sharp images are made on; `None` makes them here.
    #[cfg(not(target_arch = "wasm32"))]
    worker: Option<RenderWorker>,
    /// Sharp images that are ready and not on screen yet.
    arrived: Vec<Arrived>,
    stats: Option<RenderStats>,
    /// How many times the image was rendered.
    renders: u64,
    /// The camera when the drag in progress began.
    drag_from: Option<CameraState>,
    /// The box the camera was last fitted to automatically.
    framed: Bbox3,
    themed: bool,
    quit: bool,
    rows: Vec<TreeRow>,
    /// Why the scene could not be made.
    problem: Option<String>,
    /// Why the image could not be made; cleared by the next image.
    render_problem: Option<String>,
}

impl Default for Shell {
    fn default() -> Self {
        Self::new()
    }
}

fn status_text(state: FeatureState, is_sketch: bool, dof: Option<usize>) -> String {
    match (state, is_sketch, dof) {
        (FeatureState::Ok, true, Some(0)) => "fully constrained".to_string(),
        (FeatureState::Ok, true, Some(dof)) => format!("{dof} DOF"),
        (FeatureState::Ok, _, _) => "ok".to_string(),
        (FeatureState::Failed, _, _) => "failed".to_string(),
        (FeatureState::Suppressed, _, _) => "suppressed".to_string(),
        (FeatureState::RolledBack, _, _) => "rolled back".to_string(),
    }
}

/// A clock for timing a render, where the platform has one.
#[cfg(not(target_arch = "wasm32"))]
fn stopwatch() -> impl FnOnce() -> Option<f64> {
    let start = std::time::Instant::now();
    move || Some(start.elapsed().as_secs_f64() * 1000.0)
}

#[cfg(target_arch = "wasm32")]
fn stopwatch() -> impl FnOnce() -> Option<f64> {
    || None
}

impl Shell {
    pub fn new() -> Self {
        Self {
            state: UiState::default(),
            tokens: Tokens::default(),
            last_command: None,
            size: DEFAULT_VIEWPORT,
            pixels_per_point: 1.0,
            rect: None,
            texture: None,
            overlay: Overlay::default(),
            overlay_size: DEFAULT_VIEWPORT,
            scheduler: Scheduler::new(Mode::Inline),
            #[cfg(not(target_arch = "wasm32"))]
            worker: None,
            arrived: Vec::new(),
            stats: None,
            renders: 0,
            drag_from: None,
            framed: Bbox3::EMPTY,
            themed: false,
            quit: false,
            rows: Vec::new(),
            problem: None,
            render_problem: None,
        }
    }

    /// A shell that has its sharp images made on a thread of its own, so
    /// the thread that draws the window only ever makes quick previews:
    /// what a window uses. [`Self::new`] makes every image where it is
    /// drawn, which is what a host without threads needs and what makes a
    /// frame's outcome independent of timing.
    ///
    /// When the thread cannot be started this is the same as `new`.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn threaded() -> Self {
        let mut shell = Self::new();
        let worker = RenderWorker::start(RenderJob::render, |job: &RenderJob| {
            if let Some(ctx) = &job.wake {
                ctx.request_repaint();
            }
        });
        if let Ok(worker) = worker {
            shell.worker = Some(worker);
            shell.scheduler.set_mode(Mode::Background);
        }
        shell
    }

    /// There are no threads here: the same as [`Self::new`].
    #[cfg(target_arch = "wasm32")]
    pub fn threaded() -> Self {
        Self::new()
    }

    /// True when sharp images are made on a thread of their own.
    pub fn renders_in_background(&self) -> bool {
        self.scheduler.mode() == Mode::Background
    }

    /// Waits until the sharp image that is being made has been made, for
    /// at most `timeout`. The next frame puts it on screen (if it is still
    /// the image of what that frame shows). True when nothing is being
    /// made any more; false when the time ran out.
    ///
    /// For hosts that need the finished picture now (a recording, a test):
    /// the window itself never waits.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn wait_for_sharp(&mut self, timeout: std::time::Duration) -> bool {
        let Some(ticket) = self.scheduler.pending() else {
            return true;
        };
        if self.arrived.iter().any(|(arrived, _)| *arrived == ticket) {
            return true;
        }
        let Some(worker) = &self.worker else {
            return false;
        };
        let deadline = std::time::Instant::now().checked_add(timeout);
        loop {
            let left = deadline.map_or(timeout, |deadline| {
                deadline.saturating_duration_since(std::time::Instant::now())
            });
            let Some(done) = worker.wait(left) else {
                return false;
            };
            let found = done.ticket == ticket;
            self.arrived.push((done.ticket, done.result));
            if found {
                return true;
            }
            if left.is_zero() {
                return false;
            }
        }
    }

    /// Nothing is made elsewhere here, so there is nothing to wait for.
    #[cfg(target_arch = "wasm32")]
    pub fn wait_for_sharp(&mut self, _timeout: std::time::Duration) -> bool {
        true
    }

    /// The viewport's size in pixels.
    pub fn viewport_size(&self) -> [u32; 2] {
        self.size
    }

    pub fn pixels_per_point(&self) -> f32 {
        self.pixels_per_point
    }

    /// Where the viewport was laid out in the last frame, in points.
    pub fn viewport_rect(&self) -> Option<egui::Rect> {
        self.rect
    }

    /// The rows of the feature tree as of the last [`Self::sync`].
    pub fn tree_rows(&self) -> &[TreeRow] {
        &self.rows
    }

    pub fn render_stats(&self) -> Option<RenderStats> {
        self.stats
    }

    /// How many times the viewport image has been rendered.
    pub fn render_count(&self) -> u64 {
        self.renders
    }

    /// Why there is no picture, or not the right one, when that is so.
    pub fn problem(&self) -> Option<&str> {
        self.problem.as_deref().or(self.render_problem.as_deref())
    }

    /// True once something asked the application to close.
    pub fn quit_requested(&self) -> bool {
        self.quit
    }

    pub fn request_quit(&mut self) {
        self.quit = true;
    }

    fn aspect(&self) -> f64 {
        let [w, h] = self.size;
        f64::from(w.max(1)) / f64::from(h.max(1))
    }

    /// Brings the shell up to date with the session: the scene of the
    /// active document, the tree rows, the selection, and the camera when
    /// it follows the model. Called every frame and after every request.
    pub fn sync(&mut self, session: &mut Session) -> Option<Arc<Scene>> {
        let scene = match session.scene(None) {
            Ok(scene) => {
                self.problem = None;
                scene
            }
            Err(e) => {
                self.problem = Some(e.message().to_string());
                None
            }
        };
        let Some(scene) = scene else {
            self.rows.clear();
            self.state.selected_feature = None;
            self.framed = Bbox3::EMPTY;
            return None;
        };
        if let Some(first) = scene.problems.first() {
            self.problem = Some(first.clone());
        }
        if self
            .state
            .selected_feature
            .as_ref()
            .is_some_and(|id| !scene.features.iter().any(|f| f.id == *id))
        {
            self.state.selected_feature = None;
        }
        let selected = self.state.selected_feature.as_deref();
        self.rows = scene
            .features
            .iter()
            .map(|f| TreeRow {
                id: f.id.clone(),
                name: f.name.clone(),
                kind: f.kind,
                state: f.state,
                status: status_text(f.state, f.is_sketch, f.dof),
                error: f.error.clone(),
                selected: selected == Some(f.id.as_str()),
            })
            .collect();
        if self.state.camera.auto_fit && scene.bbox != self.framed {
            self.framed = scene.bbox;
            let aspect = self.aspect();
            self.state.camera.fit(&scene.bbox, aspect);
        }
        Some(scene)
    }

    /// Frames everything, and keeps following the model from now on.
    /// Returns false when there is nothing to frame.
    pub fn fit(&mut self, session: &mut Session) -> bool {
        let Some(scene) = self.sync(session) else {
            return false;
        };
        if scene.bbox.is_empty() {
            return false;
        }
        let aspect = self.aspect();
        self.state.camera.fit(&scene.bbox, aspect);
        self.state.camera.auto_fit = true;
        self.framed = scene.bbox;
        true
    }

    /// Draws the whole shell into `ui` (the window's root) for one frame.
    pub fn ui(&mut self, ui: &mut egui::Ui, session: &mut Session) {
        if !self.themed {
            self.tokens.apply(ui.ctx());
            self.themed = true;
        }
        let scene = self.sync(session);
        let status = session.document_status();
        if self.state.panels.status {
            egui::Panel::bottom("craft3d-status")
                .resizable(false)
                .show(ui, |ui| self.status_bar(ui, status.as_ref()));
        }
        if self.state.panels.tree {
            egui::Panel::left("craft3d-tree")
                .resizable(true)
                .default_size(260.0)
                .show(ui, |ui| self.tree(ui, status.is_some()));
        }
        egui::CentralPanel::no_frame().show(ui, |ui| self.viewport(ui, scene.as_ref()));
    }

    fn status_bar(&self, ui: &mut egui::Ui, status: Option<&DocumentStatus>) {
        let dim = color(self.tokens.text_dim);
        ui.horizontal(|ui| {
            match status {
                Some(status) => {
                    ui.label(format!("{} · {}", status.length_unit, status.angle_unit));
                    ui.separator();
                    let name = status.path.as_deref().unwrap_or("untitled");
                    let dirty = if status.dirty { " *" } else { "" };
                    ui.label(format!("{name}{dirty}"));
                }
                None => {
                    ui.colored_label(dim, "no document");
                }
            }
            if let Some(report) = &self.last_command {
                ui.separator();
                let tint = if report.ok {
                    self.tokens.ok
                } else {
                    self.tokens.failed
                };
                ui.colored_label(color(tint), &report.label);
                ui.add(egui::Label::new(egui::RichText::new(&report.text).color(dim)).truncate());
            }
            if let Some(stats) = self.stats {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let time = stats
                        .milliseconds
                        .map_or_else(String::new, |ms| format!(" · {ms:.0} ms"));
                    ui.colored_label(
                        dim,
                        format!(
                            "CPU {}×{} · {} triangles{time}",
                            stats.width, stats.height, stats.triangles
                        ),
                    );
                });
            }
        });
    }

    fn tree(&mut self, ui: &mut egui::Ui, has_document: bool) {
        let tokens = self.tokens;
        let dim = color(tokens.text_dim);
        ui.add_space(tokens.inset * 0.5);
        ui.label(egui::RichText::new("Features").strong());
        ui.separator();
        if self.rows.is_empty() {
            let hint = if has_document {
                "No features yet."
            } else {
                "No document."
            };
            ui.colored_label(dim, hint);
            return;
        }
        let mut clicked = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            for row in &self.rows {
                let tint = match row.state {
                    FeatureState::Ok => tokens.ok,
                    FeatureState::Failed => tokens.failed,
                    FeatureState::Suppressed | FeatureState::RolledBack => tokens.inactive,
                };
                ui.horizontal(|ui| {
                    ui.colored_label(color(tint), "●");
                    if ui.selectable_label(row.selected, &row.name).clicked() {
                        clicked = Some(row.id.clone());
                    }
                    ui.colored_label(dim, format!("{} · {}", row.kind, row.status));
                });
                if let Some(error) = &row.error {
                    ui.indent(("craft3d-error", &row.id), |ui| {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(error)
                                    .color(color(tokens.failed))
                                    .small(),
                            )
                            .wrap(),
                        );
                    });
                }
            }
        });
        // Clicking the selected row again clears the selection. Which row
        // is highlighted is view state: nothing in the document changes.
        if let Some(id) = clicked {
            self.state.selected_feature = if self.state.selected_feature.as_ref() == Some(&id) {
                None
            } else {
                Some(id)
            };
        }
    }

    fn viewport(&mut self, ui: &mut egui::Ui, shared: Option<&Arc<Scene>>) {
        let scene = shared.map(Arc::as_ref);
        let tokens = self.tokens;
        let (rect, response) = ui.allocate_exact_size(ui.available_size(), egui::Sense::drag());
        let scale = ui.pixels_per_point();
        self.rect = Some(rect);
        self.pixels_per_point = scale;
        // Rounded and clamped to a usable image size.
        let side = |points: f32| {
            let pixels = (f64::from(points) * f64::from(scale)).round();
            if pixels.is_finite() {
                (pixels.clamp(f64::from(MIN_SIDE), f64::from(MAX_SIDE))) as u32
            } else {
                MIN_SIDE
            }
        };
        let size = [side(rect.width()), side(rect.height())];
        // A window being resized changes size every frame: those frames are
        // drawn like a moving camera, and the sharp image follows.
        let resized = size != self.size && self.texture.is_some();
        if size != self.size {
            self.size = size;
            // The model was framed for another shape of viewport.
            self.framed = Bbox3::EMPTY;
            if self.state.camera.auto_fit
                && let Some(scene) = scene
            {
                self.framed = scene.bbox;
                let aspect = self.aspect();
                self.state.camera.fit(&scene.bbox, aspect);
            }
        }

        let moving = self.pointer(ui, &response, rect, scene) || resized;

        // A quick preview whenever what is shown changes, and the sharp
        // image once the camera rests (see `frames`).
        let largest = ui.input(|i| i.max_texture_side);
        let key = FrameKey {
            revision: scene.map(|s| s.revision),
            camera: self.state.camera,
            size,
            largest,
            selected: self.state.selected_feature.clone(),
            tokens,
        };
        self.refresh(ui.ctx(), shared, &key, moving);
        if moving {
            // One more frame after the last movement, for the sharp image.
            ui.ctx().request_repaint();
        }

        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, color(tokens.viewport_background));
        if let Some(texture) = &self.texture {
            let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            painter.image(texture.id(), rect, uv, egui::Color32::WHITE);
        }
        paint_overlay(&painter, rect, self.overlay_size, &self.overlay, &tokens);

        let inset = tokens.inset;
        let inner = rect.shrink(inset);
        let hint = |text: &str| {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                text,
                egui::FontId::proportional(15.0),
                color(tokens.text_dim),
            );
        };
        match (scene, self.problem()) {
            (None, None) => hint("No document. Run file.new or file.open."),
            (Some(scene), _) if scene.bbox.is_empty() => hint("Empty document."),
            _ => {}
        }
        if let Some(problem) = self.problem() {
            painter.text(
                inner.left_bottom(),
                egui::Align2::LEFT_BOTTOM,
                problem,
                egui::FontId::proportional(13.0),
                color(tokens.failed),
            );
        }
        if !self.state.notice.is_empty() {
            let galley = painter.layout_no_wrap(
                self.state.notice.clone(),
                egui::FontId::proportional(17.0),
                color(tokens.notice_text),
            );
            let text_size = galley.size();
            let center = egui::pos2(rect.center().x, inner.top() + text_size.y);
            let plate = egui::Rect::from_center_size(center, text_size).expand(inset * 0.8);
            painter.rect_filled(
                plate,
                egui::CornerRadius::same(tokens.notice_radius),
                color_alpha(tokens.notice_fill, tokens.notice_opacity),
            );
            painter.galley(
                egui::pos2(center.x - text_size.x * 0.5, center.y - text_size.y * 0.5),
                galley,
                color(tokens.notice_text),
            );
        }

        // View buttons, bottom-right: camera only, so not commands.
        let mut view = None;
        let mut fit = false;
        let mut toggle = false;
        let bar = egui::Rect::from_min_max(
            egui::pos2(inner.left(), inner.bottom() - 22.0),
            inner.right_bottom(),
        );
        ui.scope_builder(egui::UiBuilder::new().max_rect(bar), |ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let projection = match self.state.camera.projection {
                    ProjectionKind::Perspective => "Perspective",
                    ProjectionKind::Orthographic => "Orthographic",
                };
                toggle = ui.small_button(projection).clicked();
                for (name, ..) in VIEWS.iter().rev() {
                    if ui.small_button(*name).clicked() {
                        view = Some(*name);
                    }
                }
                fit = ui.small_button("Fit").clicked();
            });
        });
        if let Some(name) = view {
            // The names come from the table, so this cannot fail.
            let _ = self.state.camera.set_view(name);
            ui.ctx().request_repaint();
        }
        if toggle {
            self.state.camera.projection = match self.state.camera.projection {
                ProjectionKind::Perspective => ProjectionKind::Orthographic,
                ProjectionKind::Orthographic => ProjectionKind::Perspective,
            };
            ui.ctx().request_repaint();
        }
        if fit || view.is_some() {
            self.fit_to(scene);
            ui.ctx().request_repaint();
        }
    }

    fn fit_to(&mut self, scene: Option<&Scene>) {
        if let Some(scene) = scene
            && !scene.bbox.is_empty()
        {
            let aspect = self.aspect();
            self.state.camera.fit(&scene.bbox, aspect);
            self.state.camera.auto_fit = true;
            self.framed = scene.bbox;
        }
    }

    /// Mouse and keys over the viewport. Returns true while the camera is
    /// being moved.
    fn pointer(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        rect: egui::Rect,
        scene: Option<&Scene>,
    ) -> bool {
        let mut moving = false;
        if response.drag_started() {
            self.drag_from = Some(self.state.camera);
        }
        if response.dragged() {
            // From where the drag began, by how far the pointer is from
            // there now: the result does not depend on how the movement was
            // cut into frames.
            if let (Some(from), Some(total)) = (self.drag_from, response.total_drag_delta()) {
                let (primary, secondary, middle, shift) = ui.input(|i| {
                    (
                        i.pointer.button_down(egui::PointerButton::Primary),
                        i.pointer.button_down(egui::PointerButton::Secondary),
                        i.pointer.button_down(egui::PointerButton::Middle),
                        i.modifiers.shift,
                    )
                });
                let mut camera = from;
                let (dx, dy) = (f64::from(total.x), f64::from(total.y));
                if secondary || middle || (primary && shift) {
                    camera.pan(dx, dy, f64::from(rect.height()));
                } else if primary {
                    camera.orbit(dx, dy);
                }
                self.state.camera = camera;
            }
            moving = true;
        } else {
            self.drag_from = None;
        }
        if response.contains_pointer() {
            let (scroll, fit) = ui.input(|i| {
                (
                    f64::from(i.smooth_scroll_delta().y),
                    i.key_pressed(egui::Key::F),
                )
            });
            if scroll != 0.0 {
                self.state.camera.zoom(scroll);
                moving = true;
            }
            if fit {
                self.fit_to(scene);
            }
        }
        moving
    }

    /// Brings the image on screen up to date with `key`: makes a preview
    /// here when nothing of the key is on screen, starts the sharp image,
    /// and shows the sharp images that arrived and are still current.
    fn refresh(
        &mut self,
        ctx: &egui::Context,
        scene: Option<&Arc<Scene>>,
        key: &FrameKey,
        moving: bool,
    ) {
        #[cfg(not(target_arch = "wasm32"))]
        if self.worker.as_ref().is_some_and(|worker| !worker.alive()) {
            // The thread is gone (it cannot be, but if): go on without it
            // rather than wait for images that will not come.
            self.worker = None;
            self.scheduler.set_mode(Mode::Inline);
        }
        if self.texture.is_none() {
            self.scheduler.forget();
        }
        let job = |quality: Quality| RenderJob {
            scene: scene.cloned(),
            camera: key.camera,
            quality,
            tokens: key.tokens,
            selected: key.selected.clone(),
            #[cfg(not(target_arch = "wasm32"))]
            wake: Some(ctx.clone()),
        };
        let plan = self.scheduler.plan(key, moving);
        if plan.preview {
            match job(image_quality(key.size, true, key.largest)).render() {
                Ok(rendered) => {
                    self.show(ctx, rendered, false);
                    self.scheduler.preview_shown(key);
                }
                Err(e) => {
                    self.render_problem = Some(format!("the viewport cannot be rendered: {e}"));
                }
            }
        }
        if let Some(ticket) = plan.sharp {
            let job = job(image_quality(key.size, false, key.largest));
            if let Some(job) = self.hand_over(ticket, job) {
                // No thread to do it: here and now.
                self.arrived.push((ticket, job.render()));
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(worker) = &self.worker {
            while let Some(done) = worker.poll() {
                self.arrived.push((done.ticket, done.result));
            }
        }
        for (ticket, result) in std::mem::take(&mut self.arrived) {
            match result {
                // Stale images are dropped here: `deliver` accepts only
                // the image of what this frame shows.
                Ok(rendered) => {
                    if self.scheduler.deliver(ticket) {
                        self.show(ctx, rendered, true);
                    }
                }
                Err(e) => {
                    if self.scheduler.fail(ticket) {
                        self.render_problem = Some(format!("the viewport cannot be rendered: {e}"));
                    }
                }
            }
        }
    }

    /// Gives the job to the render thread. Hands it back when there is no
    /// thread to take it.
    #[cfg(not(target_arch = "wasm32"))]
    fn hand_over(&mut self, ticket: Ticket, job: RenderJob) -> Option<RenderJob> {
        match &self.worker {
            Some(worker) => worker.submit(ticket, job).err(),
            None => Some(job),
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn hand_over(&mut self, _ticket: Ticket, job: RenderJob) -> Option<RenderJob> {
        Some(job)
    }

    /// Puts an image on screen.
    fn show(&mut self, ctx: &egui::Context, rendered: Rendered, sharp: bool) {
        let Rendered {
            composed,
            quality,
            milliseconds,
        } = rendered;
        self.renders = self.renders.saturating_add(1);
        self.stats = Some(RenderStats {
            width: composed.width,
            height: composed.height,
            ssaa: quality.ssaa,
            triangles: composed.triangles,
            milliseconds,
            sharp,
        });
        let image = egui::ColorImage::from_rgb(
            [composed.width as usize, composed.height as usize],
            &composed.rgb,
        );
        match &mut self.texture {
            Some(texture) => texture.set(image, egui::TextureOptions::LINEAR),
            None => {
                self.texture =
                    Some(ctx.load_texture("craft3d-viewport", image, egui::TextureOptions::LINEAR));
            }
        }
        self.overlay = composed.overlay;
        self.overlay_size = [composed.width, composed.height];
        self.render_problem = None;
    }
}

/// How to render the image for a viewport of `size` pixels: full size at
/// rest, supersampled when that fits [`REST_SAMPLES`]; [`MOVING_DIVISOR`]
/// times smaller and unfiltered while the camera moves, within
/// [`MOVING_SAMPLES`]. Never larger than the backend's largest texture
/// (`largest` pixels per side). An image that has to be smaller than the
/// viewport keeps its shape and is stretched over it.
pub fn image_quality(size: [u32; 2], moving: bool, largest: usize) -> Quality {
    let largest = u32::try_from(largest)
        .unwrap_or(MAX_SIDE)
        .clamp(MIN_SIDE, MAX_SIDE);
    let (divisor, budget) = if moving {
        (MOVING_DIVISOR, MOVING_SAMPLES)
    } else {
        (1, REST_SAMPLES)
    };
    let (mut width, mut height) = (
        (size[0] / divisor).max(MIN_SIDE),
        (size[1] / divisor).max(MIN_SIDE),
    );
    // The same factor for both sides, the smaller of what the texture limit
    // and the sample budget allow.
    let longest = f64::from(width.max(height));
    let pixels = f64::from(width) * f64::from(height);
    let factor = (f64::from(largest) / longest).min((budget as f64 / pixels).sqrt());
    if factor < 1.0 {
        // Rounded down: both products stay within their limit.
        let shrink = |side: u32| (f64::from(side) * factor).floor() as u32;
        (width, height) = (shrink(width), shrink(height));
    }
    let (width, height) = (
        width.clamp(MIN_SIDE, largest),
        height.clamp(MIN_SIDE, largest),
    );
    let supersampled =
        u64::from(width) * u64::from(height) * u64::from(REST_SSAA) * u64::from(REST_SSAA);
    Quality {
        width,
        height,
        ssaa: if !moving && supersampled <= budget {
            REST_SSAA
        } else {
            1
        },
    }
}

/// Paints the overlay, which is in image pixels, into `rect`, which is the
/// same image in points.
fn paint_overlay(
    painter: &egui::Painter,
    rect: egui::Rect,
    size: [u32; 2],
    overlay: &Overlay,
    tokens: &Tokens,
) {
    let [w, h] = size;
    let (sx, sy) = (
        f64::from(rect.width()) / f64::from(w.max(1)),
        f64::from(rect.height()) / f64::from(h.max(1)),
    );
    let place = |p: [f64; 2]| {
        egui::pos2(
            rect.left() + (p[0] * sx) as f32,
            rect.top() + (p[1] * sy) as f32,
        )
    };
    for line in &overlay.lines {
        let tint = color_alpha(
            line.kind.color(tokens),
            viewport::opacity(line.hidden, tokens),
        );
        painter.line_segment(
            [place(line.from), place(line.to)],
            egui::Stroke::new(line.kind.width(tokens), tint),
        );
    }
    for point in &overlay.points {
        let tint = color_alpha(
            point.kind.color(tokens),
            viewport::opacity(point.hidden, tokens),
        );
        painter.circle_filled(place(point.at), tokens.point_radius, tint);
    }
}
