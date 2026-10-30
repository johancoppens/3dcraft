//! Layer 3.5's solid textures: the noise's bits (against an independent
//! evaluation of the same published formulas), its statistics, period and
//! seeds; wood's rings against the closed form; the pictures' determinism
//! and doubt (docs/design/rendering.md, *Layer 3.5: solid textures*).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::needless_range_loop
)]

use craft3d_math::Vec3;
use craft3d_render::studio::layers::Optics;
use craft3d_render::studio::pattern::{
    self, Pattern, PatternKind, atan2_exact, cellular, fbm, noise, offset, pcg3d, ring_profile,
    sin_pi,
};
use craft3d_render::studio::{
    HdrImage, Look, Prepared, env::texel_direction, render_studio, render_studio_doubt,
};
use craft3d_render::{Camera, FaceColors, Finish, Projection, RenderParams};
use craft3d_tess::Mesh;
use std::path::PathBuf;

/// A small deterministic generator for test points (xorshift64*).
struct Points(u64);

impl Points {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        let v = self.0.wrapping_mul(0x2545_f491_4f6c_dd1d);
        (v >> 11) as f64 / (1u64 << 53) as f64
    }

    fn point(&mut self, half: f64) -> [f64; 3] {
        [0, 1, 2].map(|_| (self.next() * 2.0 - 1.0) * half)
    }
}

#[test]
fn the_hash_and_the_noise_are_the_published_formulas_bit_for_bit() {
    // Worked out apart from the code, by a separate evaluation of
    // Jarzynski and Olano's pcg3d and Perlin's improved noise in f64 (the
    // same operations in the same order, so the same bits on any IEEE
    // machine).
    assert_eq!(
        pcg3d([1, 2, 3]),
        [4_204_755_366, 1_223_881_804, 1_500_469_937]
    );
    assert_eq!(offset(7, 0), [3_045_078_429, 3_393_433_451, 2_586_273_628]);
    let cases: [([f64; 3], [u64; 4]); 3] = [
        (
            [0.3, 0.7, 0.1],
            [
                0x3fd5_ae83_7df0_dbbb,
                0x3fae_39e3_7874_f609,
                0x3fe7_b26b_2d06_2e7b,
                0x3fe1_cee7_8000_0000,
            ],
        ),
        (
            [-12.25, 3.5, 100.125],
            [
                0x3fc7_aae5_2000_0000,
                0xbfc6_ac04_2000_0000,
                0x3fe9_76e1_31ce_170e,
                0x3fdb_5817_c000_0000,
            ],
        ),
        (
            [100_000.4, -1999.4, 0.9],
            [
                0x3fb7_ba77_5aea_6314,
                0xbfc1_4042_b51d_880a,
                0x3fe4_dcfd_5c66_1499,
                0x3fbf_e48d_0000_0000,
            ],
        ),
    ];
    for (q, [n, f, d, v]) in cases {
        assert_eq!(noise(q, offset(0, 0)).to_bits(), n, "noise at {q:?}");
        assert_eq!(fbm(q, 42, 4, 0).to_bits(), f, "fBm at {q:?}");
        let (dist, value) = cellular(q, offset(3, 0));
        assert_eq!(dist.to_bits(), d, "cellular distance at {q:?}");
        assert_eq!(value.to_bits(), v, "cellular value at {q:?}");
    }
}

