//! The multisampled stages of a see-through and of a cut-open picture in
//! two forms (docs/design/rendering.md, *Per-sample stages on Vulkan*):
//!
//! - **masked** (`peel_samples.wgsl`, `cut_samples.wgsl`): a fragment is
//!   shaded once per pixel and decides with the sample mask on which of
//!   its samples it is drawn, from a loop over the samples of a
//!   multisampled texture. What every backend but Vulkan draws.
//! - **per sample** (`peel_samples_persample.wgsl`,
//!   `cut_samples_persample.wgsl`): no sample mask; the stages that wrote
//!   one run once per sample (`@builtin(sample_index)`), each sample reading
//!   its own texel and discarding itself. It needs per-sample shading
//!   (`DownlevelFlags::MULTISAMPLED_SHADING`).
//!
//! Why two: naga 30.0.1 writes a WGSL `@builtin(sample_mask)` fragment
//! output into SPIR-V as a scalar `Output` variable decorated `BuiltIn
//! SampleMask`, where Vulkan requires an array of 32-bit integers
//! (`spirv-val`: VUID-SampleMask-SampleMask-04359). Every stage of the
//! masked form that writes the mask is therefore invalid SPIR-V on every
//! Vulkan driver: Mesa takes it, Intel's and AMD's drivers on Windows crash
//! on it (2026-11-16 and 17). So on Vulkan the per-sample stages, on every
//! system ([`stages`]); a Vulkan device without per-sample shading draws
//! the viewport with one sample instead (no multisampled stage at all);
//! DirectX 12, OpenGL, Metal and the browser's WebGPU keep the masked
//! stages, byte for byte. `CRAFT3D_PEEL_VARIANT` (`diag::PEEL_VARIANT_VAR`)
//! forces either form: `today` (masked), `persample`, or `auto`, the
//! default.

use std::fmt;

/// The form of the multisampled see-through and section stages.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PeelVariant {
    /// Per pixel, with the sample mask: what every backend but Vulkan draws
    /// (`today`).
    #[default]
    Masked,
    /// Per sample, without a sample mask (`persample`): Vulkan's.
    PerSample,
}

impl fmt::Display for PeelVariant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            PeelVariant::Masked => "today",
            PeelVariant::PerSample => "persample",
        })
    }
}

/// Which form to draw with: one forced, or [`auto_choice`]'s for the
/// backend.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PeelSetting {
    #[default]
    Auto,
    Fixed(PeelVariant),
}

impl PeelSetting {
    /// The setting `value` names: `auto` (also an empty value), `today`
    /// or `persample`, blanks and case ignored; anything else is an error.
    pub fn parse(value: &str) -> Result<PeelSetting, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "" | "auto" => Ok(PeelSetting::Auto),
            "today" => Ok(PeelSetting::Fixed(PeelVariant::Masked)),
            "persample" => Ok(PeelSetting::Fixed(PeelVariant::PerSample)),
            _ => {
                let shown: String = value.trim().chars().take(40).collect();
                Err(format!("`{shown}` is not one of auto, today and persample"))
            }
        }
    }
}

/// What [`PeelSetting::Auto`] takes on `backend`, and why: the per-sample
/// stages on Vulkan, on every system and for every vendor (the masked ones
/// would be invalid SPIR-V there: naga writes their sample-mask output as a
/// scalar, module doc); the masked ones on every other backend, byte for
/// byte what they drew before.
pub fn auto_choice(backend: wgpu::Backend) -> (PeelVariant, &'static str) {
    if backend == wgpu::Backend::Vulkan {
        (
            PeelVariant::PerSample,
            "the per-sample stages on Vulkan: naga writes a sample-mask output as a scalar, \
             which Vulkan forbids (VUID-SampleMask-SampleMask-04359)",
        )
    } else {
        (PeelVariant::Masked, "the masked stages: not Vulkan")
    }
}

/// What the viewport draws its see-through and section stages with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stages {
    pub variant: PeelVariant,
    /// The samples per pixel: those asked for, or one where neither form
    /// can be drawn with several.
    pub samples: u32,
    pub why: &'static str,
}

