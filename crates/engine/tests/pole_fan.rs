//! Domes closed across a seam in a tip, as a STEP file holds them (no
//! command builds one), through the commands: read by `file.import`,
//! measured, and written by `file.export` to STL and OBJ, the files read
//! back here by their coordinates as another program reads them. Until
//! 2026-11-15 such a dome's STL could be a mesh that is no manifold,
//! written without a word (docs/design/tessellator.md › *Sideways at a
//! seam*); every file is now the closed surface of a solid.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{session_in, workdir};
use craft3d_geom::{Curve3, NurbsCurve, NurbsSurface, Plane, Surface};
use craft3d_io::{StepBody, StepOptions, export_step};
use craft3d_math::{Frame, Vec3};
use craft3d_topo::{Body, BodyKind, Coedge};
use serde_json::json;
use std::collections::BTreeMap;
use std::f64::consts::{FRAC_1_SQRT_2, PI};

/// A dome over the ellipse with the half axes `a` and `b`, closed across
/// a seam (on `+x`, or `−x` with `flip`) in a tip `length` over `foot`,
/// on a flat base (as `crates/kernel/tests/pole_fan.rs` builds it).
fn seamed_dome(frame: &Frame, (a, b): (f64, f64), flip: bool, foot: [f64; 2], length: f64) -> Body {
    let s = if flip { -1.0 } else { 1.0 };
    let ring = [
        (1.0, 0.0),
        (1.0, 1.0),
        (0.0, 1.0),
        (-1.0, 1.0),
        (-1.0, 0.0),
        (-1.0, -1.0),
        (0.0, -1.0),
        (1.0, -1.0),
        (1.0, 0.0),
    ];
    let top = frame.point(foot[0], foot[1], length);
    let (mut net, mut weights, mut rim, mut rim_weights) = (vec![], vec![], vec![], vec![]);
    for (i, &(x, y)) in ring.iter().enumerate() {
        let w = if i % 2 == 1 { FRAC_1_SQRT_2 } else { 1.0 };
        let low = frame.point(s * a * x, s * b * y, 0.0);
        net.extend([low, frame.point(s * a * x, s * b * y, length), top]);
        weights.extend([w; 3]);
        rim.push(low);
        rim_weights.push(w);
    }
    let knots = vec![
        0.0, 0.0, 0.0, 0.25, 0.25, 0.5, 0.5, 0.75, 0.75, 1.0, 1.0, 1.0,
    ];
    let ends = vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0];
    let surface = NurbsSurface::new(
        2,
        2,
        knots.clone(),
        ends.clone(),
        9,
        3,
        net.clone(),
        weights,
    )
    .unwrap();
    let rim = NurbsCurve::new(2, knots, rim, rim_weights).unwrap();
    let seam = NurbsCurve::new(2, ends, net[..3].to_vec(), vec![1.0; 3]).unwrap();
    let (forward, backward) = (
        |edge| Coedge {
            edge,
            reversed: false,
        },
        |edge| Coedge {
            edge,
            reversed: true,
        },
    );
    let mut body = Body::new(BodyKind::Solid);
    let (start, apex) = (body.add_vertex(net[0]), body.add_vertex(top));
    let rim = body.add_edge(Curve3::Nurbs(rim), (0.0, 1.0), start, start);
    let seam = body.add_edge(Curve3::Nurbs(seam), (0.0, 1.0), start, apex);
    let outline = body.add_loop(vec![forward(rim), forward(seam), backward(seam)]);
    let dome = body.add_face(Surface::Nurbs(surface), false, vec![outline]);
    let down = frame.reversed_at(frame.origin()).unwrap();
    let outline = body.add_loop(vec![backward(rim)]);
    let base = body.add_face(Surface::Plane(Plane::new(down)), false, vec![outline]);
    body.add_shell(vec![dome, base]);
    body
}

/// What keeps the triangles from being the closed surface of a solid,
/// their corners taken as the same point where they are the same bits:
/// a triangle with two corners at one point, a side not used once each
/// way, a corner whose triangles make more than one fan.
fn not_closed(triangles: &[[[u32; 3]; 3]]) -> Option<String> {
    let mut index: BTreeMap<[u32; 3], u32> = BTreeMap::new();
    let mut corners = Vec::with_capacity(triangles.len());
    for t in triangles {
        let mut c = [0_u32; 3];
        for (slot, p) in c.iter_mut().zip(t) {
            let next = index.len() as u32;
            *slot = *index.entry(*p).or_insert(next);
        }
        if c[0] == c[1] || c[1] == c[2] || c[2] == c[0] {
            return Some(format!("a triangle with two corners at one point: {t:?}"));
        }
        corners.push(c);
    }
    let mut sides: BTreeMap<(u32, u32), u32> = BTreeMap::new();
    let mut round: BTreeMap<u32, BTreeMap<u32, u32>> = BTreeMap::new();
    for &[a, b, c] in &corners {
        for (from, to, other) in [(a, b, c), (b, c, a), (c, a, b)] {
            *sides.entry((from, to)).or_insert(0) += 1;
            round.entry(from).or_default().insert(to, other);
        }
    }
    for (&(a, b), &n) in &sides {
        let back = sides.get(&(b, a)).copied().unwrap_or(0);
        if n != 1 || back != 1 {
            return Some(format!(
                "a side used {n} times one way and {back} the other"
            ));
        }
    }
    for next in round.values() {
        let Some((&start, _)) = next.iter().next() else {
            continue;
        };
        let (mut here, mut count) = (start, 0);
        loop {
            here = next[&here];
            count += 1;
            if here == start || count > next.len() {
                break;
            }
        }
        if count != next.len() {
            return Some("a corner whose triangles make more than one fan".to_string());
        }
    }
    None
}