#[test]
fn the_noise_has_the_statistics_of_gradient_noise() {
    let mut points = Points(0x9e37_79b9_7f4a_7c15);
    let samples: Vec<[f64; 3]> = (0..20_000).map(|_| points.point(50.0)).collect();
    let stats = |values: &[f64]| {
        let n = values.len() as f64;
        let mean = values.iter().sum::<f64>() / n;
        let var = values.iter().map(|v| (v - mean) * (v - mean)).sum::<f64>() / n;
        (mean, var)
    };
    let ns: Vec<f64> = samples.iter().map(|&q| noise(q, offset(0, 0))).collect();
    let (mean, var) = stats(&ns);
    // Gradient noise is odd under the hash's symmetric gradients: mean 0;
    // its variance for Perlin's twelve gradients is about 0.07 (measured
    // by the independent evaluation: 0.0734 over 20 000 points).
    assert!(mean.abs() < 0.01, "mean {mean}");
    assert!((0.055..0.09).contains(&var), "variance {var}");
    assert!(ns.iter().all(|v| v.abs() <= 1.0), "within −1 to 1");
    let fs: Vec<f64> = samples.iter().map(|&q| fbm(q, 0, 4, 0)).collect();
    let (fmean, fvar) = stats(&fs);
    // Four octaves of gain ½ add variances (the octaves are independent):
    // var · (1 + 1/4 + 1/16 + 1/64) = 1.33 var.
    assert!(fmean.abs() < 0.01, "fBm mean {fmean}");
    assert!(
        (fvar / var - 1.328).abs() < 0.08,
        "fBm variance {fvar} against {var} · 1.33"
    );
    // Cellular noise: values uniform on [0, 1), the nearest of a jittered
    // grid of one point per cell about half a cell away.
    let cs: Vec<(f64, f64)> = samples
        .iter()
        .take(5000)
        .map(|&q| cellular(q, offset(0, 0)))
        .collect();
    let dmean = cs.iter().map(|c| c.0).sum::<f64>() / cs.len() as f64;
    let vmean = cs.iter().map(|c| c.1).sum::<f64>() / cs.len() as f64;
    assert!((0.45..0.6).contains(&dmean), "F1 mean {dmean}");
    assert!((vmean - 0.5).abs() < 0.03, "cell value mean {vmean}");
}

#[test]
fn the_noise_vanishes_on_the_lattice_has_no_short_period_and_follows_its_seed() {
    for (i, j, k) in [(0, 0, 0), (3, -7, 12), (-1000, 55, 9)] {
        let q = [f64::from(i), f64::from(j), f64::from(k)];
        assert_eq!(
            noise(q, offset(5, 0)),
            0.0,
            "zero at the lattice point {q:?}"
        );
    }
    let mut points = Points(17);
    let samples: Vec<[f64; 3]> = (0..4000).map(|_| points.point(20.0)).collect();
    let correlation = |a: &[f64], b: &[f64]| {
        let n = a.len() as f64;
        let (ma, mb) = (a.iter().sum::<f64>() / n, b.iter().sum::<f64>() / n);
        let cov: f64 = a.iter().zip(b).map(|(x, y)| (x - ma) * (y - mb)).sum();
        let va: f64 = a.iter().map(|x| (x - ma) * (x - ma)).sum();
        let vb: f64 = b.iter().map(|y| (y - mb) * (y - mb)).sum();
        cov / (va * vb).sqrt()
    };
    let base: Vec<f64> = samples.iter().map(|&q| noise(q, offset(0, 0))).collect();
    // No period the lattice reaches in a part: shifted by any whole number
    // of cells from 1 to 256 along any axis, the field is unrelated.
    for period in (1..=256).step_by(5) {
        for axis in 0..3 {
            let shifted: Vec<f64> = samples
                .iter()
                .map(|&q| {
                    let mut q = q;
                    q[axis] += f64::from(period);
                    noise(q, offset(0, 0))
                })
                .collect();
            let c = correlation(&base, &shifted);
            assert!(c.abs() < 0.1, "period {period} along {axis}: {c}");
        }
    }
    // The same seed gives the same field; another seed an unrelated one.
    let again: Vec<f64> = samples.iter().map(|&q| noise(q, offset(0, 0))).collect();
    assert_eq!(base, again);
    for seed in [1, 2, 99, u32::MAX] {
        let other: Vec<f64> = samples.iter().map(|&q| noise(q, offset(seed, 0))).collect();
        let c = correlation(&base, &other);
        assert!(c.abs() < 0.1, "seed {seed}: {c}");
    }
}

