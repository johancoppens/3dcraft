//! The specular BRDF of the studio look and the split-sum table.
//!
//! Microfacet specular with the GGX (Trowbridge-Reitz) distribution, the
//! height-correlated Smith visibility and Schlick's Fresnel term, as in
//! Karis, *Real Shading in Unreal Engine 4* (SIGGRAPH 2013 course), and
//! the Filament documentation (*Physically Based Rendering in Filament*,
//! §4.4 and §5.3.4). Roughness `r` is perceptual: `α = r²`.
//!
//! The split sum (Karis 2013): the specular light from an environment is
//! approximated by the prefiltered environment times
//! `F0 · A(n·v, r) + B(n·v, r)`, where `A` and `B` integrate the BRDF
//! over the hemisphere with Fresnel split out ([`brdf_lut`]).

use craft3d_math::Portable;
use craft3d_math::Vec3;
use std::f64::consts::PI;

/// Side of the split-sum table: `n·v` across, roughness down.
pub const LUT_SIZE: usize = 32;
/// Importance samples per texel of the split-sum table.
pub const LUT_SAMPLES: u32 = 512;

/// The `i`-th of `n` points of the Hammersley set in the unit square:
/// `(i / n, radical inverse of i in base 2)`. Deterministic, the same
/// points on every run and platform (AGENTS.md rule 5).
pub fn hammersley(i: u32, n: u32) -> (f64, f64) {
    let bits = i.reverse_bits();
    (
        f64::from(i) / f64::from(n.max(1)),
        f64::from(bits) / 4_294_967_296.0,
    )
}

/// The GGX normal distribution `D(h)` for `α = r²`.
pub fn ggx_d(n_dot_h: f64, roughness: f64) -> f64 {
    let a = roughness * roughness;
    let a2 = a * a;
    let d = n_dot_h * n_dot_h * (a2 - 1.0) + 1.0;
    if d <= 0.0 {
        return 0.0;
    }
    a2 / (PI * d * d)
}

/// The height-correlated Smith visibility `V = G / (4 n·l n·v)`
/// (Heitz 2014; Filament §4.4.2).
pub fn smith_v(n_dot_v: f64, n_dot_l: f64, roughness: f64) -> f64 {
    let a = roughness * roughness;
    let a2 = a * a;
    let gv = n_dot_l * (n_dot_v * n_dot_v * (1.0 - a2) + a2).sqrt();
    let gl = n_dot_v * (n_dot_l * n_dot_l * (1.0 - a2) + a2).sqrt();
    let sum = gv + gl;
    if sum <= 0.0 { 0.0 } else { 0.5 / sum }
}

/// A unit vector orthogonal to `n`, and a third completing the frame.
pub fn frame(n: Vec3) -> (Vec3, Vec3) {
    let up = if n.z.abs() < 0.999 { Vec3::Z } else { Vec3::X };
    let t = up.cross(n).normalized().unwrap_or(Vec3::X);
    (t, n.cross(t))
}

/// A half vector around `n` drawn from the GGX distribution for the point
/// `(u, v)` of the unit square (Karis 2013, *ImportanceSampleGGX*).
pub fn sample_ggx(u: f64, v: f64, roughness: f64, n: Vec3) -> Vec3 {
    let a = roughness * roughness;
    let phi = 2.0 * PI * u;
    let cos_t = ((1.0 - v) / (1.0 + (a * a - 1.0) * v)).max(0.0).sqrt();
    let sin_t = (1.0 - cos_t * cos_t).max(0.0).sqrt();
    let (t, b) = frame(n);
    t * (sin_t * phi.pcos()) + b * (sin_t * phi.psin()) + n * cos_t
}

/// `A` and `B` of the split sum at `n·v` and roughness `r`, by GGX
/// importance sampling with [`LUT_SAMPLES`] Hammersley points.
pub fn split_sum(n_dot_v: f64, roughness: f64) -> (f64, f64) {
    let nv = n_dot_v.clamp(1e-4, 1.0);
    let v = Vec3::new((1.0 - nv * nv).max(0.0).sqrt(), 0.0, nv);
    let (mut a, mut b) = (0.0, 0.0);
    for i in 0..LUT_SAMPLES {
        let (u, w) = hammersley(i, LUT_SAMPLES);
        let h = sample_ggx(u, w, roughness, Vec3::Z);
        let vh = v.dot(h);
        let l = h * (2.0 * vh) - v;
        let (nl, nh) = (l.z, h.z);
        if nl > 0.0 && nh > 0.0 && vh > 0.0 {
            // pdf(l) = D nh / (4 vh); f cos / pdf = V · 4 nl vh / nh.
            let g = smith_v(nv, nl, roughness) * 4.0 * nl * vh / nh;
            let fc = (1.0 - vh).ppowi(5);
            a += (1.0 - fc) * g;
            b += fc * g;
        }
    }
    let n = f64::from(LUT_SAMPLES);
    (a / n, b / n)
}

