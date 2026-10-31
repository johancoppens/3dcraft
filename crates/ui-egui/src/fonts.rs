//! The interface's fonts (docs/design/rendering.md › *The interface's
//! font*): Inter for text, at its regular weight and at a medium weight
//! for headings ([`MEDIUM`]), and JetBrains Mono (the built-in `mono`) for
//! monospace text. Neither is in the program: on the desktop they are read
//! where the installers put the built-in fonts (`craft3d_engine::fonts`),
//! in the browser the page fetches Inter beside itself and the shell swaps
//! it in when it arrives. Until then, and wherever a font is missing,
//! egui's own fonts are used, so the interface always has text; they stay
//! behind Inter as the fallback for the few marks it lacks (✔ ⏸ ⌖).

use std::sync::Arc;

use egui::epaint::text::VariationCoords;
use egui::{FontData, FontDefinitions, FontFamily, FontTweak};

/// The name of the interface's text font (`craft3d_font::UI_FONTS`).
pub const TEXT: &str = "inter";
/// The name of the monospace font (the built-in `mono`).
pub const MONO: &str = "mono";
/// The family of the medium weight, which headings use.
pub const MEDIUM: &str = "medium";
/// Inter's weight for running text and for the medium family.
const WEIGHT_REGULAR: f32 = 400.0;
const WEIGHT_MEDIUM: f32 = 500.0;

/// Which of the interface's fonts are in the context's definitions.
#[derive(Debug, Default)]
pub struct UiFonts {
    asked: bool,
    installed: [bool; 2],
    /// Whether headings are in the family [`MEDIUM`] now.
    medium_headings: bool,
}

impl UiFonts {
    /// Asks for the fonts once and puts in those that are here, again
    /// whenever one more arrives (in the browser, a fetch that ended).
    /// Cheap when nothing changed: a few lookups. Called once per frame
    /// with the pass's root `ui`, before anything is drawn in it.
    pub fn update(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if !self.asked {
            self.asked = true;
            // On the desktop this reads the files; in the browser it lists
            // Inter for the page to fetch. Monospace is fetched there only
            // when a text wants it: the interface uses it when it is here.
            // A font that is not here leaves egui's own in place; on the
            // desktop that is said once (the page says it for the web).
            #[cfg(not(target_arch = "wasm32"))]
            for name in [TEXT, MONO] {
                if let Err(why) = craft3d_engine::fonts::load(name) {
                    log::warn!("the interface keeps egui's font: {why}");
                }
            }
            #[cfg(target_arch = "wasm32")]
            let _ = craft3d_engine::fonts::load(TEXT);
        }
        // Headings use the family [`MEDIUM`] only while the fonts in force
        // have it: new definitions take effect at the next pass, and a
        // family that is not bound would stop egui.
        let medium = ctx.fonts(|f| {
            f.definitions()
                .families
                .contains_key(&FontFamily::Name(Arc::from(MEDIUM)))
        });
        if medium != self.medium_headings {
            self.medium_headings = medium;
            ctx.all_styles_mut(|style| style_headings(style, medium));
        }
        // The root `ui` took its style before this pass's fonts were known.
        let heading = if medium {
            FontFamily::Name(Arc::from(MEDIUM))
        } else {
            FontFamily::Proportional
        };
        let root = ui.style().text_styles.get(&egui::TextStyle::Heading);
        if root.is_some_and(|f| f.family != heading) {
            style_headings(ui.style_mut(), medium);
        }
        let have = [TEXT, MONO].map(craft3d_engine::fonts::ready);
        if have == self.installed {
            return;
        }
        self.installed = have;
        let text = have[0]
            .then(|| craft3d_engine::fonts::load(TEXT).ok())
            .flatten();
        let mono = have[1]
            .then(|| craft3d_engine::fonts::load(MONO).ok())
            .flatten();
        ctx.set_fonts(definitions(text.as_deref(), mono.as_deref()));
        // New fonts take effect at the next pass: this one is laid out
        // again at once, so no frame (and no test that reads where things
        // are after one frame) sees the old fonts' layout once the new ones
        // are here.
        ctx.request_discard("the interface's fonts arrived");
    }
}

fn weighted(bytes: &[u8], weight: f32) -> FontData {
    FontData::from_owned(bytes.to_vec()).tweak(FontTweak {
        coords: VariationCoords::new([(b"wght", weight)]),
        ..FontTweak::default()
    })
}

