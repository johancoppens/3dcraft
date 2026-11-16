//! The multisampled stages of a see-through and of a cut-open picture in
//! two forms (docs/design/rendering.md, *Per-sample stages on Intel's
//! Vulkan driver*):
//!
//! - **masked** (`peel_samples.wgsl`, `cut_samples.wgsl`): a fragment is
//!   shaded once per pixel and decides with the sample mask on which of
//!   its samples it is drawn, from a loop over the samples of a
//!   multisampled texture. What every adapter draws, unless it is one of
//!   the next.
//! - **per sample** (`peel_samples_persample.wgsl`,
//!   `cut_samples_persample.wgsl`): no sample mask; the stages that wrote
//!   one run once per sample (`@builtin(sample_index)`), each sample reading
//!   its own texel and discarding itself. Intel's Vulkan driver on Windows
//!   crashes in its shader compiler (`igc64.dll`) on every stage that writes
//!   the sample mask, whatever feeds it (measured 2026-11-16 on UHD
//!   Graphics, driver 32.0.101.7077), and draws these right. It needs
//!   per-sample shading (`DownlevelFlags::MULTISAMPLED_SHADING`).
//!
//! Which form an adapter gets is [`auto_choice`]'s: by vendor, backend
//! and platform, not everywhere, since the per-sample picture is not bit
//! for bit the masked one on every driver (it is on Mesa's Intel driver;
//! llvmpipe differs in a few pixels). `CRAFT3D_PEEL_VARIANT`
//! (`diag::PEEL_VARIANT_VAR`) forces either: `today` (masked), `persample`,
//! or `auto`, the default.

use std::fmt;

/// The form of the multisampled see-through and section stages.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PeelVariant {
    /// Per pixel, with the sample mask: what every adapter draws unless it
    /// needs the other (`today`).
    #[default]
    Masked,
    /// Per sample, without a sample mask (`persample`).
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
/// adapter.
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

/// What [`PeelSetting::Auto`] takes on an adapter of `vendor` on `backend`,
/// on Windows or not (`windows`), with per-sample shading or not, and why:
/// the per-sample stages on Intel's Vulkan driver on Windows, whose shader
/// compiler crashes on a stage that writes the sample mask; the masked ones
/// everywhere else.
pub fn auto_choice(
    vendor: u32,
    backend: wgpu::Backend,
    windows: bool,
    per_sample: bool,
) -> (PeelVariant, &'static str) {
    if !(vendor == INTEL && backend == wgpu::Backend::Vulkan && windows) {
        (
            PeelVariant::Masked,
            "the masked stages: not Intel's Vulkan driver on Windows",
        )
    } else if !per_sample {
        (
            PeelVariant::Masked,
            "the masked stages: Intel's Vulkan driver on Windows, but no per-sample shading",
        )
    } else {
        (
            PeelVariant::PerSample,
            "the per-sample stages: Intel's Vulkan driver on Windows crashes on a stage that \
             writes the sample mask",
        )
    }
}

/// Intel's PCI vendor id.
const INTEL: u32 = 0x8086;

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

    /// The default draws today's stages, byte for byte, on every adapter
    /// but Intel's Vulkan driver on Windows with per-sample shading.
    #[test]
    fn auto_takes_the_per_sample_stages_on_intels_vulkan_driver_on_windows_only() {
        use wgpu::Backend::{BrowserWebGpu, Dx12, Gl, Metal, Vulkan};
        let (chosen, why) = auto_choice(0x8086, Vulkan, true, true);
        assert_eq!(chosen, PeelVariant::PerSample, "{why}");
        assert_eq!(peel_samples(chosen).1, "peel_samples_persample.wgsl");
        for (vendor, backend, windows, per_sample) in [
            (0x8086, Vulkan, true, false),
            (0x8086, Vulkan, false, true),
            (0x8086, Dx12, true, true),
            (0x8086, Gl, true, true),
            (0x8086, Metal, false, true),
            (0x8086, BrowserWebGpu, true, true),
            (0x10de, Vulkan, true, true),
            (0x1002, Vulkan, true, true),
            (0x106b, Metal, false, true),
            (0, Vulkan, true, true),
        ] {
            let (chosen, why) = auto_choice(vendor, backend, windows, per_sample);
            assert_eq!(
                chosen,
                PeelVariant::Masked,
                "{vendor:#x} {backend:?} {windows}"
            );
            assert!(why.starts_with("the masked stages"), "{why}");
            assert_eq!(peel_samples(chosen).0, include_str!("peel_samples.wgsl"));
            assert_eq!(cut_samples(chosen).0, include_str!("cut_samples.wgsl"));
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
}
