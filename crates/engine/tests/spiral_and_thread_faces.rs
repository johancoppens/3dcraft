//! Two faces a field test could not mesh (R1, 2026-10-25), meshed now:
//!
//! - **A band between two spirals.** A profile bounded by an inner and an
//!   outer `sketch.curve` spiral of five and a half turns and two short
//!   lines, extruded (a wound tape spring, 0.15 mm thick on turns 0.5 mm
//!   apart): the feature built, but the side faces could not be meshed
//!   ("its boundary is not closed in parameter space"), so the body was not
//!   drawn and the thumbnail of a save failed. The search for the foot of
//!   a boundary point on a spline surface starts from a grid of 16 samples
//!   across, which on a side winding five times round lies between turns;
//!   it ended on a turn beside the point's own.
//! - **A thread of more than three turns joined to a turned neck.** A
//!   trapezoid swept along a helix and joined to a revolved neck: the
//!   neck's face has its two rims and the thread's foot as a hole that
//!   winds round the axis as many times as the thread. No straight seam
//!   passes the foot, and the seam in pieces, whose layout reached one turn
//!   either way, found no way from the foot's end to the upper rim above
//!   three turns ("no way for a seam between its loops").
//!
//! Checked against closed-form volumes at three scales, the meshes of the
//! exports watertight, with the volume of the exact body, and a picture and
//! a save with its thumbnail.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::{body, built, run};
use craft3d_engine::Session;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::f64::consts::PI;
use std::path::Path;

const SCALES: [f64; 3] = [1e-3, 1.0, 1e3];

fn new_document(s: &mut Session, k: f64) {
    run(
        s,
        "file.new",
        json!({"units": "mm", "tolerance": 1e-6 * k.max(1.0)}),
    );
}

#[track_caller]
fn close(found: f64, expected: f64, relative: f64, what: &str) {
    assert!(
        (found - expected).abs() <= relative * expected.abs(),
        "{what}: found {found}, expected {expected} (off by {:e})",
        (found - expected).abs() / expected.abs()
    );
}

/// The triangles of a binary STL file, welded by the bits of their corners:
/// whether every side is used by exactly two triangles, once each way, and
/// the volume the triangles enclose.
fn stl_closed_volume(path: &Path) -> (bool, f64) {
    let bytes = std::fs::read(path).unwrap();
    let count = u32::from_le_bytes(bytes[80..84].try_into().unwrap()) as usize;
    assert_eq!(bytes.len(), 84 + 50 * count, "{}", path.display());
    let mut ids: BTreeMap<[u32; 3], usize> = BTreeMap::new();
    let mut sides: BTreeMap<(usize, usize), i32> = BTreeMap::new();
    let mut volume = 0.0;
    for t in 0..count {
        let at = 84 + 50 * t + 12;
        let corner = |c: usize| -> ([u32; 3], [f64; 3]) {
            let mut bits = [0_u32; 3];
            let mut value = [0.0_f64; 3];
            for (k, (b, v)) in bits.iter_mut().zip(&mut value).enumerate() {
                let o = at + 12 * c + 4 * k;
                *b = u32::from_le_bytes(bytes[o..o + 4].try_into().unwrap());
                *v = f64::from(f32::from_bits(*b));
            }
            (bits, value)
        };
        let corners = [corner(0), corner(1), corner(2)];
        let next = ids.len();
        let mut index = [0_usize; 3];
        for (i, (bits, _)) in corners.iter().enumerate() {
            index[i] = *ids.entry(*bits).or_insert(next + i);
        }
        let [a, b, c] = corners.map(|(_, v)| v);
        volume += (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
            + a[2] * (b[0] * c[1] - b[1] * c[0]))
            / 6.0;
        for (p, q) in [(0, 1), (1, 2), (2, 0)] {
            let (p, q) = (index[p], index[q]);
            if p == q {
                continue;
            }
            *sides.entry((p.min(q), p.max(q))).or_default() += if p < q { 1 } else { -1 };
        }
    }
    // A side used once each way sums to 0; every side of a closed mesh
    // is used twice (counted by the map's entries: none used once).
    (sides.values().all(|&n| n == 0), volume)
}

