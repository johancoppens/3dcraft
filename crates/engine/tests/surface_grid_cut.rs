//! Blocks cut by sheets through grids of points, held to oracles that do
//! not go through the boolean (docs/design/surfacing.md › *A sheet through
//! a grid of points* › *A solid from the sheet*; docs/design/booleans.md
//! › *Groups of spans*): the volume under the sheet over the block's
//! footprint by exact quadrature per span (the divergence theorem on the
//! sheet the feature interpolates, worked out here from the same points),
//! the part above it and the identity V(A) = V(A ∩ S) + V(A − S), the
//! union of the two parts the block again, vertical rays at the nodes on
//! the sheet's points, the deep check, and a closed mesh. The ordinary
//! cases of a terrain: a block of the sheet's footprint (its border on
//! the block's sides) and a block within it, at three scales, and a sheet
//! that folds back over itself. The large sheets (up to the grid's cap,
//! 512 × 512 points) are the same test in release, ignored by default.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::{body, built, new_document, run, session_in, workdir};
use craft3d_engine::Session;
use craft3d_geom::grid::{Spacing, interpolate};
use craft3d_geom::{NurbsSurface, Surface};
use craft3d_math::{Portable, Vec3};
use serde_json::{Value, json};
use std::time::Instant;

/// Hills and a ripple over `[0, 1]²` (as `grid_timing.rs` has them), in
/// units of the sheet's width.
fn height(x: f64, y: f64) -> f64 {
    let mut h = 0.0;
    for (cx, cy, r, a) in [
        (0.3, 0.4, 0.15, 0.18),
        (0.7, 0.6, 0.1, 0.25),
        (0.5, 0.2, 0.2, 0.1),
        (0.8, 0.85, 0.07, 0.12),
    ] {
        h += a * (-((x - cx) * (x - cx) + (y - cy) * (y - cy)) / (2.0 * r * r)).pexp();
    }
    h + 0.02 * (9.0 * x + 3.0 * y).psin() * (7.0 * y).pcos()
}

/// A sheet's points: `columns` × `rows`, `width` wide (the step the same
/// both ways), row by row.
struct Sheet {
    columns: usize,
    rows: usize,
    points: Vec<Vec3>,
}

impl Sheet {
    /// The terrain of [`height`].
    fn terrain(columns: usize, rows: usize, width: f64) -> Sheet {
        let step = width / (columns - 1) as f64;
        let mut points = Vec::with_capacity(columns * rows);
        for r in 0..rows {
            for c in 0..columns {
                let (x, y) = (c as f64 * step, r as f64 * step);
                points.push(Vec3::new(x, y, width * height(x / width, y / width)));
            }
        }
        Sheet {
            columns,
            rows,
            points,
        }
    }

    /// A sheet that runs along x, turns back under itself and on again,
    /// rising all the while (an overhang: a vertical line meets it three
    /// times in the middle, and it never meets itself), its first and last
    /// columns at x = 0 and x = `width`, rising and falling along y.
    fn folded(columns: usize, rows: usize, width: f64) -> Sheet {
        let mut points = Vec::with_capacity(columns * rows);
        for r in 0..rows {
            for c in 0..columns {
                let s = c as f64 / (columns - 1) as f64;
                let t = r as f64 / (rows - 1) as f64;
                let a = 2.0 * std::f64::consts::PI * s;
                let x = width * (s + 0.25 * a.psin());
                let z = width * (0.2 + 0.4 * s + 0.05 * (3.0 * t).psin());
                points.push(Vec3::new(x, width * t * 0.8, z));
            }
        }
        Sheet {
            columns,
            rows,
            points,
        }
    }

    fn at(&self, r: usize, c: usize) -> Vec3 {
        self.points[r * self.columns + c]
    }

    /// The points as `feature.surface.grid` takes them.
    fn json(&self) -> Value {
        Value::Array(
            (0..self.rows)
                .map(|r| {
                    Value::Array(
                        (0..self.columns)
                            .map(|c| {
                                let p = self.at(r, c);
                                json!([p.x, p.y, p.z])
                            })
                            .collect(),
                    )
                })
                .collect(),
        )
    }

    /// The sheet the feature makes from these points at even spacing.
    fn surface(&self, tolerance: f64) -> NurbsSurface {
        interpolate(
            &self.points,
            self.rows,
            self.columns,
            Spacing::Even,
            tolerance,
            1e-6,
        )
        .unwrap()
        .surface
    }
}

