//! Solids with faces that close in a point whose meridians leave it
//! unevenly, through the command layer: read from a STEP file (the one
//! way such a body reaches a document today: nothing in the kernel
//! builds a tip yet, and any modeller that lofts to a point writes such
//! faces) and measured by `document.measure` with its bound, against
//! the closed forms (docs/design/topology.md › *A rail beside its
//! border*).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::plain::run;
use common::{session_in, workdir};
use craft3d_io::{StepBody, StepOptions, export_step};
use craft3d_math::Frame;
use craft3d_testkit::tips::{PARABOLA, ROUND, polygon_area, prism_tip};
use serde_json::json;

/// A body: its name, its section, its scale, the tip's foot, its length,
/// its form (fullness and share of base times length), and how wide the
/// measure's bound may be as a share of the volume.
type Case<'a> = (&'a str, &'a [[f64; 2]], f64, [f64; 2], f64, (f64, f64), f64);

#[test]
fn imported_tips_measure_as_their_closed_forms_within_their_bounds() {
    let dir = workdir("pole-measure", "import");
    let l = [
        [0.0, 0.0],
        [6.0, 0.0],
        [6.0, 2.0],
        [2.0, 2.0],
        [2.0, 6.0],
        [0.0, 6.0],
    ];
    let blade = [[-0.5, -5e-4], [0.5, -5e-4], [0.5, 5e-4], [-0.5, 5e-4]];
    let triangle = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];
    // Name, section, scale, the tip's foot, its length, its form, and how
    // wide the bound may be as a share of the volume. Before 2026-11-14,
    // by `topo::mass_bound` on the same bodies: the blade 2.8e-4 of its
    // volume short within a bound of 8.0e-3, the L's 3.0e-6 within
    // 1.4e-5 and 6.7e-7 within 9.1e-7, the triangle 7.7e-7 within 1.1e-6.
    let cases: [Case; 4] = [
        ("blade", &blade, 1000.0, [0.0, 0.0], 1e-3, ROUND, 1e-8),
        ("l-short", &l, 1.0, [0.03, 0.03], 0.5, PARABOLA, 1e-10),
        ("l-long", &l, 1.0, [0.01, 0.01], 3.0, PARABOLA, 1e-10),
        (
            "triangle",
            &triangle,
            1.0,
            [0.003, 0.003],
            0.5,
            PARABOLA,
            1e-10,
        ),
    ];
    let mut measured = Vec::new();
    for (name, corners, k, foot, length, (fullness, share), within) in cases {
        let (_, body) = prism_tip(&Frame::WORLD, k, corners, foot, length, fullness).unwrap();
        let (bytes, _) =
            export_step(&[StepBody::new("Tip", &body)], &StepOptions::default()).unwrap();
        let file = format!("tip-{name}.step");
        std::fs::write(dir.join(&file), bytes).unwrap();
        let mut s = session_in(&dir);
        run(&mut s, "file.new", json!({"units": "mm"}));
        let result = run(&mut s, "file.import", json!({"path": file}));
        // Every one of them was read before the borders were looked at
        // (and by 0.1.x): none may be refused for a foot put right.
        assert_eq!(result["bodies"][0]["status"], "ok", "{name}: {result}");
        let measure = run(&mut s, "document.measure", json!({"bound": true}));
        let want = share * polygon_area(corners) * length * k * k * k;
        let volume = measure["volume"].as_f64().unwrap();
        let bound = measure["volumeBound"].as_f64().unwrap();
        println!(
            "{name}: imported; the volume {:e} of itself off, its bound {:e}",
            (volume - want).abs() / want,
            bound / want
        );
        assert!(
            (volume - want).abs() <= bound + 1e-13 * want,
            "{name}: {volume} against {want}, outside the bound {bound}"
        );
        assert!(
            bound <= within * want,
            "{name}: a bound of {bound} on {want}"
        );
        measured.push(name);
    }
    assert_eq!(measured.len(), 4, "{measured:?}");
}
