//! The plate and sheet-metal hunts (`crates/kernel/tests/hunt_sheet_*.rs`)
//! through the commands a user calls, for a seeded sample: sheet-metal
//! parts by `feature.sheetMetal` (a rectangle, one to four flanges at
//! random angles, radii, K-factors and lengths, sometimes a hole of a
//! circle or of arcs of one circle), their volume against the closed form
//! (`T` times the flats plus `θ w T (R + T/2)` per bend), their flat
//! pattern exported to DXF and SVG through `file.export` and read back
//! here: the outline's area by Green's theorem on the DXF's lines, arcs
//! and circles against the closed form (the flats plus `w θ (R + K T)` per
//! bend) and the export's `flatArea`, the DXF's and the SVG's box against
//! the pattern's closed-form extent; and discs of arcs of one circle drawn
//! with `sketch.arc`, filled and plated with `feature.plate`, against
//! `π r² T`. At 1e-3, 1 and 1e6.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::plain::{body, new_document, run, session_in, workdir};
use craft3d_math::Portable;
use serde_json::{Value, json};
use std::f64::consts::{PI, TAU};

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

/// A seeded generator (SplitMix64).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn between(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.unit()
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

/// One entity of the DXF's `outline` layer: its ends and its part of
/// `∮ (x dy − y dx) / 2` run from its start to its end.
struct Piece {
    from: [f64; 2],
    to: [f64; 2],
    green: f64,
    closed: bool,
}

/// The pieces of `layer` in an ASCII DXF, and the box of their points
/// (arcs by their extreme points), read here from the group codes.
fn dxf_pieces(text: &str, layer: &str) -> (Vec<Piece>, [f64; 4]) {
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    let mut pieces = Vec::new();
    let mut bbox = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    let mut add = |p: [f64; 2]| {
        bbox[0] = bbox[0].min(p[0]);
        bbox[1] = bbox[1].min(p[1]);
        bbox[2] = bbox[2].max(p[0]);
        bbox[3] = bbox[3].max(p[1]);
    };
    let mut i = 0;
    let mut in_entities = false;
    while i + 1 < lines.len() {
        let (code, value) = (lines[i], lines[i + 1]);
        if code == "2" && value == "ENTITIES" {
            in_entities = true;
        }
        if in_entities && code == "0" && matches!(value, "LINE" | "ARC" | "CIRCLE") {
            let kind = value;
            let mut groups: Vec<(String, String)> = Vec::new();
            let mut j = i + 2;
            while j + 1 < lines.len() && lines[j] != "0" {
                groups.push((lines[j].to_string(), lines[j + 1].to_string()));
                j += 2;
            }
            let get = |c: &str| -> f64 {
                groups
                    .iter()
                    .find(|(k, _)| k == c)
                    .map(|(_, v)| v.parse().unwrap())
                    .unwrap_or(f64::NAN)
            };
            let on = groups.iter().any(|(k, v)| k == "8" && v == layer);
            if on {
                match kind {
                    "LINE" => {
                        let (a, b) = ([get("10"), get("20")], [get("11"), get("21")]);
                        add(a);
                        add(b);
                        pieces.push(Piece {
                            from: a,
                            to: b,
                            green: 0.5 * (a[0] * b[1] - b[0] * a[1]),
                            closed: false,
                        });
                    }
                    "ARC" => {
                        let (c, r) = ([get("10"), get("20")], get("40"));
                        let a0 = get("50").to_radians();
                        let mut a1 = get("51").to_radians();
                        while a1 <= a0 {
                            a1 += TAU;
                        }
                        let at = |a: f64| [c[0] + r * a.pcos(), c[1] + r * a.psin()];
                        add(at(a0));
                        add(at(a1));
                        for q in 0..8 {
                            let e = f64::from(q) * 0.5 * PI;
                            let e = e + ((a0 - e) / TAU).ceil() * TAU;
                            if e <= a1 {
                                add(at(e));
                            }
                        }
                        let green = 0.5
                            * (r * c[0] * (a1.psin() - a0.psin())
                                - r * c[1] * (a1.pcos() - a0.pcos())
                                + r * r * (a1 - a0));
                        pieces.push(Piece {
                            from: at(a0),
                            to: at(a1),
                            green,
                            closed: false,
                        });
                    }
                    _ => {
                        let (c, r) = ([get("10"), get("20")], get("40"));
                        add([c[0] - r, c[1] - r]);
                        add([c[0] + r, c[1] + r]);
                        pieces.push(Piece {
                            from: c,
                            to: c,
                            green: PI * r * r,
                            closed: true,
                        });
                    }
                }
            }
            i = j;
            continue;
        }
        i += 2;
    }
    (pieces, bbox)
}