/// The volume between the sheet and the plane `z = floor` over the part of
/// its parameters `u × v`: `∫∫ (Z − floor) (X_u Y_v − X_v Y_u) du dv` (the
/// divergence theorem with the field `(0, 0, z − floor)` on the solid
/// behind the sheet, whose other faces are vertical or at the floor), by
/// 6-point Gauss–Legendre quadrature over each knot span cut to the part,
/// exact for the polynomial integrand of a bicubic (of degree 8 each way).
/// Its sign is the sheet's orientation's.
fn volume_under(surface: &NurbsSurface, floor: f64, u: (f64, f64), v: (f64, f64)) -> f64 {
    let nodes = [
        (-0.932_469_514_203_152, 0.171_324_492_379_170),
        (-0.661_209_386_466_265, 0.360_761_573_048_139),
        (-0.238_619_186_083_197, 0.467_913_934_572_691),
        (0.238_619_186_083_197, 0.467_913_934_572_691),
        (0.661_209_386_466_265, 0.360_761_573_048_139),
        (0.932_469_514_203_152, 0.171_324_492_379_170),
    ];
    let spans = |knots: &[f64], (lo, hi): (f64, f64)| -> Vec<(f64, f64)> {
        knots
            .windows(2)
            .filter(|w| w[1] > w[0])
            .map(|w| (w[0].max(lo), w[1].min(hi)))
            .filter(|(a, b)| b > a)
            .collect()
    };
    let (su, sv) = (spans(surface.knots_u(), u), spans(surface.knots_v(), v));
    let mut total = 0.0;
    for &(u0, u1) in &su {
        for &(v0, v1) in &sv {
            let mut cell = 0.0;
            for &(a, wa) in &nodes {
                for &(b, wb) in &nodes {
                    let u = 0.5 * (u0 + u1) + 0.5 * (u1 - u0) * a;
                    let v = 0.5 * (v0 + v1) + 0.5 * (v1 - v0) * b;
                    let (p, du, dv) = surface.derivatives(u, v);
                    cell += wa * wb * (p.z - floor) * (du.x * dv.y - dv.x * du.y);
                }
            }
            total += cell * 0.25 * (u1 - u0) * (v1 - v0);
        }
    }
    total
}

/// The parameters of `surface` over `[x0, x1] × [y0, y1]`, for a sheet
/// whose x and y are linear in its parameters (a terrain at even
/// spacing: interpolation reproduces the points' even steps).
fn parameters_over(
    surface: &NurbsSurface,
    (x0, x1): (f64, f64),
    (y0, y1): (f64, f64),
) -> ((f64, f64), (f64, f64)) {
    let s = Surface::Nurbs(surface.clone());
    let ((u0, u1), (v0, v1)) = s.domain();
    let (p00, p10, p01) = (s.eval(u0, v0), s.eval(u1, v0), s.eval(u0, v1));
    // Which parameter runs along x, and the share of it at a coordinate.
    let share = |q: f64, start: f64, end: f64| (q - start) / (end - start);
    let along = |a: f64, b: f64, lo: f64, hi: f64, start: f64, end: f64| {
        let (t0, t1) = (share(a, start, end), share(b, start, end));
        (lo + t0.min(t1) * (hi - lo), lo + t0.max(t1) * (hi - lo))
    };
    if (p10.x - p00.x).abs() > (p10.y - p00.y).abs() {
        (
            along(x0, x1, u0, u1, p00.x, p10.x),
            along(y0, y1, v0, v1, p00.y, p01.y),
        )
    } else {
        (
            along(y0, y1, u0, u1, p00.y, p10.y),
            along(x0, x1, v0, v1, p00.x, p01.x),
        )
    }
}

fn number(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("not a number: {v}"))
}

/// A block of `size` at `position`, named.
fn block(s: &mut Session, name: &str, size: [f64; 3], position: [f64; 3]) {
    built(
        s,
        "feature.block",
        json!({"size": size, "position": position, "name": name}),
    );
}

/// What a cut showed: the volumes under and over the sheet, and the time
/// the three booleans took.
struct Cut {
    under: f64,
    over: f64,
    seconds: f64,
}

