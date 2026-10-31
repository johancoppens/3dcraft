//! Transcendental functions that give the same bits on every platform.
//!
//! `f64::sin`, `cos`, `atan2`, `exp`, `ln`, `powf` and the other
//! transcendental methods of the standard library call the platform's C
//! maths library (glibc on Linux, Apple's libm on macOS, the UCRT on
//! Windows, a compiled-in port on wasm). Those agree to within an ulp or
//! two, but not bit for bit, and the compiler may fold a call with a
//! constant argument using the build machine's library. A last-bit
//! difference is enough to flip a branch in the kernel (which fan, which
//! face count), so the same document regenerated to different bodies on
//! different platforms, against AGENTS.md rule 5 (docs/design/geometry.md,
//! Portable maths).
//!
//! [`Portable`] routes every such function through the pure-Rust `libm`
//! crate (MIT, a port of musl's libm): plain Rust arithmetic, so the same
//! bits on every target. Its only platform-specific paths are the
//! correctly rounded IEEE 754 operations (`sqrt`, `fma`, `floor`, `ceil`,
//! `trunc`, `rint`, `fabs`), whose results the standard fixes exactly.
//! `powi` is written out here (the standard's leaves the order of its
//! multiplications to the compiler). `sqrt`, `abs`, `floor`, `min` and the
//! like are exact and stay the standard's.
//!
//! The method names carry a `p` because a trait method of the same name as
//! an inherent one is never called. Clippy's `disallowed-methods`
//! (clippy.toml) refuses the standard's versions everywhere in the
//! workspace.

/// Portable transcendental functions for `f64` and `f32`: the same bits on
/// every platform. Import it with `use craft3d_math::Portable as _;`.
pub trait Portable: Sized + Copy {
    fn psin(self) -> Self;
    fn pcos(self) -> Self;
    /// `(sin, cos)`, as `sin_cos`.
    fn psin_cos(self) -> (Self, Self);
    fn ptan(self) -> Self;
    fn pasin(self) -> Self;
    fn pacos(self) -> Self;
    fn patan(self) -> Self;
    /// The angle of `(other, self)`, as `atan2`: `y.patan2(x)`.
    fn patan2(self, other: Self) -> Self;
    fn psinh(self) -> Self;
    fn pcosh(self) -> Self;
    fn ptanh(self) -> Self;
    fn pasinh(self) -> Self;
    fn pacosh(self) -> Self;
    fn patanh(self) -> Self;
    fn pexp(self) -> Self;
    fn pexp2(self) -> Self;
    /// `e^self - 1`, accurate near zero.
    fn pexp_m1(self) -> Self;
    /// The natural logarithm, as `ln`.
    fn pln(self) -> Self;
    /// `ln(1 + self)`, accurate near zero.
    fn pln_1p(self) -> Self;
    fn plog10(self) -> Self;
    fn plog2(self) -> Self;
    /// The logarithm in `base`, as `log`: `ln(self) / ln(base)`.
    fn plog(self, base: Self) -> Self;
    /// `self` to a real power, as `powf`.
    fn ppowf(self, n: Self) -> Self;
    /// `self` to an integer power, as `powi`, by repeated squaring in a
    /// fixed order.
    fn ppowi(self, n: i32) -> Self;
    fn pcbrt(self) -> Self;
    /// `sqrt(self² + other²)` without undue overflow, as `hypot`.
    fn phypot(self, other: Self) -> Self;
    /// `self * a + b` rounded once, as `mul_add`.
    fn pmul_add(self, a: Self, b: Self) -> Self;
}

