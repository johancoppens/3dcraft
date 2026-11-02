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
use crate::state::LookState;
use crate::theme::Tokens;
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
        let panel = self.popover(ui, "studio-options", place, chip, |shell, ui| {
            look_options(ui, &mut shell.state.look, &tokens, &lens, &mut hits);
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

/// The lens's rows: switched on and off, its f-number, where it focuses.
/// Its notes are the secondary text of the plates ([`Tokens::plate_secondary`]).
fn lens_rows(
    ui: &mut egui::Ui,
    look: &mut LookState,
    tokens: &Tokens,
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
    ui.label("Lens");
    let mut on = look.dof.is_some();
    ui.horizontal(|ui| {
        toggle(ui, "lens", "Depth of field", &mut on, hits);
    });
    ui.end_row();
    if on != look.dof.is_some() {
        look.dof = on.then(|| serde_json::json!({ "fstop": FSTOP, "focus": "selection" }));
        return;
    }
    // An orthographic view has no lens: nothing to set.
    let settable = on && !lens.orthographic;
    let mut fstop = dof.map_or(FSTOP, |d| d.fstop);
    let mut focus = dof.map_or(FocusSpec::Centre, |d| d.focus);
    let mut changed = false;
    ui.label("Aperture");
    ui.add_enabled_ui(settable, |ui| {
        ui.horizontal(|ui| {
            let mut stop = nearest_stop(fstop) as f64;
            let last = (FSTOPS.len() - 1) as f64;
            let response = ui
                .add(
                    egui::Slider::new(&mut stop, 0.0..=last)
                        .step_by(1.0)
                        .show_value(false),
                )
                .on_hover_text(
                    "The f-number: a smaller one opens the lens wider and blurs more of what is out of focus",
                );
            hits.push(("fstop".to_string(), response.rect));
            if response.changed()
                && let Some(f) = FSTOPS.get(stop.round().clamp(0.0, last) as usize)
            {
                fstop = *f;
                changed = true;
            }
            ui.label(fstop_text(fstop));
        });
    });
    ui.end_row();
    // Which way blurs more, under the slider: after its value it would
    // not fit the popover's width.
    ui.label("");
    let hint = ui.label(tokens.plate_secondary(APERTURE_HINT));
    hits.push(("apertureHint".to_string(), hint.rect));
    ui.end_row();
    ui.label("Focus");
    ui.add_enabled_ui(settable, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = tokens.nav_spacing;
            for (name, shown) in FOCUSES {
                let chosen = matches!(
                    (name, focus),
                    ("selection", FocusSpec::Selection)
                        | ("picked", FocusSpec::Picked)
                        | ("centre", FocusSpec::Centre)
                        | ("distance", FocusSpec::Distance(_))
                );
                let tip = match name {
                    "selection" => "Focus on what is selected, in the viewport or the feature list",
                    "picked" => "Focus on the point of the last click in the viewport",
                    "centre" => "Focus on the centre of the model",
                    _ => "Focus at a distance along the view",
                };
                let response = ui.selectable_label(chosen, shown).on_hover_text(tip);
                if response.clicked() && !chosen {
                    focus = match name {
                        "selection" => FocusSpec::Selection,
                        "picked" => FocusSpec::Picked,
                        "centre" => FocusSpec::Centre,
                        // From where it focuses now, so the picture
                        // does not jump.
                        _ => FocusSpec::Distance(
                            lens.distance
                                .filter(|d| d.is_finite() && *d > 0.0)
                                .unwrap_or(lens.camera_distance),
                        ),
                    };
                    changed = true;
                }
                hits.push((format!("focus.{name}"), response.rect));
            }
        });
    });
    ui.end_row();
    if let FocusSpec::Distance(mut d) = focus {
        ui.label("");
        ui.add_enabled_ui(settable, |ui| {
            ui.horizontal(|ui| {
                let scale = lens.camera_distance.max(f64::MIN_POSITIVE);
                let range = scale * DISTANCE_RANGE[0]..=scale * DISTANCE_RANGE[1];
                let response = ui.add(
                    egui::Slider::new(&mut d, range)
                        .logarithmic(true)
                        .clamping(egui::SliderClamping::Edits)
                        .show_value(false),
                );
                hits.push(("focusDistance".to_string(), response.rect));
                if response.changed() && d.is_finite() && d > 0.0 {
                    focus = FocusSpec::Distance(d);
                    changed = true;
                }
                ui.label(significant(d));
            });
        });
        ui.end_row();
    }
    // What the lens does instead of what it was asked: no blur at all
    // first, then where it focuses instead.
    let note = if !on {
        None
    } else if let Some(why) = lens.no_blur {
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
        ui.label("");
        let response = ui.add(egui::Label::new(tokens.plate_secondary(note)).wrap());
        hits.push(("lensNote".to_string(), response.rect));
        ui.end_row();
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

/// The controls, in a grid of a label and a control a row.
fn look_options(
    ui: &mut egui::Ui,
    look: &mut LookState,
    tokens: &Tokens,
    lens: &LensContext,
    hits: &mut LookHits,
) {
    // One width for every slider, from the tokens.
    ui.spacing_mut().slider_width = tokens.popover_slider_width;
    egui::Grid::new("studio-options-grid")
        .num_columns(2)
        .spacing([tokens.nav_spacing, tokens.nav_inset * 0.5])
        .show(ui, |ui| {
            choice_row(
                ui,
                tokens.nav_spacing,
                "Environment",
                "environment",
                &ENVIRONMENTS,
                &mut look.environment,
                hits,
            );
            slider_row(
                ui,
                "Rotation",
                "rotation",
                &mut look.rotation,
                -180.0..=180.0,
                "°",
                hits,
            );
            slider_row(
                ui,
                "Exposure",
                "exposure",
                &mut look.exposure,
                -4.0..=4.0,
                " EV",
                hits,
            );
            choice_row(
                ui,
                tokens.nav_spacing,
                "Background",
                "background",
                &BACKGROUNDS,
                &mut look.background,
                hits,
            );
            ui.label("Ground");
            ui.horizontal(|ui| {
                toggle(ui, "ground", "On", &mut look.ground, hits);
            });
            ui.end_row();
            ui.label("Reflection");
            ui.add_enabled_ui(look.ground, |ui| {
                ui.horizontal(|ui| {
                    let response = ui.add(
                        egui::Slider::new(&mut look.ground_reflection, 0.0..=1.0)
                            .clamping(egui::SliderClamping::Edits)
                            .show_value(false),
                    );
                    hits.push(("groundReflection".to_string(), response.rect));
                    ui.label(format!("{:.2}", look.ground_reflection));
                });
            });
            ui.end_row();
            choice_row(
                ui,
                tokens.nav_spacing,
                "Tone curve",
                "tonemap",
                &TONEMAPS,
                &mut look.tonemap,
                hits,
            );
            ui.label("Light");
            ui.horizontal(|ui| {
                toggle(ui, "shadows", "Shadows", &mut look.shadows, hits);
                toggle(ui, "ao", "AO", &mut look.ao, hits);
                toggle(
                    ui,
                    "reflections",
                    "Reflections",
                    &mut look.reflections,
                    hits,
                );
            });
            ui.end_row();
            slider_row(ui, "Bloom", "bloom", &mut look.bloom, 0.0..=1.0, "", hits);
            lens_rows(ui, look, tokens, lens, hits);
        });
}

/// A row of a label and a segmented choice of `options` (value, label).
fn choice_row(
    ui: &mut egui::Ui,
    gap: f32,
    label: &str,
    id: &str,
    options: &[(&str, &str)],
    value: &mut String,
    hits: &mut LookHits,
) {
    ui.label(label);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        for (name, shown) in options {
            let response = ui.selectable_label(value == name, *shown);
            if response.clicked() {
                *value = (*name).to_string();
            }
            hits.push((format!("{id}.{name}"), response.rect));
        }
    });
    ui.end_row();
}

/// A row of a label and a slider over `range`, its value written after
/// it. A value set outside the range (by `ui.set`) is kept until the
/// slider is moved.
fn slider_row(
    ui: &mut egui::Ui,
    label: &str,
    id: &str,
    value: &mut f64,
    range: std::ops::RangeInclusive<f64>,
    unit: &str,
    hits: &mut LookHits,
) {
    ui.label(label);
    ui.horizontal(|ui| {
        let response = ui.add(
            egui::Slider::new(value, range)
                .clamping(egui::SliderClamping::Edits)
                .show_value(false),
        );
        hits.push((id.to_string(), response.rect));
        ui.label(format!("{value:.2}{unit}"));
    });
    ui.end_row();
}

fn toggle(ui: &mut egui::Ui, id: &str, label: &str, value: &mut bool, hits: &mut LookHits) {
    let response = ui.checkbox(value, label);
    hits.push((id.to_string(), response.rect));
}
