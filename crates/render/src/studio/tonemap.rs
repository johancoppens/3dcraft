//! From scene light to display values: exposure, a tone curve and the
//! sRGB encoding (IEC 61966-2-1). Shading is linear throughout; colours
//! from the document (sRGB bytes) are decoded first.

/// The tone curve.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Tonemap {
    /// Troy Sobotka's AgX, as described in B. Wrensch, *Minimal AgX
    /// Implementation* (2023): an inset matrix, a log2 encoding over
    /// 16.5 stops and a sixth-order fit of the sigmoid, then the outset
    /// matrix. Bright colours desaturate towards white instead of
    /// clipping to a hue. The default. The snippet's last `pow(x, 2.2)`
    /// and the framebuffer's sRGB encoding after it are both skipped: the
    /// outset value is the display value (docs/design/rendering.md,
    /// *Sources*: lighter darks than with the two steps).
    #[default]
    Agx,
    /// The ACES filmic curve as fitted by K. Narkowicz, *ACES Filmic Tone
    /// Mapping Curve* (2016), then the sRGB encoding.
    Aces,
    /// No curve: clamped to 0..1, then the sRGB encoding (for the tests
    /// of the light itself).
    Linear,
}

impl Tonemap {
    /// The name `ui.set` and `view.render` use.
    pub fn name(self) -> &'static str {
        match self {
            Tonemap::Agx => "agx",
            Tonemap::Aces => "aces",
            Tonemap::Linear => "linear",
        }
    }

    pub fn from_name(name: &str) -> Option<Tonemap> {
        match name {
            "agx" => Some(Tonemap::Agx),
            "aces" => Some(Tonemap::Aces),
            "linear" => Some(Tonemap::Linear),
            _ => None,
        }
    }

    /// The number the GPU's uniform block carries.
    pub fn index(self) -> u32 {
        match self {
            Tonemap::Agx => 0,
            Tonemap::Aces => 1,
            Tonemap::Linear => 2,
        }
    }
}

/// The sRGB transfer function, linear to encoded, both in 0..1.
pub fn srgb_encode(c: f64) -> f64 {
    let c = if c.is_finite() {
        c.clamp(0.0, 1.0)
    } else {
        0.0
    };
    if c <= 0.003_130_8 {
        12.92 * c
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// The sRGB transfer function, encoded to linear.
pub fn srgb_decode(c: f64) -> f64 {
    if c <= 0.040_45 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// A byte of an sRGB colour as linear light.
pub fn byte_to_linear(b: u8) -> f64 {
    srgb_decode(f64::from(b) / 255.0)
}

const AGX_MIN_EV: f64 = -12.473_931_188;
const AGX_MAX_EV: f64 = 4.026_068_812;

fn agx_curve(x: f64) -> f64 {
    let x2 = x * x;
    let x4 = x2 * x2;
    15.5 * x4 * x2 - 40.14 * x4 * x + 31.96 * x4 - 6.868 * x2 * x + 0.4298 * x2 + 0.1191 * x
        - 0.002_32
}

fn agx(c: [f64; 3]) -> [f64; 3] {
    let [r, g, b] = c.map(|v| if v.is_finite() { v.max(0.0) } else { 0.0 });
    let inset = [
        0.842_479_062_253_094 * r + 0.078_433_599_999_999_2 * g + 0.079_223_745_147_764_3 * b,
        0.042_328_242_261_012_3 * r + 0.878_468_636_469_772 * g + 0.079_166_127_460_543_4 * b,
        0.042_375_654_905_705_1 * r + 0.078_433_6 * g + 0.879_142_973_793_104 * b,
    ];
    let curve = inset.map(|v| {
        let ev = if v > 0.0 { v.log2() } else { AGX_MIN_EV };
        let t = (ev.clamp(AGX_MIN_EV, AGX_MAX_EV) - AGX_MIN_EV) / (AGX_MAX_EV - AGX_MIN_EV);
        agx_curve(t)
    });
    let [r, g, b] = curve;
    [
        1.196_879_005_120_17 * r - 0.098_020_881_140_136_8 * g - 0.099_029_744_079_720_5 * b,
        -0.052_896_851_757_456_2 * r + 1.151_903_129_904_17 * g - 0.098_961_176_844_843_3 * b,
        -0.052_971_635_514_443_8 * r - 0.098_043_450_117_124_1 * g + 1.151_073_672_641_16 * b,
    ]
}

fn aces(x: f64) -> f64 {
    let x = if x.is_finite() { x.max(0.0) * 0.6 } else { 0.0 };
    (x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14)
}

/// Linear scene light (already times the exposure) to display values in
/// 0..1.
pub fn display(c: [f64; 3], curve: Tonemap) -> [f64; 3] {
    match curve {
        // The fit gives display values (its sigmoid is made for a 2.2
        // display); clamped.
        Tonemap::Agx => agx(c).map(|v| v.clamp(0.0, 1.0)),
        Tonemap::Aces => c.map(|v| srgb_encode(aces(v))),
        Tonemap::Linear => c.map(srgb_encode),
    }
}

/// A display value as a byte, rounded as the plain shading rounds.
pub fn to_byte(v: f64) -> u8 {
    // Clamped to 0..=255 before the cast.
    (v * 255.0).round().clamp(0.0, 255.0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srgb_round_trips_every_byte() {
        for b in 0..=255_u8 {
            assert_eq!(to_byte(srgb_encode(byte_to_linear(b))), b);
        }
    }

    #[test]
    fn the_curves_are_monotonic_and_bounded() {
        for curve in [Tonemap::Agx, Tonemap::Aces, Tonemap::Linear] {
            let mut last = -1.0;
            for i in 0..2000 {
                let x = (f64::from(i) / 100.0 - 12.0).exp2();
                let [v, ..] = display([x; 3], curve);
                assert!((0.0..=1.0).contains(&v), "{curve:?} {x} {v}");
                assert!(v >= last - 1e-9, "{curve:?} not monotonic at {x}");
                last = v;
            }
        }
        // Middle grey lands in the middle of the display range.
        let [g, ..] = display([0.18; 3], Tonemap::Agx);
        assert!((0.4..0.6).contains(&g), "{g}");
        assert_eq!(
            display([f64::NAN; 3], Tonemap::Agx),
            display([0.0; 3], Tonemap::Agx)
        );
    }
}