macro_rules! portable {
    ($t:ty, $sin:ident, $cos:ident, $sincos:ident, $tan:ident, $asin:ident,
     $acos:ident, $atan:ident, $atan2:ident, $sinh:ident, $cosh:ident,
     $tanh:ident, $asinh:ident, $acosh:ident, $atanh:ident, $exp:ident,
     $exp2:ident, $expm1:ident, $log:ident, $log1p:ident, $log10:ident,
     $log2:ident, $pow:ident, $cbrt:ident, $hypot:ident, $fma:ident) => {
        impl Portable for $t {
            #[inline]
            fn psin(self) -> Self {
                libm::$sin(self)
            }
            #[inline]
            fn pcos(self) -> Self {
                libm::$cos(self)
            }
            #[inline]
            fn psin_cos(self) -> (Self, Self) {
                libm::$sincos(self)
            }
            #[inline]
            fn ptan(self) -> Self {
                libm::$tan(self)
            }
            #[inline]
            fn pasin(self) -> Self {
                libm::$asin(self)
            }
            #[inline]
            fn pacos(self) -> Self {
                libm::$acos(self)
            }
            #[inline]
            fn patan(self) -> Self {
                libm::$atan(self)
            }
            #[inline]
            fn patan2(self, other: Self) -> Self {
                libm::$atan2(self, other)
            }
            #[inline]
            fn psinh(self) -> Self {
                libm::$sinh(self)
            }
            #[inline]
            fn pcosh(self) -> Self {
                libm::$cosh(self)
            }
            #[inline]
            fn ptanh(self) -> Self {
                libm::$tanh(self)
            }
            #[inline]
            fn pasinh(self) -> Self {
                libm::$asinh(self)
            }
            #[inline]
            fn pacosh(self) -> Self {
                libm::$acosh(self)
            }
            #[inline]
            fn patanh(self) -> Self {
                libm::$atanh(self)
            }
            #[inline]
            fn pexp(self) -> Self {
                libm::$exp(self)
            }
            #[inline]
            fn pexp2(self) -> Self {
                libm::$exp2(self)
            }
            #[inline]
            fn pexp_m1(self) -> Self {
                libm::$expm1(self)
            }
            #[inline]
            fn pln(self) -> Self {
                libm::$log(self)
            }
            #[inline]
            fn pln_1p(self) -> Self {
                libm::$log1p(self)
            }
            #[inline]
            fn plog10(self) -> Self {
                libm::$log10(self)
            }
            #[inline]
            fn plog2(self) -> Self {
                libm::$log2(self)
            }
            #[inline]
            fn plog(self, base: Self) -> Self {
                libm::$log(self) / libm::$log(base)
            }
            #[inline]
            fn ppowf(self, n: Self) -> Self {
                libm::$pow(self, n)
            }
            #[inline]
            fn ppowi(self, n: i32) -> Self {
                // compiler-rt's `__powidf2`: square the base, multiply the
                // bits of the exponent in from the lowest.
                let mut base = self;
                let mut m = n.unsigned_abs();
                let mut r: Self = 1.0;
                loop {
                    if m & 1 == 1 {
                        r *= base;
                    }
                    m >>= 1;
                    if m == 0 {
                        break;
                    }
                    base *= base;
                }
                if n < 0 { 1.0 / r } else { r }
            }
            #[inline]
            fn pcbrt(self) -> Self {
                libm::$cbrt(self)
            }
            #[inline]
            fn phypot(self, other: Self) -> Self {
                libm::$hypot(self, other)
            }
            #[inline]
            fn pmul_add(self, a: Self, b: Self) -> Self {
                libm::$fma(self, a, b)
            }
        }
    };
}

portable!(
    f64, sin, cos, sincos, tan, asin, acos, atan, atan2, sinh, cosh, tanh, asinh, acosh, atanh,
    exp, exp2, expm1, log, log1p, log10, log2, pow, cbrt, hypot, fma
);
portable!(
    f32, sinf, cosf, sincosf, tanf, asinf, acosf, atanf, atan2f, sinhf, coshf, tanhf, asinhf,
    acoshf, atanhf, expf, exp2f, expm1f, logf, log1pf, log10f, log2f, powf, cbrtf, hypotf, fmaf
);

/// A sweep of arguments: a seeded walk over several decades, both signs,
/// and some special values.
pub(crate) fn sweep() -> Vec<f64> {
    let mut out = vec![0.0, -0.0, 1.0, -1.0, 0.5, f64::MIN_POSITIVE, 1e-300, 1e300];
    let mut state: u64 = 0x9e37_79b9_7f4a_7c15;
    for _ in 0..20_000 {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let mantissa = (state >> 11) as f64 / (1u64 << 53) as f64;
        let decade = ((state >> 3) % 13) as i32 - 6;
        let sign = if state & 1 == 1 { -1.0 } else { 1.0 };
        out.push(sign * mantissa * 10f64.ppowi(decade));
    }
    out
}

/// A hash (FNV-1a) of the bits of every [`Portable`] function over
/// [`sweep`] (pairs of neighbouring arguments for the functions of two),
/// `f32` included. It is the same on every platform, wasm included: the
/// contract of this module, pinned by its test and run on the targets the
/// tests do not reach (docs/design/geometry.md, Portable maths).
#[doc(hidden)]
pub fn fingerprint() -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    // A NaN's sign and payload are left to the hardware (x86 gives a
    // negative one, ARM and wasm a positive one): every NaN is hashed as
    // one.
    let mut mix = |x: f64| {
        let bits = if x.is_nan() {
            f64::NAN.to_bits()
        } else {
            x.to_bits()
        };
        for byte in bits.to_le_bytes() {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
        }
    };
    let xs = sweep();
    for pair in xs.windows(2) {
        let &[y, x] = pair else {
            continue;
        };
        for v in [
            x.psin(),
            x.pcos(),
            x.ptan(),
            x.pasin(),
            x.pacos(),
            x.patan(),
            y.patan2(x),
            x.psinh(),
            x.pcosh(),
            x.ptanh(),
            x.pasinh(),
            x.abs().pacosh(),
            x.patanh(),
            x.pexp(),
            x.pexp2(),
            x.pexp_m1(),
            x.abs().pln(),
            x.pln_1p(),
            x.abs().plog10(),
            x.abs().plog2(),
            y.abs().ppowf(x.clamp(-30.0, 30.0)),
            x.ppowi(7),
            x.pcbrt(),
            y.phypot(x),
            y.pmul_add(x, 1.5),
            f64::from((x as f32).psin()),
            f64::from((y as f32).patan2(x as f32)),
        ] {
            mix(v);
        }
    }
    hash
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // compares against the standard's versions
mod tests {
    use super::{Portable, fingerprint, sweep};