/// egui's fonts with Inter (regular, and medium as the family
/// [`MEDIUM`]) and JetBrains Mono put first where they are given; the
/// family [`MEDIUM`] exists either way (egui's text font when Inter is
/// missing), so a heading always has a font.
pub fn definitions(text: Option<&[u8]>, mono: Option<&[u8]>) -> FontDefinitions {
    let mut defs = FontDefinitions::default();
    let fallback = defs
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    defs.families
        .insert(FontFamily::Name(Arc::from(MEDIUM)), fallback);
    if let Some(bytes) = text {
        put(
            &mut defs,
            "Inter",
            weighted(bytes, WEIGHT_REGULAR),
            &[
                (FontFamily::Proportional, true),
                // Behind the monospace font: the marks it lacks.
                (FontFamily::Monospace, false),
            ],
        );
        put(
            &mut defs,
            "Inter Medium",
            weighted(bytes, WEIGHT_MEDIUM),
            &[(FontFamily::Name(Arc::from(MEDIUM)), true)],
        );
    }
    if let Some(bytes) = mono {
        put(
            &mut defs,
            "JetBrains Mono",
            FontData::from_owned(bytes.to_vec()),
            &[(FontFamily::Monospace, true)],
        );
    }
    defs
}

/// Adds font `name` to `families`, first (`true`) or last in each.
fn put(defs: &mut FontDefinitions, name: &str, data: FontData, families: &[(FontFamily, bool)]) {
    defs.font_data.insert(name.to_owned(), Arc::new(data));
    for (family, first) in families {
        let list = defs.families.entry(family.clone()).or_default();
        if *first {
            list.insert(0, name.to_owned());
        } else {
            list.push(name.to_owned());
        }
    }
}

/// Headings in the medium weight (`medium`), or in egui's proportional
/// family; egui's text styles otherwise as they are.
pub fn style_headings(style: &mut egui::Style, medium: bool) {
    if let Some(font) = style.text_styles.get_mut(&egui::TextStyle::Heading) {
        font.family = if medium {
            FontFamily::Name(Arc::from(MEDIUM))
        } else {
            FontFamily::Proportional
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(ctx: &egui::Context, fonts: &mut UiFonts) {
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            fonts.update(ui);
            ui.heading("Export");
            ui.label("text");
        });
        out.textures_delta.clear();
    }

    #[test]
    fn a_heading_never_names_a_family_the_fonts_in_force_lack() {
        // Fonts set in a pass take effect at the next one: the headings
        // follow the fonts in force, with the files here or not.
        let ctx = egui::Context::default();
        let mut fonts = UiFonts::default();
        for _ in 0..3 {
            frame(&ctx, &mut fonts);
        }
        // Definitions with the family `medium` (here without Inter's
        // bytes: the families are what matters), then egui's own fonts put
        // back by someone else: no pass names a family that is not bound.
        ctx.set_fonts(definitions(None, None));
        frame(&ctx, &mut fonts);
        frame(&ctx, &mut fonts);
        assert!(fonts.medium_headings);
        ctx.set_fonts(FontDefinitions::default());
        frame(&ctx, &mut fonts);
        assert!(!fonts.medium_headings);
        frame(&ctx, &mut fonts);
    }

    #[test]
    fn without_the_files_egui_fonts_remain_and_the_medium_family_exists() {
        let defs = definitions(None, None);
        let base = FontDefinitions::default();
        assert_eq!(defs.font_data.len(), base.font_data.len());
        let medium = defs.families.get(&FontFamily::Name(Arc::from(MEDIUM)));
        assert_eq!(medium, base.families.get(&FontFamily::Proportional));
        assert!(medium.is_some_and(|m| !m.is_empty()));
    }

    #[test]
    fn inter_goes_first_with_egui_fonts_behind_it() {
        // Any bytes do for the order; the files are checked where loaded.
        let defs = definitions(Some(b"inter"), Some(b"mono"));
        let first = |f: FontFamily| defs.families.get(&f).and_then(|l| l.first().cloned());
        assert_eq!(first(FontFamily::Proportional).as_deref(), Some("Inter"));
        assert_eq!(
            first(FontFamily::Monospace).as_deref(),
            Some("JetBrains Mono")
        );
        assert_eq!(
            first(FontFamily::Name(Arc::from(MEDIUM))).as_deref(),
            Some("Inter Medium")
        );
        // egui's fonts stay behind as the fallback (✔ ⏸ ⌖ are not in Inter).
        let prop = defs
            .families
            .get(&FontFamily::Proportional)
            .cloned()
            .unwrap();
        assert!(prop.len() > 1);
        let mono = defs.families.get(&FontFamily::Monospace).cloned().unwrap();
        assert_eq!(mono.last().map(String::as_str), Some("Inter"));
    }
}