/// The body `name` is valid, has `volume` (within `slack`), passes the deep
/// check, is drawn, exports as a watertight STL holding its volume (to its
/// area times the chordal tolerance of the mesh), and as 3MF and STEP; a
/// save draws its thumbnail.
#[track_caller]
fn assert_meshed(s: &mut Session, dir: &Path, name: &str, (volume, slack): (f64, f64)) {
    let found = body(s, name);
    assert_eq!(found["valid"], true, "{found}");
    close(
        found["volume"].as_f64().unwrap(),
        volume,
        slack / volume,
        name,
    );
    let check = run(s, "document.check", json!({"deep": true}));
    assert_eq!(check["ok"], true, "{check}");
    let picture = run(s, "view.render", json!({"width": 200, "height": 150}));
    assert!(picture["triangles"].as_u64().unwrap() > 0, "{picture}");
    let stl = run(s, "file.export", json!({"path": "band.stl"}));
    assert_eq!(stl["closed"], stl["bodies"], "{stl}");
    let (closed, meshed) = stl_closed_volume(&dir.join("band.stl"));
    assert!(closed, "{name}: the STL is not closed");
    // Every point of the mesh within the tolerance of the surface: the
    // volume between them within the area times that (and the rounding to
    // f32 of the file).
    let tolerance = stl["tolerance"].as_f64().unwrap();
    let area = found["area"].as_f64().unwrap();
    close(
        meshed,
        volume,
        (area * tolerance + slack) / volume + 1e-6,
        "the STL's volume",
    );
    let three = run(s, "file.export", json!({"path": "band.3mf"}));
    assert_eq!(three["closed"], three["bodies"], "{three}");
    let step = run(s, "file.export", json!({"path": "band.step"}));
    assert_eq!(step["solids"], step["bodies"], "{step}");
    let saved = run(
        s,
        "file.save",
        json!({"path": "band.3dcraft", "thumbnail": true}),
    );
    assert_eq!(saved["thumbnail"], true, "{saved}");
    assert!(saved.get("thumbnailError").is_none(), "{saved}");
}

/// A band between the spirals `r = a + c t / 2π` and `r = a + w + c t / 2π`
/// for `t` from 0 to `turns · 2π`, closed by two lines and extruded by `h`:
/// its area is `∫ (r_o² − r_i²) / 2 dt = w/2 · ((2a + w) T + c T² / 2π)`.
/// With how far the body may be from it: each spiral is fitted within
/// half the tolerance of its formula, along its length (about `∫ r dt`).
fn spiral_band(s: &mut Session, k: f64, turns: f64) -> (f64, f64) {
    let (a, w, c, h) = (3.0 * k, 0.15 * k, 0.5 * k, 3.3 * k);
    let t = turns * 2.0 * PI;
    run(
        s,
        "sketch.create",
        json!({"plane": "xy", "offset": format!("{} mm", 2.0 * k), "name": "Coil"}),
    );
    for inner in [a, a + w] {
        let r = format!("({inner} mm + {c} mm * t / (2 * pi))");
        run(
            s,
            "sketch.curve",
            json!({"sketch": "Coil", "x": format!("{r} * cos(t)"), "y": format!("{r} * sin(t)"),
                   "t": ["0", format!("{} * pi", 2.0 * turns)]}),
        );
    }
    run(
        s,
        "sketch.line",
        json!({"sketch": "Coil", "from": "e1", "to": "e4"}),
    );
    run(
        s,
        "sketch.line",
        json!({"sketch": "Coil", "from": "e2", "to": "e5"}),
    );
    built(
        s,
        "feature.extrude",
        json!({"sketch": "Coil", "distance": format!("{h} mm"), "name": "Band"}),
    );
    let volume = h * w / 2.0 * ((2.0 * a + w) * t + c * t * t / (2.0 * PI));
    let lengths = 1.01 * ((2.0 * a + w) * t + c * t * t / (2.0 * PI));
    let tolerance = 1e-6 * k.max(1.0);
    (volume, h * 0.5 * tolerance * lengths + 1e-9 * volume)
}

#[test]
fn a_band_between_two_spirals_of_many_turns_is_meshed_at_every_scale() {
    for k in SCALES {
        let dir = common::workdir("spiral-band", &format!("{k:e}"));
        let mut s = common::session_in(&dir);
        new_document(&mut s, k);
        let volume = spiral_band(&mut s, k, 5.5);
        assert_meshed(&mut s, &dir, "Band", volume);
    }
}

#[test]
fn a_band_between_two_spirals_moved_away_is_meshed() {
    // The field test's: the band moved off the origin by a transform.
    let dir = common::workdir("spiral-band", "moved");
    let mut s = common::session_in(&dir);
    new_document(&mut s, 1.0);
    let volume = spiral_band(&mut s, 1.0, 5.5);
    built(
        &mut s,
        "feature.transform",
        json!({"source": "Band", "translate": ["45 mm", 0, 0], "name": "Moved"}),
    );
    assert_meshed(&mut s, &dir, "Moved", volume);
}

