//! The built-in fonts: which files they are, where `cargo xtask fonts`
//! fetches them from, and the SHA-256 each must have.
//!
//! Font files are never committed (AGENTS.md): the installers carry them
//! (`packaging/`), the desktop app and the CLI find them at run time, and
//! the web page fetches them over HTTP when a text first uses one. The
//! single-stroke font is not a file: it is data in this crate
//! ([`crate::StrokeFont`]).

/// A built-in font file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BuiltinFont {
    /// What a text calls it (`font: "sans"`).
    pub name: &'static str,
    /// The font file's name where the fonts are kept.
    pub file: &'static str,
    pub url: &'static str,
    /// SHA-256 of the file, lowercase hexadecimal.
    pub sha256: &'static str,
    /// The licence file kept beside it.
    pub licence: &'static str,
    pub licence_url: &'static str,
    pub licence_sha256: &'static str,
}

/// The name of the single-stroke font, which has no file.
pub const STROKE: &str = "stroke";

/// Noto Sans Regular (CFF outlines: cubic) and JetBrains Mono Regular
/// (TrueType outlines: quadratic), both under the SIL Open Font Licence
/// 1.1, pinned to a commit and a release.
pub const BUILTIN: &[BuiltinFont] = &[
    BuiltinFont {
        name: "sans",
        file: "NotoSans-Regular.otf",
        url: "https://raw.githubusercontent.com/notofonts/notofonts.github.io/412a7edb2c8c535d1066157e5b17e661fd5591fe/fonts/NotoSans/unhinted/otf/NotoSans-Regular.otf",
        sha256: "bfb44de2c2031c391ff856943fe8f105ebb5d725cb9a04d493d211d5bbd5c21b",
        licence: "NotoSans-OFL.txt",
        licence_url: "https://raw.githubusercontent.com/notofonts/notofonts.github.io/412a7edb2c8c535d1066157e5b17e661fd5591fe/fonts/LICENSE",
        licence_sha256: "f2095b08bed08b23a6fe26112fcd679a2bee3f002eef077eb05d215ed1051bd8",
    },
    BuiltinFont {
        name: "mono",
        file: "JetBrainsMono-Regular.ttf",
        url: "https://raw.githubusercontent.com/JetBrains/JetBrainsMono/v2.304/fonts/ttf/JetBrainsMono-Regular.ttf",
        sha256: "a0bf60ef0f83c5ed4d7a75d45838548b1f6873372dfac88f71804491898d138f",
        licence: "JetBrainsMono-OFL.txt",
        licence_url: "https://raw.githubusercontent.com/JetBrains/JetBrainsMono/v2.304/OFL.txt",
        licence_sha256: "30f0c136e3c88e422d0791acd97238870f9054a9729bc34cf2ff0d4ed8cac4ad",
    },
];

/// The built-in font called `name`.
pub fn builtin(name: &str) -> Option<&'static BuiltinFont> {
    BUILTIN.iter().find(|f| f.name == name)
}