fn wood(scale: f32) -> Pattern {
    Pattern {
        kind: PatternKind::Wood,
        ramp: [[220, 180, 130], [120, 75, 40], [0; 3], [0; 3]],
        stops: 2,
        scale,
        origin: [1.0, -2.0, 0.5],
        axis: [0.0, 0.0, 1.0],
        ..Pattern::default()
    }
}

#[test]
fn wood_without_noise_has_rings_of_its_scale_along_a_radius() {
    for scale in [0.004_f32, 1.5, 3000.0] {
        let p = wood(scale).sanitized();
        let s = f64::from(scale);
        let origin = Vec3::new(1.0, -2.0, 0.5);
        let dir = Vec3::new(0.6, 0.8, 0.0);
        // The value along the radius is the closed-form profile of the
        // place within the ring, `fract(r / s)`.
        let steps = 4000;
        let mut values = Vec::with_capacity(steps);
        for i in 0..steps {
            let r = 8.0 * s * i as f64 / steps as f64;
            let at = origin + dir * r + Vec3::Z * (0.3 * s);
            let t = p.value(p.local(at));
            let f = (r / s).fract();
            assert!(
                (t - ring_profile(f)).abs() < 1e-6,
                "scale {scale} r {r}: {t} against {}",
                ring_profile(f)
            );
            values.push(t);
        }
        // The rings' darkest places are one scale apart.
        let peaks: Vec<f64> = (1..steps - 1)
            .filter(|&i| values[i] > values[i - 1] && values[i] >= values[i + 1] && values[i] > 0.9)
            .map(|i| 8.0 * s * i as f64 / steps as f64)
            .collect();
        assert_eq!(peaks.len(), 8, "scale {scale}: {peaks:?}");
        for pair in peaks.windows(2) {
            let spacing = pair[1] - pair[0];
            assert!(
                (spacing - s).abs() < 2.0 * 8.0 * s / steps as f64,
                "scale {scale}: spacing {spacing}"
            );
        }
    }
}

#[test]
fn every_kind_stays_on_its_ramp_and_its_mean_colour_lies_between_its_ends() {
    let mut points = Points(3);
    for kind in PatternKind::ALL {
        let p = Pattern {
            kind,
            ramp: [[250, 240, 230], [20, 30, 40], [0; 3], [0; 3]],
            stops: 2,
            distortion: 1.0,
            grain: 0.5,
            rays: 0.5,
            scale: 0.7,
            roughness: Some([0.2, 0.8]),
            ..Pattern::default()
        }
        .sanitized();
        let mut lo = f64::INFINITY;
        let mut hi = f64::NEG_INFINITY;
        for _ in 0..3000 {
            let q = points.point(20.0);
            let at = Vec3::new(q[0], q[1], q[2]);
            let t = p.value(p.local(at));
            assert!((0.0..=1.0).contains(&t), "{kind:?} at {q:?}: {t}");
            let (_, r) = p.at(at, 0.5);
            // The ramp's ends are f32 numbers.
            assert!(
                (0.2 - 1e-6..=0.8 + 1e-6).contains(&r),
                "{kind:?}: roughness {r}"
            );
            lo = lo.min(t);
            hi = hi.max(t);
        }
        assert!(hi - lo > 0.3, "{kind:?} varies: {lo} to {hi}");
        let mean = p.mean_color();
        for c in 0..3 {
            assert!(
                (20..=250).contains(&mean[c]),
                "{kind:?}: mean colour {mean:?}"
            );
        }
        // A ramp of one colour has that colour as its mean.
        let flat = Pattern {
            ramp: [[90, 140, 200]; 4],
            ..p
        };
        assert_eq!(flat.mean_color(), [90, 140, 200], "{kind:?}");
    }
}

