//! The studio look's options, in the Studio split button's popover
//! (`shell/popover.rs`) while the look is on and the popover open (and no
//! analysis, which is drawn in the plain look, is shown): the environment and its rotation, the exposure, the
//! background, the ground and its reflection, the tone curve, the
//! shadows, ambient occlusion and reflections, bloom and a lens (its
//! f-number on a slider of full stops, and where it focuses: the
//! selection, the last click, the model's centre or a distance). Every
//! control drives a field of `state.look` (the fields `ui.set {"look":
//! ...}` takes and `ui.get` gives), so a change is drawn at once and an
//! agent sees it.

use super::Shell;
use crate::kit::{Kit, Segment, Slider};
use crate::state::LookState;
use crate::theme::{Tokens, color};
use craft3d_math::Portable;
use egui::Rect;

/// The environments a person can choose (`look.environment`).
pub const ENVIRONMENTS: [(&str, &str); 3] = [
    ("studio", "Studio"),
    ("outdoor", "Outdoor"),
    ("workshop", "Workshop"),
];
/// The backgrounds (`look.background`).
pub const BACKGROUNDS: [(&str, &str); 2] = [("environment", "Environment"), ("plain", "Plain")];
/// The tone curves (`look.tonemap`).
pub const TONEMAPS: [(&str, &str); 3] = [("agx", "AgX"), ("aces", "ACES"), ("linear", "Linear")];
/// The f-number a lens is switched on with (`look.dof`), the engine's
/// default.
const FSTOP: f64 = craft3d_engine::look::DEFAULT_FSTOP;
/// The f-numbers the aperture slider stops at: the full stops of a
/// camera lens, each letting in half the light of the one before, so the
/// slider is logarithmic in the f-number. `ui.set` takes any f-number
/// from 0.5 to 64.
pub const FSTOPS: [f64; 9] = [1.4, 2.0, 2.8, 4.0, 5.6, 8.0, 11.0, 16.0, 22.0];
/// Where a lens can focus (`look.dof.focus`), as the panel offers it:
/// the selection, the last click, the model's centre, or a distance
/// along the view.
pub const FOCUSES: [(&str, &str); 4] = [
    ("selection", "Selection"),
    ("picked", "Click"),
    ("centre", "Centre"),
    ("distance", "Distance"),
];
/// The range of the focus distance slider, as factors of the camera's
/// distance to its target.
const DISTANCE_RANGE: [f64; 2] = [0.05, 4.0];

/// Where each control of the panel was drawn, by name: `environment.<v>`,
/// `background.<v>`, `tonemap.<v>` and `focus.<v>` ([`FOCUSES`]) for the
/// choices, `rotation`, `exposure`, `groundReflection`, `bloom`, `fstop`
/// and `focusDistance` for the sliders (their track only), `ground`,
/// `shadows`, `ao`, `reflections` and `lens` for the toggles, `lensNote`
/// for the lens's note (where it focuses instead, or why it blurs
/// nothing), `apertureHint` for the line under the aperture, and `panel`
/// for the whole panel.
pub type LookHits = Vec<(String, Rect)>;

/// What the lens rows need to know besides the look: where the lens
/// focuses now, and the camera's distance to its target (the scale of
/// the focus distance slider).
struct LensContext {
    at: craft3d_engine::look::FocusAt,
    /// The focus distance along the view now, if there is a lens.
    distance: Option<f64>,
    camera_distance: f64,
    /// Why the lens blurs nothing ([`craft3d_engine::look::no_blur`]).
    no_blur: Option<&'static str>,
    /// An orthographic view: there is no lens to set.
    orthographic: bool,
}

impl Shell {
    /// Draws the panel as the popover of the Studio split button `chip`
    /// at `place` ([`super::popover_place`]) while the studio look is on
    /// and its popover open.
    pub(super) fn look_panel(&mut self, ui: &mut egui::Ui, place: Option<Rect>, chip: Rect) {
        let Some(place) = place else {
            self.look_hits.clear();
            return;
        };
        if !self.state.look.is_studio()
            || self.state.analysis.is_on()
            || !self.state.panels.look_options
        {
            self.look_hits.clear();
            return;
        }
        let tokens = self.tokens;
        let before = self.state.look.clone();
        let mut hits = LookHits::new();
        let now = self.lens_focus();
        let lens = LensContext {
            at: self.focus_at(),
            distance: now.as_ref().and_then(|focus| focus.distance),
            camera_distance: self.state.camera.distance,
            no_blur: now.as_ref().and_then(|focus| focus.no_blur),
            orthographic: self.state.camera.projection
                == crate::state::ProjectionKind::Orthographic,
        };
        let sheet = self.narrow();
        let panel = self.popover(ui, "studio-options", place, chip, |shell, ui| {
            look_options(ui, &mut shell.state.look, &tokens, &lens, &mut hits, sheet);
        });
        hits.push(("panel".to_string(), panel));
        self.look_hits = hits;
        if self.state.look != before {
            ui.ctx().request_repaint();
        }
    }