/// Cuts a block of `size` at `position` by the sheet `sheet` (a feature of
/// that name) both ways and unites the two parts again, holding each to
/// the oracles: the volume under the sheet (`expected`, when given) to
/// `relative`, the two parts to the block, their union to the block, rays
/// at every `ray_step`-th node within the block on the sheet's point, the
/// deep check and closed meshes.
#[allow(clippy::too_many_arguments)]
fn cut_both_ways(
    s: &mut Session,
    sheet: &Sheet,
    name: &str,
    size: [f64; 3],
    position: [f64; 3],
    expected: Option<f64>,
    relative: f64,
    ray_step: usize,
    label: &str,
) -> Cut {
    let [a, b, u] = [
        format!("{label} A"),
        format!("{label} B"),
        format!("{label} under"),
    ];
    let (o, w) = (format!("{label} over"), format!("{label} whole"));
    block(s, &a, size, position);
    block(s, &b, size, position);
    let t = Instant::now();
    built(
        s,
        "feature.boolean",
        json!({"op": "intersect", "target": a, "tool": name, "keepTool": true, "name": u}),
    );
    built(
        s,
        "feature.boolean",
        json!({"op": "subtract", "target": b, "tool": name, "keepTool": true, "name": o}),
    );
    let mut seconds = t.elapsed().as_secs_f64();
    let whole = size[0] * size[1] * size[2];
    let (under_body, over_body) = (body(s, &u), body(s, &o));
    for part in [&under_body, &over_body] {
        assert_eq!(part["kind"], "solid", "{label}: {part}");
        assert_eq!(part["valid"], true, "{label}: {part}");
    }
    let (under, over) = (number(&under_body["volume"]), number(&over_body["volume"]));
    if let Some(expected) = expected {
        assert!(
            (under - expected).abs() <= relative * whole,
            "{label}: the volume under the sheet {under}, by quadrature {expected} (off by {:e} of the block)",
            (under - expected) / whole
        );
    }
    // A = (A ∩ S) ∪ (A − S), in volume, and as a body.
    assert!(
        (under + over - whole).abs() <= relative * whole,
        "{label}: under {under} + over {over} = {}, the block {whole}",
        under + over
    );
    // Rays down onto the nodes within the block meet the solid under the
    // sheet at the sheet's point (first, where the sheet does not fold
    // over the node).
    let top = position[2] + size[2] + 1.0;
    let mut rays = 0;
    for r in (0..sheet.rows).step_by(ray_step) {
        for c in (0..sheet.columns).step_by(ray_step) {
            let p = sheet.at(r, c);
            let inside = p.x > position[0]
                && p.x < position[0] + size[0]
                && p.y > position[1]
                && p.y < position[1] + size[1];
            if !inside {
                continue;
            }
            let hit = run(
                s,
                "measure.ray",
                json!({"from": [p.x, p.y, top], "direction": "-z", "of": u}),
            );
            let zs: Vec<f64> = hit["hits"]
                .as_array()
                .unwrap()
                .iter()
                .map(|h| number(&h["point"][2]))
                .collect();
            assert!(
                zs.iter().any(|z| (z - p.z).abs() <= 1e-9 * size[0]),
                "{label}: no hit at node ({r}, {c}) at {}: {zs:?}",
                p.z
            );
            rays += 1;
        }
    }
    assert!(rays > 0, "{label}: no node within the block");
    let check = run(s, "document.check", json!({"deep": true}));
    assert_eq!(check["ok"], true, "{label}: {check}");
    let mesh = run(
        s,
        "file.export",
        json!({"path": format!("{label}.stl"), "bodies": [u, o]}),
    );
    assert_eq!(mesh["closed"], 2, "{label}: {mesh}");
    let t = Instant::now();
    built(
        s,
        "feature.boolean",
        json!({"op": "union", "target": u.clone(), "tool": o.clone(), "keepTool": true, "name": w}),
    );
    seconds += t.elapsed().as_secs_f64();
    let united = body(s, &w);
    assert_eq!(united["valid"], true, "{label}: {united}");
    let back = number(&united["volume"]);
    assert!(
        (back - whole).abs() <= relative * whole,
        "{label}: the two parts united {back}, the block {whole}"
    );
    Cut {
        under,
        over,
        seconds,
    }
}