/// The stages for `setting` on `backend` with `samples` per pixel, on a
/// device that can shade per sample or not (`shades_per_sample`). With one
/// sample there is no multisampled stage (the stages without a mask). The
/// per-sample stages without per-sample shading cannot be drawn: on Vulkan,
/// where the masked ones would be invalid SPIR-V, the viewport takes one
/// sample per pixel; elsewhere the masked ones. A form the person forces
/// (`CRAFT3D_PEEL_VARIANT`) is taken as it is where it can be drawn, also
/// the masked stages on Vulkan, for a comparison.
pub fn stages(
    setting: PeelSetting,
    backend: wgpu::Backend,
    samples: u32,
    shades_per_sample: bool,
) -> Stages {
    if samples <= 1 {
        return Stages {
            variant: PeelVariant::Masked,
            samples,
            why: "one sample per pixel: the stages without a mask",
        };
    }
    let (variant, why) = match setting {
        PeelSetting::Auto => auto_choice(backend),
        PeelSetting::Fixed(PeelVariant::Masked) => (
            PeelVariant::Masked,
            "the masked stages, as CRAFT3D_PEEL_VARIANT says",
        ),
        PeelSetting::Fixed(PeelVariant::PerSample) => (
            PeelVariant::PerSample,
            "the per-sample stages, as CRAFT3D_PEEL_VARIANT says",
        ),
    };
    if variant == PeelVariant::PerSample && !shades_per_sample {
        if backend == wgpu::Backend::Vulkan {
            return Stages {
                variant: PeelVariant::Masked,
                samples: 1,
                why: "one sample per pixel, since this Vulkan device cannot shade per sample \
                      and the masked stages would be invalid SPIR-V (naga writes a sample-mask \
                      output as a scalar, which Vulkan forbids: VUID-SampleMask-SampleMask-04359)",
            };
        }
        return Stages {
            variant: PeelVariant::Masked,
            samples,
            why: "the masked stages: no per-sample shading",
        };
    }
    Stages {
        variant,
        samples,
        why,
    }
}

/// The see-through's multisampled stages in the form `variant`, with the
/// file it is.
pub(crate) fn peel_samples(variant: PeelVariant) -> (&'static str, &'static str) {
    match variant {
        PeelVariant::Masked => (include_str!("peel_samples.wgsl"), "peel_samples.wgsl"),
        PeelVariant::PerSample => (
            include_str!("peel_samples_persample.wgsl"),
            "peel_samples_persample.wgsl",
        ),
    }
}

