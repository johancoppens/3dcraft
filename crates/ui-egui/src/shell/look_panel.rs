//! The studio look's options, in the Studio chip's popover
//! (`shell/popover.rs`) while the chip is on (and no analysis, which is
//! drawn in the plain look, is shown): the environment and its rotation, the exposure, the
//! background, the ground and its reflection, the tone curve, the
//! shadows, ambient occlusion and reflections, bloom and a lens. Every
//! control drives a field of `state.look` (the fields `ui.set {"look":
//! ...}` takes and `ui.get` gives), so a change is drawn at once and an
//! agent sees it.

use super::Shell;
use crate::state::LookState;
use crate::theme::Tokens;
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
const FSTOP: f64 = 2.8;

/// Where each control of the panel was drawn, by name: `environment.<v>`,
/// `background.<v>` and `tonemap.<v>` for the choices, `rotation`,
/// `exposure`, `groundReflection` and `bloom` for the sliders (their
/// track only), `ground`, `shadows`, `ao`, `reflections` and `lens`
/// for the toggles, `collapse` for the header that folds the panel, and
/// `panel` for the whole panel.
pub type LookHits = Vec<(String, Rect)>;

impl Shell {
    /// Draws the panel as the Studio chip's popover at `place`
    /// ([`super::popover_place`]) while the studio look is on.
    pub(super) fn look_panel(&mut self, ui: &mut egui::Ui, place: Option<Rect>) {
        let Some(place) = place else {
            self.look_hits.clear();
            return;
        };
        if !self.state.look.is_studio() || self.state.analysis.is_on() {
            self.look_hits.clear();
            return;
        }
        let tokens = self.tokens;
        let before = self.state.look.clone();
        let mut hits = LookHits::new();
        let mut open = self.look_panel_open;
        let drawn = self.popover(
            ui,
            "studio-options",
            "Studio look",
            place,
            &mut open,
            |shell, ui| look_options(ui, &mut shell.state.look, &tokens, &mut hits),
        );
        self.look_panel_open = open;
        hits.push(("collapse".to_string(), drawn.header));
        hits.push(("panel".to_string(), drawn.panel));
        self.look_hits = hits;
        if self.state.look != before {
            ui.ctx().request_repaint();
        }
    }

    /// Whether the studio look's panel is unfolded (its header folds it).
    pub fn look_panel_open(&self) -> bool {
        self.look_panel_open
    }

    /// Where the controls of the studio look's panel are, in points, once
    /// it was drawn ([`LookHits`]); empty while it is not shown.
    pub fn look_hits(&self) -> &[(String, Rect)] {
        &self.look_hits
    }

    /// Whether the Analysis popover's options are unfolded.
    pub fn analysis_panel_open(&self) -> bool {
        self.analysis_panel_open
    }

    /// Where the Analysis popover's header (`collapse`) and whole panel
    /// (`panel`) were drawn, in points; empty while it is not shown.
    pub fn analysis_hits(&self) -> &[(String, Rect)] {
        &self.analysis_hits
    }
}

/// The controls, in a grid of a label and a control a row.
fn look_options(ui: &mut egui::Ui, look: &mut LookState, tokens: &Tokens, hits: &mut LookHits) {
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
                let response = ui.add_enabled(
                    look.ground,
                    egui::Slider::new(&mut look.ground_reflection, 0.0..=1.0)
                        .clamping(egui::SliderClamping::Edits)
                        .show_value(false),
                );
                hits.push(("groundReflection".to_string(), response.rect));
                ui.label(format!("reflection {:.2}", look.ground_reflection));
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
            ui.label("Lens");
            let mut lens = look.dof.is_some();
            ui.horizontal(|ui| {
                toggle(ui, "lens", "Depth of field", &mut lens, hits);
            });
            if lens != look.dof.is_some() {
                look.dof = lens.then(|| serde_json::json!({ "fstop": FSTOP }));
            }
            ui.end_row();
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