    /// Where the studio look's lens focuses as the viewport draws it now
    /// (`ui.get`'s `lens`), or `None` without a lens.
    pub fn lens_focus(&self) -> Option<LensFocus> {
        // An analysis is drawn in the plain look, without a lens.
        if !self.state.look.is_studio() || self.state.analysis.is_on() {
            return None;
        }
        let spec = self.state.look.spec().ok()?;
        let asked = spec.dof?.focus;
        let model = self
            .shown_scene
            .as_ref()
            .map_or(craft3d_math::Bbox3::EMPTY, |scene| scene.bbox);
        let at = self.focus_at();
        let dof = spec.resolved(&model, self.length_mm, &at, 1).dof?;
        let camera = self.state.camera.to_render();
        let point = match dof.focus {
            craft3d_render::studio::polish::Focus::Point(p) => Some(p),
            craft3d_render::studio::polish::Focus::Distance(_) => None,
        };
        let distance = camera
            .axes()
            .ok()
            .and_then(|axes| dof.focus_distance(axes.eye, axes.forward));
        Some(LensFocus {
            focus: asked,
            point,
            distance,
            fallback: asked.fallback(&at),
            no_blur: craft3d_engine::look::no_blur(&dof, &camera),
        })
    }

    /// Whether the studio look's popover is open (the Studio caret opens
    /// and closes it; it shows while the look is on).
    pub fn look_panel_open(&self) -> bool {
        self.state.panels.look_options
    }

    /// Where the caret of the Studio split button was drawn.
    pub fn look_caret(&self) -> Option<Rect> {
        self.look_caret
    }

    /// Where the caret of the Analysis split button was drawn.
    pub fn analysis_caret(&self) -> Option<Rect> {
        self.analysis_caret
    }

    /// Where the controls of the studio look's panel are, in points, once
    /// it was drawn ([`LookHits`]); empty while it is not shown.
    pub fn look_hits(&self) -> &[(String, Rect)] {
        &self.look_hits
    }

    /// Whether the Analysis popover is open (it shows while an analysis
    /// is on).
    pub fn analysis_panel_open(&self) -> bool {
        self.state.panels.analysis_options
    }

    /// Where the Analysis popover's panel (`panel`) was drawn, in points;
    /// empty while it is not shown.
    pub fn analysis_hits(&self) -> &[(String, Rect)] {
        &self.analysis_hits
    }
}

/// Where the lens focuses ([`Shell::lens_focus`]).
#[derive(Clone, Debug, PartialEq)]
pub struct LensFocus {
    /// Where it was asked to focus (`look.dof.focus`).
    pub focus: craft3d_engine::look::FocusSpec,
    /// The point it focuses on (the plane square to the view through
    /// it), `None` for a focus given as a distance.
    pub point: Option<[f64; 3]>,
    /// How far along the view from the eye it focuses, `None` where that
    /// is not ahead of the eye.
    pub distance: Option<f64>,
    /// Why it focuses on the model's centre rather than where it was
    /// asked to ([`craft3d_engine::look::FocusSpec::fallback`]).
    pub fallback: Option<&'static str>,
    /// Why it blurs nothing: an orthographic view, a focus behind the
    /// camera ([`craft3d_engine::look::no_blur`]).
    pub no_blur: Option<&'static str>,
}

impl LensFocus {
    /// As `ui.get` gives it.
    pub fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "focus": self.focus.json(),
            "point": self.point,
            "distance": self.distance,
            "fallback": self.fallback,
            "noBlur": self.no_blur,
        })
    }
}

/// The f-number as a lens's ring says it: `f/2.8`, `f/11`.
pub fn fstop_text(fstop: f64) -> String {
    if fstop >= 10.0 {
        format!("f/{fstop:.0}")
    } else {
        let text = format!("{fstop:.1}");
        format!("f/{}", text.strip_suffix(".0").unwrap_or(&text))
    }
}

/// The full stop ([`FSTOPS`]) nearest to `fstop` on the slider's
/// logarithmic scale, as its index.
pub fn nearest_stop(fstop: f64) -> usize {
    let log = fstop.max(1e-3).pln();
    FSTOPS
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| (a.pln() - log).abs().total_cmp(&(b.pln() - log).abs()))
        .map_or(0, |(i, _)| i)
}

