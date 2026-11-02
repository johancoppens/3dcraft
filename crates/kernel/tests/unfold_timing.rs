//! How long the unfold of the folded sheets and the kayak's sheer panel
//! takes (Rule 12; run in release on the Strix with `--ignored
//! --nocapture`, before and after a change to the unfold): the time of
//! `develop` alone, the median of three runs, and whether it laid the
//! sheet or refused it. Not a gate: it prints, it does not assert.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_geom::{Curve3, NurbsCurve};
use craft3d_kernel::developable::{Rail, RailPiece, connect};
use craft3d_kernel::unfold::develop;
use craft3d_math::{Portable, Tolerance, Vec3};
use craft3d_topo::FaceId;

fn rail(curve: Curve3, range: (f64, f64)) -> Rail {
    Rail {
        pieces: vec![RailPiece {
            curve,
            range,
            reversed: false,
            source: 1,
        }],
    }
}

/// The rails of `tests/developable.rs` › `passing`.
fn passing(sigma: f64, alpha: f64, cut: f64) -> (Rail, Rail) {
    let x = [-1.0, -1.0 / 3.0, 1.0 / 3.0, 1.0];
    let x2 = [1.0, -1.0 / 3.0, -1.0 / 3.0, 1.0];
    let x3 = [-1.0, 1.0, -1.0, 1.0];
    let sin60 = 60f64.to_radians().psin();
    let (c3, c2, c0) = (
        20.0 / sigma.ppowi(3),
        20.0 * sin60 / sigma.ppowi(2),
        40.0 * sin60,
    );
    let knots = vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0];
    let a: Vec<Vec3> = (0..4)
        .map(|i| Vec3::new(100.0 * x[i], alpha * x2[i], 20.0 * x3[i]))
        .collect();
    let b: Vec<Vec3> = (0..4)
        .map(|i| {
            Vec3::new(
                100.0 * x[i],
                20.0 + 10.0 * x2[i],
                c3 * x3[i] + c0 + c2 * x2[i],
            )
        })
        .collect();
    let a = NurbsCurve::polynomial(3, knots.clone(), a).unwrap();
    let b = NurbsCurve::polynomial(3, knots, b).unwrap();
    let part = |(lo, hi): (f64, f64)| (lo, lo + cut * (hi - lo));
    let (ra, rb) = (part(a.domain()), part(b.domain()));
    (rail(Curve3::Nurbs(a), ra), rail(Curve3::Nurbs(b), rb))
}

/// The kayak's sheer rails (`tests/developable_rails/sea_kayak_sheer.json`).
fn sheer() -> (Rail, Rail) {
    let text = std::fs::read_to_string(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/developable_rails/sea_kayak_sheer.json"),
    )
    .unwrap();
    let data: serde_json::Value = serde_json::from_str(&text).unwrap();
    let rail_of = |name: &str| -> Rail {
        let d = &data[name];
        let floats = |v: &serde_json::Value| -> Vec<f64> {
            v.as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_f64().unwrap())
                .collect()
        };
        let ctrl: Vec<Vec3> = d["ctrl"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| {
                let p = floats(p);
                Vec3::new(p[0], p[1], p[2])
            })
            .collect();
        let curve = NurbsCurve::new(
            d["degree"].as_u64().unwrap() as usize,
            floats(&d["knots"]),
            ctrl,
            floats(&d["weights"]),
        )
        .unwrap();
        let range = floats(&d["range"]);
        rail(Curve3::Nurbs(curve), (range[0], range[1]))
    };
    (rail_of("A"), rail_of("B"))
}

#[test]
#[ignore = "timings, run in release on the Strix"]
fn unfold_timings() {
    let tol = Tolerance::new(1e-6, 1e-9).unwrap();
    for (name, (a, b)) in [
        ("folded (two fans)", passing(1.01, 2.0, 0.8)),
        ("folded (bridged)", passing(1.1, 0.0, 1.0)),
        ("kayak sheer", sheer()),
    ] {
        let made = connect(&a, &b, &tol).unwrap();
        let faces: Vec<FaceId> = (0..made.body.faces.len() as u32).map(FaceId).collect();
        let mut times = Vec::new();
        let mut outcome = String::new();
        for _ in 0..3 {
            let started = std::time::Instant::now();
            let result = develop(&made.body, &faces, &tol);
            times.push(started.elapsed().as_secs_f64());
            outcome = match result {
                Ok(dev) => format!("laid flat, {:?}", dev.error()),
                Err(e) => format!("refused: {e}"),
            };
        }
        times.sort_by(f64::total_cmp);
        eprintln!(
            "TIMING {name}: {} faces, {:.3} s (median of 3), {outcome}",
            faces.len(),
            times[1]
        );
    }
}