/// The section's multisampled stages in the form `variant`, with the file
/// it is.
pub(crate) fn cut_samples(variant: PeelVariant) -> (&'static str, &'static str) {
    match variant {
        PeelVariant::Masked => (include_str!("cut_samples.wgsl"), "cut_samples.wgsl"),
        PeelVariant::PerSample => (
            include_str!("cut_samples_persample.wgsl"),
            "cut_samples_persample.wgsl",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_setting_is_auto_today_or_persample_and_nothing_else() {
        assert_eq!(PeelSetting::parse(""), Ok(PeelSetting::Auto));
        assert_eq!(PeelSetting::parse(" Auto "), Ok(PeelSetting::Auto));
        assert_eq!(
            PeelSetting::parse("today"),
            Ok(PeelSetting::Fixed(PeelVariant::Masked))
        );
        assert_eq!(
            PeelSetting::parse("PERSAMPLE"),
            Ok(PeelSetting::Fixed(PeelVariant::PerSample))
        );
        for bad in [
            "unrolled",
            "nodiscard",
            "frontkey1",
            "today,persample",
            "masked",
            "\u{0}",
            "x".repeat(1 << 20).as_str(),
        ] {
            let said = PeelSetting::parse(bad).unwrap_err();
            assert!(said.len() < 120, "{}", said.len());
        }
        assert_eq!(PeelSetting::default(), PeelSetting::Auto);
        assert_eq!(PeelVariant::Masked.to_string(), "today");
        assert_eq!(PeelVariant::PerSample.to_string(), "persample");
    }

    /// The default: the per-sample stages on Vulkan on every system, the
    /// masked ones, byte for byte, on every other backend; on a Vulkan
    /// device without per-sample shading one sample per pixel; a forced
    /// form as it is where it can be drawn.
    #[test]
    fn the_stages_are_per_sample_on_vulkan_and_masked_elsewhere() {
        use wgpu::Backend::{BrowserWebGpu, Dx12, Gl, Metal, Vulkan};
        let (chosen, why) = auto_choice(Vulkan);
        assert_eq!(chosen, PeelVariant::PerSample);
        assert!(why.contains("VUID-SampleMask-SampleMask-04359"), "{why}");
        let auto = PeelSetting::Auto;
        let got = stages(auto, Vulkan, 4, true);
        assert_eq!((got.variant, got.samples), (PeelVariant::PerSample, 4));
        assert_eq!(peel_samples(got.variant).1, "peel_samples_persample.wgsl");
        assert_eq!(cut_samples(got.variant).1, "cut_samples_persample.wgsl");
        for backend in [Dx12, Gl, Metal, BrowserWebGpu] {
            let got = stages(auto, backend, 4, true);
            assert_eq!(
                (got.variant, got.samples),
                (PeelVariant::Masked, 4),
                "{backend:?}"
            );
            assert!(got.why.starts_with("the masked stages"), "{}", got.why);
            assert_eq!(
                peel_samples(got.variant).0,
                include_str!("peel_samples.wgsl")
            );
            assert_eq!(cut_samples(got.variant).0, include_str!("cut_samples.wgsl"));
            // Without per-sample shading the same: the masked stages.
            let got = stages(auto, backend, 4, false);
            assert_eq!(
                (got.variant, got.samples),
                (PeelVariant::Masked, 4),
                "{backend:?}"
            );
        }
        // A Vulkan device that cannot shade per sample: one sample per
        // pixel, never the masked stages' invalid SPIR-V.
        let got = stages(auto, Vulkan, 4, false);
        assert_eq!(got.samples, 1);
        assert!(got.why.starts_with("one sample per pixel"), "{}", got.why);
        let forced = PeelSetting::Fixed(PeelVariant::PerSample);
        assert_eq!(stages(forced, Vulkan, 4, false).samples, 1);
        let got = stages(forced, Dx12, 4, false);
        assert_eq!((got.variant, got.samples), (PeelVariant::Masked, 4));
        let got = stages(forced, Dx12, 4, true);
        assert_eq!((got.variant, got.samples), (PeelVariant::PerSample, 4));
        // The masked stages forced on Vulkan, for a comparison.
        let today = PeelSetting::Fixed(PeelVariant::Masked);
        let got = stages(today, Vulkan, 4, true);
        assert_eq!((got.variant, got.samples), (PeelVariant::Masked, 4));
        // One sample: no multisampled stage, whatever is asked.
        for setting in [auto, forced, today] {
            for backend in [Vulkan, Dx12, Gl] {
                let got = stages(setting, backend, 1, false);
                assert_eq!((got.variant, got.samples), (PeelVariant::Masked, 1));
            }
        }
    }

    /// The per-sample files have what the masked ones have, without the
    /// mask: the same entry points, each per sample where the masked one
    /// wrote the mask, and nothing that writes one.
    #[test]
    fn the_per_sample_stages_are_the_masked_ones_without_the_mask() {
        for (masked, per_sample) in [
            (
                peel_samples(PeelVariant::Masked).0,
                peel_samples(PeelVariant::PerSample).0,
            ),
            (
                cut_samples(PeelVariant::Masked).0,
                cut_samples(PeelVariant::PerSample).0,
            ),
        ] {
            let entries = |source: &str| -> Vec<String> {
                source
                    .split("@fragment")
                    .skip(1)
                    .filter_map(|rest| {
                        let after = rest.split("fn ").nth(1)?;
                        Some(after.chars().take_while(|c| *c != '(').collect())
                    })
                    .collect()
            };
            assert_eq!(entries(masked), entries(per_sample));
            assert!(masked.contains("@builtin(sample_mask)"));
            assert!(!per_sample.contains("sample_mask"));
            assert!(!per_sample.contains("textureNumSamples"));
            assert!(per_sample.contains("@builtin(sample_index)"));
        }
    }

    /// The fragment entry points of `source` whose output writes the sample
    /// mask (`@builtin(sample_mask)` on the result, or on a member of the
    /// result's struct), as naga's front end reads them.
    fn writes_a_sample_mask(label: &str, source: &str) -> Vec<String> {
        use wgpu::naga;
        let module = naga::front::wgsl::parse_str(source)
            .unwrap_or_else(|e| panic!("{label}: {}", e.emit_to_string(source)));
        let mask = |binding: &Option<naga::Binding>| {
            matches!(
                binding,
                Some(naga::Binding::BuiltIn(naga::BuiltIn::SampleMask))
            )
        };
        let mut names: Vec<String> = module
            .entry_points
            .iter()
            .filter(|e| e.stage == naga::ShaderStage::Fragment)
            .filter(|e| {
                e.function.result.as_ref().is_some_and(|result| {
                    mask(&result.binding)
                        || match &module.types[result.ty].inner {
                            naga::TypeInner::Struct { members, .. } => {
                                members.iter().any(|m| mask(&m.binding))
                            }
                            _ => false,
                        }
                })
            })
            .map(|e| e.name.clone())
            .collect();
        names.sort();
        names
    }

    /// Every module the on-screen and off-screen renderers build, as they
    /// build it on Vulkan by the default rule ([`stages`]), for one sample
    /// per pixel and for several, on a device that shades per sample and on
    /// one that does not: no fragment stage writes the sample mask, which
    /// naga hands Vulkan as invalid SPIR-V (the module doc). The masked
    /// forms, which the other backends draw, write it in the six stages
    /// `spirv-val` refused (2026-11-16): what shows the check sees it.
    #[test]
    fn no_stage_drawn_on_vulkan_writes_a_sample_mask() {
        use crate::renderer::scene_source_for;
        // Vulkan: the fragment does not write the depth (`fragment_depth`).
        let depth = false;
        let see_through = |multisampled: bool, variant: PeelVariant| {
            scene_source_for(
                depth,
                false,
                false,
                &[crate::peel::scene_part(multisampled, variant).0],
            )
        };
        for asked in [1, 4] {
            for shades in [true, false] {
                let chosen = stages(
                    PeelSetting::Auto,
                    wgpu::Backend::Vulkan,
                    asked,
                    asked > 1 && shades,
                );
                let multisampled = chosen.samples > 1;
                let per_sample = multisampled && shades;
                let overlay = crate::onscreen::overlay_sources(chosen.samples);
                let studio = [include_str!("studio.wgsl"), include_str!("polish.wgsl")];
                let modules = [
                    ("scene", scene_source_for(depth, false, false, &[])),
                    ("studio", scene_source_for(depth, false, false, &studio)),
                    ("see-through", see_through(multisampled, chosen.variant)),
                    (
                        "cut open",
                        crate::cut::source(depth, multisampled, per_sample, chosen.variant),
                    ),
                    ("blend", crate::peel::blend_source(multisampled)),
                    ("lines", overlay.lines),
                    ("fills", overlay.fills),
                    ("distance", overlay.distance),
                    ("depth read", overlay.depth_read),
                    ("edge tiles", crate::onscreen::EDGE_TILES_SOURCE.to_string()),
                    ("resolve", include_str!("resolve.wgsl").to_string()),
                    ("present", include_str!("present.wgsl").to_string()),
                    ("accumulate", include_str!("accumulate.wgsl").to_string()),
                ];
                for (name, source) in &modules {
                    let label = format!(
                        "{name}: {asked} samples asked, per-sample shading {shades}; drawn \
                         with {} and the {} stages",
                        chosen.samples, chosen.variant
                    );
                    assert_eq!(
                        writes_a_sample_mask(&label, source),
                        Vec::<String>::new(),
                        "{label}"
                    );
                }
            }
        }
        let masked = PeelVariant::Masked;
        assert_eq!(
            writes_a_sample_mask("see-through, masked", &see_through(true, masked)),
            ["fs_behind_samples"]
        );
        assert_eq!(
            writes_a_sample_mask(
                "cut open, masked",
                &crate::cut::source(depth, true, true, masked)
            ),
            [
                "fs_behind_samples",
                "fs_behind_samples_cut",
                "fs_cap_behind_samples",
                "fs_cap_front_samples",
                "fs_cap_samples",
            ]
        );
    }
}