/// The lens's section (board s1): its heading with the switch, the
/// f-number on a slider of full stops with the hint under it, where it
/// focuses as a row of its own with its note under it.
fn lens_rows(
    ui: &mut egui::Ui,
    kit: &Kit,
    look: &mut LookState,
    lens: &LensContext,
    hits: &mut LookHits,
) {
    use craft3d_engine::look::{FocusSpec, LookSpec};
    // The lens as the engine reads it: a value it refuses is shown as
    // the default and replaced only once a control is moved.
    let dof = look.dof.as_ref().and_then(|value| {
        LookSpec::patched(&LookSpec::default(), &serde_json::json!({ "dof": value }))
            .ok()?
            .dof
    });
    let mut on = look.dof.is_some();
    ui.horizontal(|ui| {
        kit.section_header(ui, "Lens");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let response = kit
                .toggle(ui, &mut on, "")
                .on_hover_text("Depth of field: a thin lens that blurs what is out of focus");
            hits.push(("lens".to_string(), response.rect));
        });
    });
    if on != look.dof.is_some() {
        look.dof = on.then(|| serde_json::json!({ "fstop": FSTOP, "focus": "selection" }));
        return;
    }
    if !on {
        return;
    }
    // An orthographic view has no lens: nothing to set.
    let settable = !lens.orthographic;
    let mut fstop = dof.map_or(FSTOP, |d| d.fstop);
    let mut focus = dof.map_or(FocusSpec::Centre, |d| d.focus);
    let mut changed = false;
    let last = (FSTOPS.len() - 1) as f64;
    let mut stop = nearest_stop(fstop) as f64;
    let before = stop;
    let response = kit
        .slider(
            ui,
            Slider::new("Aperture", &mut stop, 0.0..=last)
                .text(fstop_text(fstop))
                .enabled(settable)
                .widths(kit.t.popover_label_width, kit.t.popover_value_width),
        )
        .on_hover_text(
            "The f-number: a smaller one opens the lens wider and blurs more of what is out of focus",
        );
    hits.push(("fstop".to_string(), response.rect));
    if settable
        && stop != before
        && let Some(f) = FSTOPS.get(stop.round().clamp(0.0, last) as usize)
    {
        fstop = *f;
        changed = true;
    }
    // Which way blurs more, under the slider.
    let hint = indented(ui, kit, |ui| {
        ui.label(
            egui::RichText::new(APERTURE_HINT)
                .font(kit.small_font())
                .color(color(kit.skin.text_dim)),
        )
    });
    hits.push(("apertureHint".to_string(), hint.rect));
    ui.label(
        egui::RichText::new("Focus")
            .font(kit.font())
            .color(color(kit.skin.text)),
    );
    let segments: Vec<Segment<'_>> = FOCUSES
        .iter()
        .map(|(_, shown)| Segment::Text(shown))
        .collect();
    let mut chosen = FOCUSES
        .iter()
        .position(|(name, _)| {
            matches!(
                (*name, focus),
                ("selection", FocusSpec::Selection)
                    | ("picked", FocusSpec::Picked)
                    | ("centre", FocusSpec::Centre)
                    | ("distance", FocusSpec::Distance(_))
            )
        })
        .unwrap_or(usize::MAX);
    let was = chosen;
    let choice = kit.segmented(ui, &mut chosen, &segments);
    for ((name, _), rect) in FOCUSES.iter().zip(&choice.items) {
        hits.push((format!("focus.{name}"), *rect));
    }
    if settable
        && chosen != was
        && let Some((name, _)) = FOCUSES.get(chosen)
    {
        focus = match *name {
            "selection" => FocusSpec::Selection,
            "picked" => FocusSpec::Picked,
            "centre" => FocusSpec::Centre,
            // From where it focuses now, so the picture does not jump.
            _ => FocusSpec::Distance(
                lens.distance
                    .filter(|d| d.is_finite() && *d > 0.0)
                    .unwrap_or(lens.camera_distance),
            ),
        };
        changed = true;
    }
    if let FocusSpec::Distance(d) = focus {
        let scale = lens.camera_distance.max(f64::MIN_POSITIVE);
        // The slider runs over the distance's logarithm.
        let (lo, hi) = (scale * DISTANCE_RANGE[0], scale * DISTANCE_RANGE[1]);
        let mut at = (d.max(lo).pln() - lo.pln()) / (hi.pln() - lo.pln()).max(f64::MIN_POSITIVE);
        let was = at;
        let response = kit.slider(
            ui,
            Slider::new("Distance", &mut at, 0.0..=1.0)
                .text(significant(d))
                .enabled(settable)
                .widths(kit.t.popover_label_width, kit.t.popover_value_width),
        );
        hits.push(("focusDistance".to_string(), response.rect));
        if settable && at != was {
            let d = (lo.pln() + at.clamp(0.0, 1.0) * (hi.pln() - lo.pln())).pexp();
            if d.is_finite() && d > 0.0 {
                focus = FocusSpec::Distance(d);
                changed = true;
            }
        }
    }
    // What the lens does instead of what it was asked: no blur at all
    // first, then where it focuses instead.
    let note = if let Some(why) = lens.no_blur {
        Some(why.to_string())
    } else {
        match focus {
            FocusSpec::Point(p) => Some(format!(
                "on the point [{}, {}, {}]",
                significant(p[0]),
                significant(p[1]),
                significant(p[2])
            )),
            _ => focus.fallback(&lens.at).map(str::to_string),
        }
    };
    if let Some(note) = note {
        let response = ui.add(
            egui::Label::new(
                egui::RichText::new(note)
                    .font(kit.small_font())
                    .color(color(kit.skin.text_dim)),
            )
            .wrap(),
        );
        hits.push(("lensNote".to_string(), response.rect));
    }
    if changed {
        look.dof = Some(serde_json::json!({ "fstop": fstop, "focus": focus.json() }));
    }
}

