//! Speed of NURBS surface evaluation by degree, and a digest of every bit
//! it gives, so that a faster evaluation is seen to give the same values.
//! Ignored: run in release,
//! `cargo test --release -p craft3d-geom --test nurbs_speed -- --ignored --nocapture`.

use craft3d_geom::NurbsSurface;
use craft3d_math::{Portable, Vec3};
use std::time::Instant;

/// A surface of degrees `(du, dv)` with `nu × nv` control points, uneven
/// knots and weights from a fixed sequence.
fn surface(du: usize, dv: usize, nu: usize, nv: usize) -> NurbsSurface {
    let knots = |d: usize, n: usize| {
        let mut k = vec![0.0; d + 1];
        let inner = n - d - 1;
        for i in 1..=inner {
            let t = i as f64 / (inner + 1) as f64;
            k.push(t + 0.01 * (i as f64 * 1.7).psin() / (inner + 1) as f64);
        }
        k.extend(std::iter::repeat_n(1.0, d + 1));
        k
    };
    let mut ctrl = Vec::new();
    let mut weights = Vec::new();
    for i in 0..nu {
        for j in 0..nv {
            let (x, y) = (i as f64 * 10.0, j as f64 * 7.0);
            let z = (x * 0.05).psin() * 20.0 + (y * 0.07).pcos() * 15.0;
            ctrl.push(Vec3::new(x, y, z));
            weights.push(1.0 + 0.3 * ((i * 7 + j * 3) as f64).psin().abs());
        }
    }
    #[allow(clippy::unwrap_used)]
    NurbsSurface::new(du, dv, knots(du, nu), knots(dv, nv), nu, nv, ctrl, weights).unwrap()
}

fn digest(h: &mut u64, v: (Vec3, Vec3, Vec3)) {
    for x in [v.0, v.1, v.2] {
        for c in [x.x, x.y, x.z] {
            *h = (*h ^ c.to_bits()).wrapping_mul(0x100_0000_01b3);
        }
    }
}

/// The surfaces timed and digested: degrees and control point counts.
const CASES: [(usize, usize, usize, usize); 6] = [
    (3, 3, 12, 12),
    (5, 5, 14, 14),
    (9, 3, 12, 20),
    (3, 9, 20, 12),
    (9, 9, 14, 14),
    (12, 4, 16, 10),
];

/// Every bit of `derivatives`, `at_u`, `at_v` and `eval` at `n × n`
/// points of each case, pinned: evaluation gives the same values however
/// it is made faster. Written on 2026-11-01 when evaluation was sized per
/// direction: at 400 × 400 points the digests of the evaluation before
/// and after were the same (`nurbs_evaluation_speed` prints both sizes).
#[test]
fn evaluation_gives_the_same_bits() {
    let pinned: [u64; 6] = PINNED;
    for (case, want) in CASES.iter().zip(pinned) {
        let (got, _) = run(*case, 41);
        assert_eq!(got, want, "degrees ({}, {}): {got:016x}", case.0, case.1);
    }
}

const PINNED: [u64; 6] = [
    0xee58_f04f_e900_341b,
    0x14ce_73bd_c214_b645,
    0x5817_d124_21df_a64a,
    0x667d_3555_1513_44ff,
    0xec1a_ae40_e3bc_659d,
    0xc7ad_7f68_8191_bd31,
];

/// Digest and timings (ns per point of each way) of one case.
fn run((du, dv, nu, nv): (usize, usize, usize, usize), n: usize) -> (u64, [f64; 4]) {
    {
        let s = surface(du, dv, nu, nv);
        let mut h = 0xcbf2_9ce4_8422_2325_u64;
        let t = Instant::now();
        for i in 0..n {
            let u = i as f64 / (n - 1) as f64;
            for j in 0..n {
                let v = j as f64 / (n - 1) as f64;
                digest(&mut h, s.derivatives(u, v));
            }
        }
        let plain = t.elapsed().as_secs_f64();
        let t = Instant::now();
        for i in 0..n {
            let u = i as f64 / (n - 1) as f64;
            let line = s.at_u(u);
            for j in 0..n {
                let v = j as f64 / (n - 1) as f64;
                digest(&mut h, line.derivatives(v));
            }
        }
        let at_u = t.elapsed().as_secs_f64();
        let t = Instant::now();
        for j in 0..n {
            let v = j as f64 / (n - 1) as f64;
            let line = s.at_v(v);
            for i in 0..n {
                let u = i as f64 / (n - 1) as f64;
                digest(&mut h, line.derivatives(u));
            }
        }
        let at_v = t.elapsed().as_secs_f64();
        let t = Instant::now();
        let mut e = 0.0;
        for i in 0..n {
            let u = i as f64 / (n - 1) as f64;
            for j in 0..n {
                let v = j as f64 / (n - 1) as f64;
                let p = s.eval(u, v);
                e += p.x;
                digest(&mut h, (p, Vec3::ZERO, Vec3::ZERO));
            }
        }
        let eval = t.elapsed().as_secs_f64();
        let per = 1e9 / (n * n) as f64;
        assert!(e.is_finite());
        (h, [plain * per, at_u * per, at_v * per, eval * per])
    }
}

#[test]
#[ignore = "a timing, run in release"]
fn nurbs_evaluation_speed() {
    for case in CASES {
        let (h, [plain, at_u, at_v, eval]) = run(case, 400);
        let (small, _) = run(case, 41);
        println!(
            "degree ({},{}): derivatives {plain:.0} ns, at_u {at_u:.0} ns, at_v {at_v:.0} ns, eval {eval:.0} ns; digest {h:016x} (41 points: 0x{small:016x})",
            case.0, case.1
        );
    }
}