/// The loops the pieces chain into (ends within `gap`), each with its
/// signed area run along the chain; `None` when they do not chain.
fn loops(pieces: &[Piece], gap: f64) -> Option<Vec<f64>> {
    let mut used = vec![false; pieces.len()];
    let mut out = Vec::new();
    let near = |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).phypot(a[1] - b[1]) <= gap;
    for start in 0..pieces.len() {
        if used[start] {
            continue;
        }
        used[start] = true;
        if pieces[start].closed {
            out.push(pieces[start].green);
            continue;
        }
        let first = pieces[start].from;
        let mut end = pieces[start].to;
        let mut area = pieces[start].green;
        let mut steps = 0;
        while !near(end, first) {
            steps += 1;
            if steps > pieces.len() {
                return None;
            }
            let next = (0..pieces.len()).find(|&j| {
                !used[j]
                    && !pieces[j].closed
                    && (near(pieces[j].from, end) || near(pieces[j].to, end))
            })?;
            used[next] = true;
            if near(pieces[next].from, end) {
                area += pieces[next].green;
                end = pieces[next].to;
            } else {
                area -= pieces[next].green;
                end = pieces[next].from;
            }
        }
        out.push(area);
    }
    Some(out)
}

/// The SVG's viewBox.
fn view_box(svg: &str) -> [f64; 4] {
    let at = svg.find("viewBox=\"").unwrap() + 9;
    let rest = &svg[at..];
    let end = rest.find('"').unwrap();
    let v: Vec<f64> = rest[..end]
        .split_whitespace()
        .map(|s| s.parse().unwrap())
        .collect();
    [v[0], v[1], v[2], v[3]]
}

/// One flange as drawn.
struct Fl {
    edge: usize,
    angle: f64,
    up: bool,
    r: f64,
    k: f64,
    l: f64,
}