/// What the aperture's slider says under it: which way blurs more.
pub const APERTURE_HINT: &str = "smaller f-number: more blur";

/// A number to four significant digits, as the panel writes lengths.
fn significant(x: f64) -> String {
    if !x.is_finite() || x == 0.0 {
        return format!("{x}");
    }
    let digits = (3 - x.abs().plog10().floor() as i32).clamp(0, 6) as usize;
    format!("{x:.digits$}")
}

/// The controls (board s1): three sections, Light, Scene and Lens, each
/// a heading with the info accent's bar and rows of a label and the
/// kit's control.
fn look_options(
    ui: &mut egui::Ui,
    look: &mut LookState,
    tokens: &Tokens,
    lens: &LensContext,
    hits: &mut LookHits,
    sheet: bool,
) {
    let kit = Kit::plate(tokens);
    let t = kit.t;
    ui.spacing_mut().item_spacing.y = t.nav_spacing;
    // As a bottom sheet (a phone, m4): one section at a time, chosen
    // above them.
    let shown = if sheet {
        let id = egui::Id::new("craft3d-studio-sheet-tab");
        let mut tab = ui.data(|d| d.get_temp::<usize>(id)).unwrap_or(0);
        let choice = kit.segmented_full(
            ui,
            &mut tab,
            &[
                Segment::Text("Light"),
                Segment::Text("Scene"),
                Segment::Text("Lens"),
            ],
        );
        for (name, rect) in ["light", "scene", "lens"].iter().zip(&choice.items) {
            hits.push((format!("sheet.{name}"), *rect));
        }
        ui.data_mut(|d| d.insert_temp(id, tab));
        Some(tab)
    } else {
        None
    };
    let show = |n: usize| shown.is_none_or(|tab| tab == n);
    if show(0) {
        light_section(ui, &kit, look, hits, sheet);
    }
    if show(1) {
        scene_section(ui, &kit, look, hits, sheet);
    }
    if show(2) {
        lens_rows(ui, &kit, look, lens, hits);
    }
    let _ = t;
}

/// The Light section.
fn light_section(
    ui: &mut egui::Ui,
    kit: &Kit,
    look: &mut LookState,
    hits: &mut LookHits,
    sheet: bool,
) {
    let kit = *kit;
    let t = kit.t;
    if !sheet {
        kit.section_header(ui, "Light");
    }
    choice_row(
        ui,
        &kit,
        "Environment",
        "environment",
        &ENVIRONMENTS,
        &mut look.environment,
        hits,
    );
    slider_row(
        ui,
        &kit,
        "Rotation",
        "rotation",
        &mut look.rotation,
        -180.0..=180.0,
        "°",
        hits,
    );
    slider_row(
        ui,
        &kit,
        "Exposure",
        "exposure",
        &mut look.exposure,
        -4.0..=4.0,
        " EV",
        hits,
    );
    choice_row(
        ui,
        &kit,
        "Tone curve",
        "tonemap",
        &TONEMAPS,
        &mut look.tonemap,
        hits,
    );
    row(ui, &kit, "Light", |ui| {
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                check(ui, &kit, "shadows", "Shadows", &mut look.shadows, hits);
                check(ui, &kit, "ao", "AO", &mut look.ao, hits);
            });
            check(
                ui,
                &kit,
                "reflections",
                "Reflections",
                &mut look.reflections,
                hits,
            );
        });
    });
    slider_row(
        ui,
        &kit,
        "Bloom",
        "bloom",
        &mut look.bloom,
        0.0..=1.0,
        "",
        hits,
    );
    if !sheet {
        ui.add_space(t.nav_spacing);
    }
}