/// A terrain of 40 × 33 points (37 × 30 = 1110 spans, in groups) cut by a
/// block of its footprint and by one within it, at three scales: the
/// volumes, the identity, the union, rays, the deep check and the meshes.
#[test]
fn a_terrain_cut_by_a_block_holds_to_the_volume_under_it_at_three_scales() {
    for k in [1e-3, 1.0, 1e6] {
        let dir = workdir("surface-grid-cut", &format!("terrain-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        let width = 100.0 * k;
        let sheet = Sheet::terrain(40, 33, width);
        built(
            &mut s,
            "feature.surface.grid",
            json!({"points": sheet.json(), "spacing": "even", "name": "Sheet"}),
        );
        let surface = sheet.surface(1e-6 * k);
        let last = sheet.at(sheet.rows - 1, sheet.columns - 1);
        let (x1, y1) = (last.x, last.y);
        let floor = -0.1 * width;
        let height = 0.6 * width;
        for (label, size, position) in [
            ("footprint", [x1, y1, height], [0.0, 0.0, floor]),
            (
                "middle",
                [0.5 * x1, 0.5 * y1, height],
                [0.25 * x1, 0.25 * y1, floor],
            ),
        ] {
            let (u, v) = parameters_over(
                &surface,
                (position[0], position[0] + size[0]),
                (position[1], position[1] + size[1]),
            );
            let expected = volume_under(&surface, floor, u, v).abs();
            cut_both_ways(
                &mut s,
                &sheet,
                "Sheet",
                size,
                position,
                Some(expected),
                1e-9,
                4,
                &format!("{label} {k}"),
            );
        }
    }
}

/// A sheet that turns back over itself (a vertical line through its
/// middle meets it three times), its border on the sides of a block of
/// its footprint: the part behind it has the volume the divergence
/// theorem gives on the sheet, the two parts make the block, and so on.
#[test]
fn a_sheet_folded_over_itself_cuts_a_block_of_its_footprint() {
    let dir = workdir("surface-grid-cut", "folded");
    let mut s = session_in(&dir);
    new_document(&mut s, 1.0);
    let width = 100.0;
    let sheet = Sheet::folded(41, 21, width);
    built(
        &mut s,
        "feature.surface.grid",
        json!({"points": sheet.json(), "spacing": "even", "name": "Fold"}),
    );
    let surface = sheet.surface(1e-6);
    let s_ = Surface::Nurbs(surface.clone());
    let domain = s_.domain();
    let floor = 0.0;
    let expected = volume_under(&surface, floor, domain.0, domain.1).abs();
    let last = sheet.at(sheet.rows - 1, sheet.columns - 1);
    let cut = cut_both_ways(
        &mut s,
        &sheet,
        "Fold",
        [width, last.y, 0.8 * width],
        [0.0, 0.0, floor],
        Some(expected),
        1e-9,
        5,
        "folded",
    );
    assert!(cut.under > 0.0 && cut.over > 0.0);
}

/// The process's peak resident memory so far, in MiB (Linux).
fn peak_mib() -> f64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmHWM:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|kib| kib.parse::<f64>().ok())
        })
        .map_or(f64::NAN, |kib| kib / 1024.0)
}

/// The terrain at `columns` × `rows` points 100 mm wide, at a tolerance of
/// 1e-3 mm, cut by a block of its footprint and by one over its middle
/// half each way, both ways and united again, held to the oracles; prints
/// the times and the peak memory. In release, one size a process.
fn large(columns: usize, rows: usize) {
    let dir = workdir("surface-grid-cut", &format!("large-{columns}x{rows}"));
    let mut s = session_in(&dir);
    run(
        &mut s,
        "file.new",
        json!({"units": "mm", "tolerance": 0.001}),
    );
    let width = 100.0;
    let sheet = Sheet::terrain(columns, rows, width);
    let t = Instant::now();
    built(
        &mut s,
        "feature.surface.grid",
        json!({"points": sheet.json(), "spacing": "even", "name": "Sheet"}),
    );
    println!(
        "{columns} x {rows}: sheet {:.3} s, peak {:.0} MiB",
        t.elapsed().as_secs_f64(),
        peak_mib()
    );
    let surface = sheet.surface(1e-3);
    let last = sheet.at(sheet.rows - 1, sheet.columns - 1);
    let (x1, y1) = (last.x, last.y);
    for (label, size, position) in [
        ("footprint", [x1, y1, 60.0], [0.0, 0.0, -10.0]),
        (
            "middle",
            [0.5 * x1, 0.5 * y1, 60.0],
            [0.25 * x1, 0.25 * y1, -10.0],
        ),
    ] {
        let (u, v) = parameters_over(
            &surface,
            (position[0], position[0] + size[0]),
            (position[1], position[1] + size[1]),
        );
        let expected = volume_under(&surface, position[2], u, v).abs();
        let t = Instant::now();
        let cut = cut_both_ways(
            &mut s,
            &sheet,
            "Sheet",
            size,
            position,
            Some(expected),
            1e-9,
            32,
            &format!("{columns}x{rows} {label}"),
        );
        println!(
            "{columns} x {rows} {label}: three booleans {:.3} s, with the oracles {:.3} s; under {} (quadrature {expected}, off {:e}), over {}; peak {:.0} MiB",
            cut.seconds,
            t.elapsed().as_secs_f64(),
            cut.under,
            (cut.under - expected) / expected,
            cut.over,
            peak_mib()
        );
    }
}

#[test]
#[ignore]
fn large_0256() {
    large(256, 256);
}

#[test]
#[ignore]
fn large_0400x0300() {
    large(400, 300);
}

#[test]
#[ignore]
fn large_0512() {
    large(512, 512);
}