#[test]
fn seeded_sheet_metal_parts_by_command_and_their_exported_patterns() {
    let mut failures = Vec::new();
    let mut tally = (0, 0);
    for (n, k) in SCALES.iter().flat_map(|k| [*k; 4]).enumerate() {
        let mut rng = Rng(0x5EE7_E461 ^ (n as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let dir = workdir("hunt-sheet", &format!("part-{n}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        let (w, d) = (rng.between(20.0, 80.0) * k, rng.between(20.0, 80.0) * k);
        let t = rng.between(0.3, 3.0) * k;
        run(
            &mut s,
            "sketch.create",
            json!({"plane": "xy", "name": "Base"}),
        );
        let made = run(
            &mut s,
            "sketch.rect",
            json!({"sketch": "Base", "from": [0.0, 0.0], "to": [w, d]}),
        );
        let lines: Vec<String> = (0..4)
            .map(|i| made["lines"][i].as_str().unwrap().to_string())
            .collect();
        // A hole: a circle, or three arcs of one circle.
        let mut hole = 0.0;
        if rng.below(2) == 0 {
            let r = rng.between(0.05, 0.15) * w.min(d);
            let c = [0.5 * w, 0.5 * d];
            if rng.below(2) == 0 {
                run(
                    &mut s,
                    "sketch.circle",
                    json!({"sketch": "Base", "center": c, "radius": r}),
                );
            } else {
                let a0 = rng.between(0.0, TAU);
                let cuts = [a0, a0 + rng.between(1.0, 2.5), a0 + rng.between(3.5, 5.0)];
                let at = |a: f64| [c[0] + r * a.pcos(), c[1] + r * a.psin()];
                for i in 0..3 {
                    let next = if i == 2 { cuts[0] + TAU } else { cuts[i + 1] };
                    run(
                        &mut s,
                        "sketch.arc",
                        json!({"sketch": "Base", "center": c, "start": at(cuts[i]), "end": at(next)}),
                    );
                }
            }
            hole = PI * r * r;
        }
        // Flanges on one to four edges, at moderate sizes.
        let mut fl = Vec::new();
        for e in 0..4 {
            if rng.below(2) == 0 && !(e == 3 && fl.is_empty()) {
                continue;
            }
            fl.push(Fl {
                edge: e,
                angle: rng.between(10.0, 170.0).to_radians(),
                up: rng.below(2) == 0,
                r: t * rng.between(0.3, 3.0),
                k: rng.between(0.0, 1.0),
                l: rng.between(3.0, 25.0) * k,
            });
        }
        let flanges: Vec<Value> = fl
            .iter()
            .map(|f| {
                json!({"edge": lines[f.edge], "length": format!("{} mm", f.l),
                       "angle": format!("{} deg", f.angle.to_degrees()),
                       "direction": if f.up { "up" } else { "down" },
                       "radius": format!("{} mm", f.r), "kFactor": f.k})
            })
            .collect();
        let result = run(
            &mut s,
            "feature.sheetMetal",
            json!({"sketch": "Base", "name": "Part", "thickness": format!("{t} mm"),
                   "radius": format!("{} mm", t), "kFactor": 0.5, "flanges": flanges}),
        );
        let what = format!(
            "part {n} at {k}: {w:e} x {d:e}, T {t:e}, hole {hole:e}, flanges {:?}",
            fl.iter()
                .map(|f| (f.edge, f.angle.to_degrees(), f.up, f.r, f.k, f.l))
                .collect::<Vec<_>>()
        );
        if result["status"] != "ok" {
            // Two flanges leaning in over one corner run into each other:
            // a true refusal. Anything else is noted.
            println!("refused: {what}: {}", result["error"]);
            tally.1 += 1;
            continue;
        }
        tally.0 += 1;
        let edge_len = |e: usize| if e.is_multiple_of(2) { w } else { d };
        let base = w * d - hole;
        let mut volume = base * t;
        let mut flat = base;
        // The pattern's extent: the base grown by `BA + L` across each
        // flanged edge (bottom −y, right +x, top +y, left −x).
        let mut ext = [0.0, 0.0, w, d];
        for f in &fl {
            let wl = edge_len(f.edge);
            let ba = f.angle * (f.r + f.k * t);
            volume += f.angle * wl * t * (f.r + 0.5 * t) + wl * f.l * t;
            flat += wl * (ba + f.l);
            match f.edge {
                0 => ext[1] -= ba + f.l,
                1 => ext[2] += ba + f.l,
                2 => ext[3] += ba + f.l,
                _ => ext[0] -= ba + f.l,
            }
        }
        let b = body(&mut s, "Part");
        let v = b["volume"].as_f64().unwrap();
        if b["valid"] != true || (v - volume).abs() > 1e-8 * volume {
            failures.push(format!(
                "{what}: valid {}, volume {v:e} against {volume:e}",
                b["valid"]
            ));
        }
        let out = run(
            &mut s,
            "file.export",
            json!({"path": "p.dxf", "sheetMetal": "Part"}),
        );
        let fa = out["flatArea"].as_f64().unwrap();
        if (fa - flat).abs() > 1e-9 * flat {
            failures.push(format!("{what}: flatArea {fa:e} against {flat:e}"));
        }
        let text = std::fs::read_to_string(dir.join("p.dxf")).unwrap();
        let (pieces, bb) = dxf_pieces(&text, "outline");
        let size = (ext[2] - ext[0]).max(ext[3] - ext[1]);
        match loops(&pieces, 1e-9 * size) {
            Some(areas) => {
                let outer = areas.iter().fold(0.0f64, |m, a| m.max(a.abs()));
                let holes: f64 = areas.iter().map(|a| a.abs()).sum::<f64>() - outer;
                let area = outer - holes;
                if (area - flat).abs() > 1e-9 * flat {
                    failures.push(format!(
                        "{what}: the DXF's outline holds {area:e} against {flat:e} ({} loops)",
                        areas.len()
                    ));
                }
            }
            None => failures.push(format!("{what}: the DXF's outline does not chain")),
        }
        let off = (0..4).map(|i| (bb[i] - ext[i]).abs()).fold(0.0, f64::max);
        if off > 1e-9 * size {
            failures.push(format!("{what}: the DXF's box {bb:?} against {ext:?}"));
        }
        run(
            &mut s,
            "file.export",
            json!({"path": "p.svg", "sheetMetal": "Part"}),
        );
        let svg = std::fs::read_to_string(dir.join("p.svg")).unwrap();
        let vb = view_box(&svg);
        let want = [ext[0], -ext[3], ext[2] - ext[0], ext[3] - ext[1]];
        let off = (0..4).map(|i| (vb[i] - want[i]).abs()).fold(0.0, f64::max);
        if off > 1e-9 * size {
            failures.push(format!("{what}: the SVG's viewBox {vb:?} against {want:?}"));
        }
    }
    println!("built {}, refused {}", tally.0, tally.1);
    assert!(failures.is_empty(), "{failures:#?}");
    assert!(tally.0 >= 6, "too few built: {tally:?}");
}

/// A pan: a disc of radius `a` with a curved flange round its rim, by
/// command: the volume by Pappus (the section's `∫ ρ dA` about the axis
/// over a turn), the flat area and the DXF's outline (the tip circle)
/// `π (a + BA + L)²`.
#[test]
fn pans_by_command_and_their_exported_patterns() {
    let mut failures = Vec::new();
    for (n, k) in SCALES.iter().flat_map(|k| [*k; 3]).enumerate() {
        let mut rng = Rng(0x0BA4_E461 ^ (n as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let dir = workdir("hunt-sheet", &format!("pan-{n}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        run(
            &mut s,
            "sketch.create",
            json!({"plane": "xy", "name": "Base"}),
        );
        let a = rng.between(20.0, 60.0) * k;
        let made = run(
            &mut s,
            "sketch.circle",
            json!({"sketch": "Base", "center": [0.0, 0.0], "radius": a}),
        );
        let circle = made["entity"].as_str().unwrap().to_string();
        let t = rng.between(0.3, 3.0) * k;
        let th = rng.between(20.0, 170.0).to_radians();
        let r = t * rng.between(0.5, 3.0);
        let kk = rng.between(0.0, 1.0);
        let l = rng.between(3.0, 15.0) * k;
        let up = rng.below(2) == 0;
        let result = run(
            &mut s,
            "feature.sheetMetal",
            json!({"sketch": "Base", "name": "Pan", "thickness": format!("{t} mm"),
                   "radius": format!("{r} mm"), "kFactor": kk,
                   "curvedFlanges": [{"edge": circle, "length": format!("{l} mm"),
                                      "angle": format!("{} deg", th.to_degrees()),
                                      "direction": if up { "up" } else { "down" }}]}),
        );
        let what = format!(
            "pan {n} at {k}: a {a:e}, T {t:e}, {} deg R {r:e} K {kk} L {l:e} {}",
            th.to_degrees(),
            if up { "up" } else { "down" }
        );
        if result["status"] != "ok" {
            failures.push(format!("{what}: refused: {}", result["error"]));
            continue;
        }
        let ro = r + t;
        let section = a * (th * (ro * ro - r * r) / 2.0 + l * t)
            + (1.0 - th.pcos()) * (ro.ppowi(3) - r.ppowi(3)) / 3.0
            + th.psin() * l * (ro * ro - r * r) / 2.0
            + th.pcos() * t * l * l / 2.0;
        let volume = PI * a * a * t + TAU * section;
        let tip = a + th * (r + kk * t) + l;
        let flat = PI * tip * tip;
        let b = body(&mut s, "Pan");
        let v = b["volume"].as_f64().unwrap();
        if b["valid"] != true || (v - volume).abs() > 1e-7 * volume {
            failures.push(format!(
                "{what}: valid {}, volume {v:e} against {volume:e}",
                b["valid"]
            ));
        }
        let out = run(
            &mut s,
            "file.export",
            json!({"path": "p.dxf", "sheetMetal": "Pan"}),
        );
        let fa = out["flatArea"].as_f64().unwrap();
        if (fa - flat).abs() > 1e-9 * flat {
            failures.push(format!("{what}: flatArea {fa:e} against {flat:e}"));
        }
        let text = std::fs::read_to_string(dir.join("p.dxf")).unwrap();
        let (pieces, _) = dxf_pieces(&text, "outline");
        match loops(&pieces, 1e-9 * tip) {
            Some(areas) => {
                let outer = areas.iter().fold(0.0f64, |m, a| m.max(a.abs()));
                if (outer - flat).abs() > 1e-9 * flat || areas.len() != 1 {
                    failures.push(format!(
                        "{what}: the DXF's outline holds {areas:?} against {flat:e}"
                    ));
                }
            }
            None => failures.push(format!("{what}: the DXF's outline does not chain")),
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn discs_of_arcs_plated_by_command_are_their_closed_form() {
    let mut failures = Vec::new();
    for (n, k) in SCALES.iter().flat_map(|k| [*k; 4]).enumerate() {
        let mut rng = Rng(0xD15C_E461 ^ (n as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let dir = workdir("hunt-sheet", &format!("disc-{n}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        run(
            &mut s,
            "sketch.create",
            json!({"plane": "xy", "name": "Disc"}),
        );
        let r = rng.between(5.0, 40.0) * k;
        let c = [rng.between(-50.0, 50.0) * k, rng.between(-50.0, 50.0) * k];
        let arcs = 2 + rng.below(6) as usize;
        let mut cuts: Vec<f64> = (0..arcs)
            .map(|i| (i as f64 + rng.between(0.1, 0.9)) * TAU / arcs as f64)
            .collect();
        let turn = rng.between(0.0, TAU);
        for a in &mut cuts {
            *a += turn;
        }
        let at = |a: f64| [c[0] + r * a.pcos(), c[1] + r * a.psin()];
        for i in 0..arcs {
            let next = if i + 1 == arcs {
                cuts[0] + TAU
            } else {
                cuts[i + 1]
            };
            run(
                &mut s,
                "sketch.arc",
                json!({"sketch": "Disc", "center": c, "start": at(cuts[i]), "end": at(next)}),
            );
        }
        let filled = run(
            &mut s,
            "feature.fill",
            json!({"sketch": "Disc", "name": "Sheet"}),
        );
        if filled["status"] != "ok" {
            failures.push(format!("disc {n}: not filled: {filled}"));
            continue;
        }
        let face = body(&mut s, "Sheet")["faceNames"][0]
            .as_str()
            .unwrap()
            .to_string();
        let t = rng.between(0.3, 3.0) * k;
        let side = ["in", "out", "mid"][rng.below(3) as usize];
        let plated = run(
            &mut s,
            "feature.plate",
            json!({"faces": [face], "thickness": format!("{t} mm"), "side": side, "name": "Plate"}),
        );
        let what = format!("disc {n} at {k}: r {r:e} of {arcs} arcs at {cuts:?}, t {t:e} {side}");
        if plated["status"] != "ok" {
            failures.push(format!("{what}: refused: {plated}"));
            continue;
        }
        let b = body(&mut s, "Plate");
        let v = b["volume"].as_f64().unwrap();
        let a = b["area"].as_f64().unwrap();
        let (vv, aa) = (PI * r * r * t, 2.0 * PI * r * r + TAU * r * t);
        if b["valid"] != true || (v - vv).abs() > 1e-9 * vv || (a - aa).abs() > 1e-9 * aa {
            failures.push(format!(
                "{what}: valid {}, volume {v:e} against {vv:e}, area {a:e} against {aa:e}",
                b["valid"]
            ));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

/// The kernel's finding by command (crates/kernel/tests/hunt_sheet_repro.rs
/// › `a_bend_down_at_one_radius_is_its_closed_form`): a base 50 × 40, 1
/// thick, with a flange 20 long bent down 140 degrees about the inner
/// radius 0.1762914118095948 (K 0.4). What the user gets there, and a
/// thousandth either side: each built part held to the
/// closed form `2000 + 40 (θ (R + ½) + 20)`, each failure printed. Until
/// 2026-11-15 the value itself failed the feature (the regeneration's own
/// check meshed the part and found the face it could not, an edge round
/// its circle 2.4 times), never a wrong body.
#[test]
fn a_bend_down_at_one_radius_by_command() {
    let mut seen = Vec::new();
    for r in [
        0.1762914118095948 * (1.0 - 1e-3),
        0.1762914118095948,
        0.1762914118095948 * (1.0 + 1e-3),
    ] {
        let dir = workdir("hunt-sheet", &format!("down-{r}"));
        let mut s = session_in(&dir);
        new_document(&mut s, 1.0);
        run(
            &mut s,
            "sketch.create",
            json!({"plane": "xy", "name": "Base"}),
        );
        let made = run(
            &mut s,
            "sketch.rect",
            json!({"sketch": "Base", "from": [0.0, 0.0], "to": [50.0, 40.0]}),
        );
        let right = made["lines"][1].as_str().unwrap().to_string();
        let result = run(
            &mut s,
            "feature.sheetMetal",
            json!({"sketch": "Base", "name": "Part", "thickness": "1 mm",
                   "radius": format!("{r:?} mm"), "kFactor": 0.4,
                   "flanges": [{"edge": right, "length": "20 mm", "angle": "140 deg",
                                "direction": "down"}]}),
        );
        let th = 140f64.to_radians();
        let want = 2000.0 + 40.0 * (th * (r + 0.5) + 20.0);
        if result["status"] == "ok" {
            let b = body(&mut s, "Part");
            let v = b["volume"].as_f64().unwrap();
            println!(
                "R {r:?}: built, valid {}, volume {v} for {want}",
                b["valid"]
            );
            seen.push(((v - want).abs() <= 1e-9 * want, b["valid"] == true));
        } else {
            println!("R {r:?}: {}: {}", result["status"], result["error"]);
            seen.push((false, false));
        }
    }
    assert_eq!(seen, vec![(true, true); 3], "{seen:?}");
}