/// The Scene section.
fn scene_section(
    ui: &mut egui::Ui,
    kit: &Kit,
    look: &mut LookState,
    hits: &mut LookHits,
    sheet: bool,
) {
    let kit = *kit;
    let t = kit.t;
    if !sheet {
        kit.section_header(ui, "Scene");
    }
    choice_row(
        ui,
        &kit,
        "Background",
        "background",
        &BACKGROUNDS,
        &mut look.background,
        hits,
    );
    row(ui, &kit, "Ground", |ui| {
        let response = kit.toggle(ui, &mut look.ground, "");
        hits.push(("ground".to_string(), response.rect));
    });
    let ground = look.ground;
    let text = format!("{:.2}", look.ground_reflection);
    let response = kit.slider(
        ui,
        Slider::new("Reflection", &mut look.ground_reflection, 0.0..=1.0)
            .text(text)
            .enabled(ground)
            .widths(t.popover_label_width, t.popover_value_width),
    );
    hits.push(("groundReflection".to_string(), response.rect));
    if !sheet {
        ui.add_space(t.nav_spacing);
    }
}

/// A row: the label in the dim ink, `popover_label_width` wide, then what
/// `add` lays out.
fn row<R>(ui: &mut egui::Ui, kit: &Kit, label: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.horizontal_top(|ui| {
        let galley = kit.galley(ui, label, kit.font(), kit.skin.text);
        let (place, _) = ui.allocate_exact_size(
            egui::vec2(kit.t.popover_label_width, kit.hit(kit.t.control_height)),
            egui::Sense::hover(),
        );
        ui.painter().galley(
            egui::pos2(place.left(), place.center().y - galley.size().y * 0.5),
            galley,
            color(kit.skin.text),
        );
        add(ui)
    })
    .inner
}

/// What `add` lays out under a row's control, in line with it.
fn indented<R>(ui: &mut egui::Ui, kit: &Kit, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.horizontal(|ui| {
        ui.add_space(kit.t.popover_label_width);
        add(ui)
    })
    .inner
}

/// A row of a label and a segmented choice of `options` (value, label).
fn choice_row(
    ui: &mut egui::Ui,
    kit: &Kit,
    label: &str,
    id: &str,
    options: &[(&str, &str)],
    value: &mut String,
    hits: &mut LookHits,
) {
    row(ui, kit, label, |ui| {
        let segments: Vec<Segment<'_>> = options
            .iter()
            .map(|(_, shown)| Segment::Text(shown))
            .collect();
        let mut chosen = options
            .iter()
            .position(|(name, _)| value == name)
            .unwrap_or(usize::MAX);
        let was = chosen;
        let choice = kit.segmented(ui, &mut chosen, &segments);
        for ((name, _), rect) in options.iter().zip(&choice.items) {
            hits.push((format!("{id}.{name}"), *rect));
        }
        if chosen != was
            && let Some((name, _)) = options.get(chosen)
        {
            *value = (*name).to_string();
        }
    });
}

/// A row of a label and a slider over `range`, its value written after
/// it. A value set outside the range (by `ui.set`) is kept until the
/// slider is moved.
#[allow(clippy::too_many_arguments)]
fn slider_row(
    ui: &mut egui::Ui,
    kit: &Kit,
    label: &str,
    id: &str,
    value: &mut f64,
    range: std::ops::RangeInclusive<f64>,
    unit: &str,
    hits: &mut LookHits,
) {
    let text = format!("{value:.2}{unit}");
    let response = kit.slider(
        ui,
        Slider::new(label, value, range)
            .text(text)
            .widths(kit.t.popover_label_width, kit.t.popover_value_width),
    );
    hits.push((id.to_string(), response.rect));
}

/// A check box of the kit, its place kept as `id`.
fn check(
    ui: &mut egui::Ui,
    kit: &Kit,
    id: &str,
    label: &str,
    value: &mut bool,
    hits: &mut LookHits,
) {
    let response = kit.checkbox(ui, value, label);
    hits.push((id.to_string(), response.rect));
}