/// [`brdf_lut`], computed once per process: it does not depend on the
/// environment, and computing it is a pure function, so sharing it changes
/// no result.
pub fn shared_lut() -> &'static [[f32; 2]] {
    static LUT: std::sync::OnceLock<Vec<[f32; 2]>> = std::sync::OnceLock::new();
    LUT.get_or_init(brdf_lut)
}

/// The split-sum table, [`LUT_SIZE`] × [`LUT_SIZE`] texels, row-major:
/// texel `(i, j)` holds `A` and `B` at `n·v = (i + 0.5) / LUT_SIZE` and
/// roughness `(j + 0.5) / LUT_SIZE`, as `f32` (what both renderers read).
pub fn brdf_lut() -> Vec<[f32; 2]> {
    let mut out = Vec::with_capacity(LUT_SIZE * LUT_SIZE);
    for j in 0..LUT_SIZE {
        let r = (j as f64 + 0.5) / LUT_SIZE as f64;
        for i in 0..LUT_SIZE {
            let nv = (i as f64 + 0.5) / LUT_SIZE as f64;
            let (a, b) = split_sum(nv, r);
            out.push([a as f32, b as f32]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The same integral by a midpoint rule over the hemisphere in
    /// spherical coordinates: a method independent of the importance
    /// sampling.
    fn quadrature(nv: f64, r: f64) -> (f64, f64) {
        let v = Vec3::new((1.0 - nv * nv).sqrt(), 0.0, nv);
        let (nt, np) = (600, 600);
        let (mut a, mut b) = (0.0, 0.0);
        for it in 0..nt {
            let t = (it as f64 + 0.5) / nt as f64 * PI / 2.0;
            for ip in 0..np {
                let p = (ip as f64 + 0.5) / np as f64 * 2.0 * PI;
                let l = Vec3::new(t.psin() * p.pcos(), t.psin() * p.psin(), t.pcos());
                let Some(h) = (l + v).normalized() else {
                    continue;
                };
                let vh = v.dot(h).max(0.0);
                let f = ggx_d(h.z, r) * smith_v(nv, l.z, r) * l.z;
                let dw = t.psin() * (PI / 2.0 / nt as f64) * (2.0 * PI / np as f64);
                let fc = (1.0 - vh).ppowi(5);
                a += (1.0 - fc) * f * dw;
                b += fc * f * dw;
            }
        }
        (a, b)
    }

    #[test]
    fn the_split_sum_matches_an_independent_quadrature() {
        for &(nv, r) in &[(0.9, 0.5), (0.5, 0.5), (0.2, 0.7), (0.7, 0.9), (0.5, 0.3)] {
            let (a, b) = split_sum(nv, r);
            let (qa, qb) = quadrature(nv, r);
            assert!(
                (a - qa).abs() < 0.02 && (b - qb).abs() < 0.01,
                "{nv} {r}: {a} {b} vs {qa} {qb}"
            );
        }
    }

    #[test]
    fn a_smooth_surface_has_the_fresnel_closed_form() {
        // r → 0: the BRDF is a mirror, A + B·F0 = F(n·v) with Schlick:
        // A = 1 − (1 − n·v)^5, B = (1 − n·v)^5.
        for &nv in &[0.2, 0.5, 0.9] {
            let (a, b) = split_sum(nv, 0.02);
            let fc = (1.0_f64 - nv).ppowi(5);
            assert!(
                (a - (1.0 - fc)).abs() < 0.01 && (b - fc).abs() < 0.01,
                "{nv}: {a} {b}"
            );
        }
    }

    #[test]
    fn the_table_is_deterministic_and_bounded() {
        let t = brdf_lut();
        assert_eq!(t, brdf_lut());
        assert!(
            t.iter()
                .all(|[a, b]| *a >= 0.0 && *b >= 0.0 && a + b <= 1.0001)
        );
    }
}