/// The triangles of a binary STL, by the bits of their corners.
fn stl_triangles(bytes: &[u8]) -> Vec<[[u32; 3]; 3]> {
    let count = u32::from_le_bytes(bytes[80..84].try_into().unwrap()) as usize;
    assert_eq!(bytes.len(), 84 + 50 * count, "the STL's length");
    (0..count)
        .map(|t| {
            let at = 84 + 50 * t + 12;
            let corner = |i: usize| {
                let o = at + 12 * i;
                [0, 1, 2].map(|j| {
                    u32::from_le_bytes(bytes[o + 4 * j..o + 4 * j + 4].try_into().unwrap())
                })
            };
            [corner(0), corner(1), corner(2)]
        })
        .collect()
}

/// The triangles of an OBJ file, by the bits of their corners as written
/// (as `f32`, the precision the STL has, so both are read alike).
fn obj_triangles(text: &str) -> Vec<[[u32; 3]; 3]> {
    let mut points: Vec<[u32; 3]> = Vec::new();
    let mut triangles = Vec::new();
    for line in text.lines() {
        let mut words = line.split_whitespace();
        match words.next() {
            Some("v") => {
                let p: Vec<f64> = words.take(3).map(|w| w.parse().unwrap()).collect();
                points.push([p[0], p[1], p[2]].map(|x| (x as f32).to_bits()));
            }
            Some("f") => {
                let c: Vec<usize> = words
                    .map(|w| w.split('/').next().unwrap().parse::<usize>().unwrap() - 1)
                    .collect();
                assert_eq!(c.len(), 3, "a face of {} corners", c.len());
                triangles.push([points[c[0]], points[c[1]], points[c[2]]]);
            }
            _ => {}
        }
    }
    triangles
}

#[test]
fn an_imported_dome_closed_across_a_seam_is_written_as_a_closed_surface() {
    let dir = workdir("pole-fan", "dome");
    let turned = Frame::new(
        Vec3::new(3.0, -2.0, 5.0),
        Vec3::new(2.0, -3.0, 6.0),
        Vec3::new(1.0, 0.3, 0.0),
    )
    .unwrap();
    let mut cases = Vec::new();
    for (frame_name, frame) in [("turned", turned), ("world", Frame::WORLD)] {
        for (flip, x) in [
            (false, 0.9),
            (true, 0.9),
            (false, 0.5),
            (true, 0.99),
            (false, 0.0),
        ] {
            cases.push((frame_name, frame, (1.0, 0.01), flip, x));
        }
        cases.push((frame_name, frame, (0.3, 0.003), false, 0.0));
    }
    let mut faults = Vec::new();
    for (index, (frame_name, frame, (a, b), flip, x)) in cases.into_iter().enumerate() {
        let what = format!(
            "the dome {a} x {b} with its tip over {x} of its half axis, its seam on {}x, in {frame_name}",
            if flip { "-" } else { "+" }
        );
        let body = seamed_dome(&frame, (a, b), flip, [x * a, 0.0], 0.5);
        let (bytes, _) =
            export_step(&[StepBody::new("Dome", &body)], &StepOptions::default()).unwrap();
        let file = format!("dome-{index}.step");
        std::fs::write(dir.join(&file), bytes).unwrap();
        let mut s = session_in(&dir);
        s.execute("file.new", &json!({"units": "mm"})).unwrap();
        let imported = s.execute("file.import", &json!({"path": file})).unwrap();
        assert_eq!(imported["bodies"][0]["status"], "ok", "{what}: {imported}");
        let measured = s.execute("document.measure", &json!({})).unwrap();
        let volume = 11.0 / 15.0 * PI * a * b * 0.5;
        let got = measured["volume"].as_f64().unwrap();
        assert!(
            (got - volume).abs() <= 1e-12 * volume,
            "{what}: {got} against {volume}"
        );
        let mut exports = Vec::new();
        for (key, value) in [
            ("tolerance", json!(0.002)),
            ("tolerance", json!(0.02)),
            ("quality", json!("coarse")),
            ("quality", json!("normal")),
            ("quality", json!("fine")),
        ] {
            for format in ["stl", "obj"] {
                let out = format!("dome-{index}-{value}.{format}").replace('"', "");
                let result = s.execute("file.export", &json!({"path": out, key: value}));
                let result = match result {
                    Ok(result) => result,
                    Err(error) => {
                        faults.push(format!(
                            "{what}, {format} at {key} {value}: refused: {}",
                            error.message()
                        ));
                        continue;
                    }
                };
                let bytes = std::fs::read(dir.join(&out)).unwrap();
                let triangles = match format {
                    "stl" => stl_triangles(&bytes),
                    _ => obj_triangles(std::str::from_utf8(&bytes).unwrap()),
                };
                if let Some(fault) = not_closed(&triangles) {
                    faults.push(format!("{what}, {format} at {key} {value}: {fault}"));
                }
                if result["closed"] != 1 {
                    faults.push(format!(
                        "{what}, {format} at {key} {value}: closed {}",
                        result["closed"]
                    ));
                }
                exports.push(format!("{format} {key} {value}: {}", triangles.len()));
            }
        }
        println!("{what}: {}", exports.join(", "));
    }
    assert!(faults.is_empty(), "{faults:#?}");
}