#[test]
fn sanitizing_brings_a_pattern_into_range() {
    let wild = Pattern {
        stops: 9,
        roughness: Some([f32::NAN, 4.0]),
        scale: -1.0,
        origin: [f64::NAN, 0.0, 0.0],
        axis: [0.0, 0.0, 0.0],
        distortion: f32::INFINITY,
        detail: 99,
        grain: -2.0,
        rays: f32::NAN,
        spot: 7.0,
        ..Pattern::default()
    }
    .sanitized();
    assert_eq!(wild.stops, pattern::MAX_STOPS as u8);
    assert_eq!(wild.roughness, Some([0.5, 1.0]));
    assert_eq!(wild.scale, pattern::MIN_SCALE);
    assert_eq!(wild.origin, [0.0; 3]);
    assert_eq!(wild.axis, [0.0, 0.0, 1.0]);
    assert_eq!(wild.distortion, 0.0);
    assert_eq!(wild.detail, pattern::MAX_DETAIL);
    assert_eq!((wild.grain, wild.rays, wild.spot), (0.0, 0.0, 1.0));
    // Hostile points give a value on the ramp.
    for q in [
        [f64::NAN, 0.0, 0.0],
        [f64::INFINITY, -1e300, 1e300],
        [1e9, 1e9, 1e9],
    ] {
        for kind in PatternKind::ALL {
            let t = Pattern { kind, ..wild }.value(q);
            assert!((0.0..=1.0).contains(&t), "{kind:?} at {q:?}: {t}");
        }
    }
}

fn environment(w: usize, f: impl Fn(Vec3) -> [f64; 3]) -> HdrImage {
    let h = w / 2;
    let mut pixels = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            pixels.push(f(texel_direction(x, y, w, h)).map(|v| v as f32));
        }
    }
    HdrImage {
        width: w,
        height: h,
        pixels,
    }
}

/// A box of half sizes `half` about `c`, faces `first..first + 6`.
fn block(c: Vec3, half: [f64; 3], first: u32) -> Mesh {
    let (u, v, w) = (Vec3::X, Vec3::Y, Vec3::Z);
    let [hu, hv, hw] = half;
    let mut mesh = Mesh::default();
    let sides = [
        (u, hu, v, hv, w, hw),
        (-u, hu, w, hw, v, hv),
        (v, hv, w, hw, u, hu),
        (-v, hv, u, hu, w, hw),
        (w, hw, u, hu, v, hv),
        (-w, hw, v, hv, u, hu),
    ];
    for (k, (n, hn, a, ha, b, hb)) in sides.into_iter().enumerate() {
        let base = mesh.positions.len() as u32;
        let centre = c + n * hn;
        for (sa, sb) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            mesh.positions.push(centre + a * (sa * ha) + b * (sb * hb));
            mesh.normals.push(n);
        }
        mesh.triangles.push([base, base + 1, base + 2]);
        mesh.triangles.push([base, base + 2, base + 3]);
        mesh.face_ids.push(first + k as u32);
        mesh.face_ids.push(first + k as u32);
    }
    mesh
}

fn evidence(name: &str, frame: &craft3d_render::Frame) {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/evidence/pattern");
    let _ = std::fs::create_dir_all(&dir);
    let png = frame.to_png().unwrap();
    let _ = std::fs::write(dir.join(name), &png);
    craft3d_testkit::evidence::print(name, &png);
}