    /// Distance in units in the last place between two finite `f64`s.
    fn ulps(a: f64, b: f64) -> u64 {
        if a == b || (a.is_nan() && b.is_nan()) {
            return 0;
        }
        let key = |x: f64| {
            let bits = x.to_bits() as i64;
            if bits < 0 { i64::MIN - bits } else { bits }
        };
        key(a).abs_diff(key(b))
    }

    type Pair = (&'static str, fn(f64) -> f64, fn(f64) -> f64);

    /// The portable functions agree with the platform's to within two ulps
    /// over the sweep (four for `tanh`), so swapping one for the other
    /// moves a result by rounding only. The worst and the number of
    /// arguments whose last bits differ are printed.
    ///
    /// Measured against glibc only: Apple's library and the UCRT promise
    /// their own accuracy, and the platform's library is what this module
    /// replaces, so its last bits are not a contract. The pinned
    /// [`fingerprint`] is.
    #[test]
    #[cfg_attr(
        not(all(target_os = "linux", target_env = "gnu")),
        ignore = "measured against glibc only; the pinned fingerprint is the contract"
    )]
    fn the_portable_functions_agree_with_the_platform_within_a_few_ulps() {
        let unary: [Pair; 17] = [
            ("sin", f64::psin, f64::sin),
            ("cos", f64::pcos, f64::cos),
            ("tan", f64::ptan, f64::tan),
            ("asin", f64::pasin, f64::asin),
            ("acos", f64::pacos, f64::acos),
            ("atan", f64::patan, f64::atan),
            ("sinh", f64::psinh, f64::sinh),
            ("cosh", f64::pcosh, f64::cosh),
            ("tanh", f64::ptanh, f64::tanh),
            ("asinh", f64::pasinh, f64::asinh),
            ("exp", f64::pexp, f64::exp),
            ("exp_m1", f64::pexp_m1, f64::exp_m1),
            ("ln", |x| x.abs().pln(), |x| x.abs().ln()),
            ("ln_1p", f64::pln_1p, f64::ln_1p),
            ("log10", |x| x.abs().plog10(), |x| x.abs().log10()),
            ("log2", |x| x.abs().plog2(), |x| x.abs().log2()),
            ("cbrt", f64::pcbrt, f64::cbrt),
        ];
        let xs = sweep();
        for (name, ours, std) in unary {
            let worst = xs.iter().map(|&x| ulps(ours(x), std(x))).max().unwrap_or(0);
            let differ = xs.iter().filter(|&&x| ulps(ours(x), std(x)) > 0).count();
            eprintln!("{name}: worst {worst} ulp, {differ} of {} differ", xs.len());
            // musl's tanh is the least accurate of its functions (three
            // ulps measured against glibc, 2026-10-31). `atanh` is left
            // out: the standard's is a formula of its own (`ln_1p` of a
            // quotient), 51 ulps from musl's near ±1.
            let bound = if name == "tanh" { 4 } else { 2 };
            assert!(worst <= bound, "{name}: {worst} ulps");
        }
        let mut worst = [0u64; 4];
        for pair in xs.windows(2) {
            let (y, x) = (pair[0], pair[1]);
            worst[0] = worst[0].max(ulps(y.patan2(x), y.atan2(x)));
            worst[1] = worst[1].max(ulps(y.phypot(x), y.hypot(x)));
            let (b, e) = (y.abs(), x.clamp(-30.0, 30.0));
            worst[2] = worst[2].max(ulps(b.ppowf(e), b.powf(e)));
            worst[3] = worst[3].max(ulps(y.pmul_add(x, 1.5), y.mul_add(x, 1.5)));
        }
        eprintln!("atan2, hypot, powf, mul_add: {worst:?} ulp");
        assert!(worst[..3].iter().all(|&w| w <= 2), "{worst:?}");
        assert_eq!(worst[3], 0, "fma is correctly rounded");
        for x in xs.iter().take(2000) {
            let (s, c) = x.psin_cos();
            assert_eq!(
                (s.to_bits(), c.to_bits()),
                (x.psin().to_bits(), x.pcos().to_bits())
            );
            for n in -12..=12 {
                assert!(ulps(x.ppowi(n), x.powi(n)) <= 2 * n.unsigned_abs() as u64 + 1);
            }
        }
    }

    /// The portable functions give the same bits on every platform: their
    /// [`fingerprint`] is pinned. A different one on some platform means its
    /// build of `libm` computes differently, which this module exists to
    /// rule out.
    #[test]
    fn the_portable_functions_give_the_same_bits_everywhere() {
        let hash = fingerprint();
        eprintln!("hash {hash:#018x}");
        assert_eq!(hash, PINNED);
    }

    /// Measured on x86_64 Linux and on wasm32 in Node (debug and release
    /// builds, 2026-10-31); every platform must give it. (The first pin
    /// hashed a NaN's own bits, which x86 and wasm make differently.)
    const PINNED: u64 = 0x5d41_fac0_5637_ef43;
}