/// A neck of radius `R` and height `H` turned from a rectangle, and a
/// thread of `turns` turns joined to it: a trapezoid with 45° flanks, `b`
/// wide at its root and `td` deep, sunk `e` into the neck, swept along a
/// helix of pitch `p` (the screw jar's thread, `3dcraft-jigs`). The
/// volume: the neck's, and outside it the trapezoid carried by its screw
/// motion, which sweeps per turn `2π ∫ r dA` (Pappus):
/// `2π · turns · ∫₀^td (R + x)(b − 2x) dx`; with how far the body may be
/// from it.
fn threaded_neck(s: &mut Session, k: f64, turns: f64) -> (f64, f64) {
    let (r, p, b, td, e) = (32.0 * k, 4.0 * k, 2.8 * k, 1.12 * k, 0.15 * k);
    let z0 = 3.0 * k;
    let h = z0 + turns * p + b + 3.0 * k;
    let mm = |x: f64| format!("{x} mm");
    run(s, "sketch.create", json!({"plane": "zx", "name": "Neck"}));
    run(
        s,
        "sketch.polyline",
        json!({"sketch": "Neck", "closed": true,
               "points": [[mm(0.0), mm(0.0)], [mm(0.0), mm(r)], [mm(h), mm(r)], [mm(h), mm(0.0)]]}),
    );
    built(
        s,
        "feature.revolve",
        json!({"sketch": "Neck", "axis": "x", "name": "NeckBody"}),
    );
    run(
        s,
        "datum.helix",
        json!({"name": "Path", "axis": "z", "start": [mm(r), mm(0.0), mm(z0)],
               "pitch": mm(p), "turns": turns}),
    );
    run(s, "sketch.create", json!({"plane": "zx", "name": "Thread"}));
    run(
        s,
        "sketch.polyline",
        json!({"sketch": "Thread", "closed": true, "points": [
            [mm(z0 - e), mm(r - e)], [mm(z0 + td), mm(r + td)],
            [mm(z0 + b - td), mm(r + td)], [mm(z0 + b + e), mm(r - e)]]}),
    );
    built(
        s,
        "feature.sweep",
        json!({"sketch": "Thread", "path": "Path", "join": "NeckBody", "name": "Jar"}),
    );
    let moment = r * b * td - r * td * td + b * td * td / 2.0 - 2.0 / 3.0 * td * td * td;
    let volume = PI * r * r * h + 2.0 * PI * turns * moment;
    // The helical faces are within the tolerance of the screw motion: the
    // body within its area (overestimated: the neck's, and twice the
    // thread's outline per turn) times that.
    let area = 2.0 * PI * r * (r + h) + turns * 2.0 * PI * (r + td) * 2.0 * (b + 3.0 * td);
    let tolerance = 1e-6 * k.max(1.0);
    (volume, area * tolerance + 1e-9 * volume)
}

#[test]
fn a_thread_of_more_than_three_turns_joined_to_a_turned_neck_is_meshed() {
    for k in SCALES {
        let dir = common::workdir("threaded-neck", &format!("{k:e}"));
        let mut s = common::session_in(&dir);
        new_document(&mut s, k);
        let volume = threaded_neck(&mut s, k, 4.0);
        assert_meshed(&mut s, &dir, "Jar", volume);
    }
}

#[test]
fn threads_of_three_to_eight_turns_on_a_turned_neck_are_meshed() {
    for turns in [3.0, 3.25, 5.5, 8.0] {
        let dir = common::workdir("threaded-neck", &format!("turns-{turns}"));
        let mut s = common::session_in(&dir);
        new_document(&mut s, 1.0);
        let volume = threaded_neck(&mut s, 1.0, turns);
        assert_meshed(&mut s, &dir, "Jar", volume);
    }
}

/// What the tests above build, for a look: the pictures are left in the
/// test's scratch directory (`spiral-and-thread-pictures`).
#[test]
fn pictures_of_the_band_and_the_jar() {
    let dir = common::workdir("spiral-and-thread", "pictures");
    let mut s = common::session_in(&dir);
    new_document(&mut s, 1.0);
    spiral_band(&mut s, 1.0, 5.5);
    let band: Value = run(&mut s, "view.render", json!({"path": "band.png"}));
    assert!(band["triangles"].as_u64().unwrap() > 0, "{band}");
    let mut s = common::session_in(&dir);
    new_document(&mut s, 1.0);
    threaded_neck(&mut s, 1.0, 5.5);
    let jar = run(&mut s, "view.render", json!({"path": "jar.png"}));
    assert!(jar["triangles"].as_u64().unwrap() > 0, "{jar}");
}