#[test]
fn patterned_pictures_are_deterministic_differ_from_plain_and_their_doubt_is_small() {
    let env = Prepared::new(&environment(64, |d| [0.5 + d.z.max(0.0) * 3.0; 3])).unwrap();
    let mesh = block(Vec3::ZERO, [2.0, 1.0, 0.4], 0);
    let camera = Camera {
        eye: Vec3::new(3.0, -6.0, 4.0),
        target: Vec3::ZERO,
        up: Vec3::Z,
        projection: Projection::Perspective { fov_y_deg: 35.0 },
    };
    let params = RenderParams {
        width: 320,
        height: 240,
        ssaa: 2,
        ..RenderParams::default()
    };
    let look = Look {
        ground: true,
        shadows: true,
        ao: true,
        ..Look::default()
    };
    let finish = Finish {
        metallic: 0,
        roughness: Some(150),
    };
    for kind in PatternKind::ALL {
        let mut colors = FaceColors::new();
        let pattern = Pattern {
            kind,
            ramp: [[225, 190, 140], [110, 70, 35], [60, 40, 20], [0; 3]],
            stops: 3,
            scale: 0.25,
            origin: [0.3, 0.0, -1.5],
            axis: [1.0, 0.0, 0.0],
            distortion: 0.6,
            grain: 0.5,
            rays: 0.4,
            seed: 11,
            ..Pattern::default()
        };
        for face in 0..6 {
            colors.set_look(face, Some(([180, 140, 100], finish)));
            assert!(colors.set_optics(
                face,
                Some(Optics {
                    pattern: Some(pattern),
                    ..Optics::default()
                })
            ));
        }
        let a = render_studio(&mesh, &colors, &camera, &params, &look, &env).unwrap();
        let b = render_studio(&mesh, &colors, &camera, &params, &look, &env).unwrap();
        assert_eq!(a.rgb, b.rgb, "{kind:?}: deterministic");
        let mut plain = FaceColors::new();
        for face in 0..6 {
            plain.set_look(face, Some(([180, 140, 100], finish)));
        }
        let p = render_studio(&mesh, &plain, &camera, &params, &look, &env).unwrap();
        let differ = a
            .rgb
            .chunks(3)
            .zip(p.rgb.chunks(3))
            .filter(|(x, y)| x != y)
            .count();
        assert!(
            differ > a.ids.len() / 10,
            "{kind:?}: the pattern shows ({differ})"
        );
        let (c, doubt) =
            render_studio_doubt(&mesh, &colors, &camera, &params, &look, &env, 128.0).unwrap();
        assert_eq!(
            a.rgb, c.rgb,
            "{kind:?}: marking doubt does not change the picture"
        );
        let covered = a
            .ids
            .iter()
            .filter(|&&id| id != craft3d_render::NO_FACE)
            .count();
        let unsure = doubt.iter().filter(|&&d| d > 0.0).count();
        println!("{kind:?}: {unsure} of {covered} covered pixels touch a sample in doubt");
        // The doubt is the pattern's steep places within a 256th of a
        // sample (the GPU's sub-pixel snapping): here, with features of a
        // sixteenth of the block, a thin set of the pixels.
        assert!(
            unsure * 8 < covered,
            "{kind:?}: {unsure} of {covered} in doubt"
        );
        evidence(&format!("{}.png", kind.name()), &a);
    }
}

#[test]
fn the_platform_free_sine_and_arc_tangent_are_the_standard_ones_within_rounding() {
    for i in -2000..=2000 {
        let x = f64::from(i) / 2000.0;
        let ours = sin_pi(x);
        let theirs = (std::f64::consts::PI * x).sin();
        assert!((ours - theirs).abs() < 1e-14, "sin(π {x}): {ours} {theirs}");
    }
    let mut points = Points(5);
    for _ in 0..20_000 {
        let [x, y, _] = points.point(10.0);
        let (ours, theirs) = (atan2_exact(y, x), y.atan2(x));
        assert!(
            (ours - theirs).abs() < 1e-14,
            "atan2({y}, {x}): {ours} {theirs}"
        );
    }
    for (y, x) in [
        (0.0, 1.0),
        (1.0, 0.0),
        (0.0, -1.0),
        (-1.0, 0.0),
        (1e-300, 1e300),
    ] {
        assert!(
            (atan2_exact(y, x) - f64::atan2(y, x)).abs() < 1e-14,
            "{y} {x}"
        );
    }
    assert_eq!(atan2_exact(0.0, 0.0), 0.0);
    assert_eq!(atan2_exact(f64::NAN, 1.0), 0.0);
    assert_eq!(sin_pi(f64::INFINITY), 0.0);
}

#[test]
fn a_speckle_of_no_size_has_no_dots() {
    let p = Pattern {
        kind: PatternKind::Speckle,
        spot: 0.0,
        ..Pattern::default()
    }
    .sanitized();
    let mut points = Points(9);
    for _ in 0..2000 {
        let q = points.point(30.0);
        assert_eq!(p.value(q), 0.0, "{q:?}");
    }
}
