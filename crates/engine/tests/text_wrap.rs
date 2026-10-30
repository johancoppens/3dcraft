//! Text wrapped onto a cylinder and a cone (`feature.extrude {wrap}`),
//! embossed and engraved, against closed forms: the development is an
//! isometry, so the wrapped region has the glyphs' own area: the wrapped
//! body (reaching `d` to both sides of the surface) has the volume `2 A d`,
//! and the part of it outside a cylinder of radius `r`, `A (d + d² / 2r)`
//! (inside: `A (d − d² / 2r)`).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::{built, failed, inspect, run, session};
use craft3d_engine::Session;
use craft3d_font::{FontFile, Metrics, filled_outline};
use serde_json::json;
use std::path::PathBuf;

fn text_area(text: &str, size: f64) -> f64 {
    let dir = match std::env::var_os("CRAFT3D_FONTS") {
        Some(d) if !d.is_empty() => PathBuf::from(d),
        _ => PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts"),
    };
    let bytes = std::fs::read(dir.join("NotoSans-Regular.otf")).unwrap();
    let f = FontFile::parse(&bytes).unwrap();
    let s = size / f.cap_height();
    text.chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| filled_outline(&f, f.glyph(c).unwrap()).unwrap().area() * s * s)
        .sum()
}

fn volume_of(s: &mut Session, name: &str) -> f64 {
    let doc = inspect(s);
    let id = doc["features"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == name)
        .unwrap()["id"]
        .clone();
    doc["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|b| b["feature"] == id)
        .map(|b| {
            assert_eq!(b["valid"], true, "{b}");
            b["volume"].as_f64().unwrap()
        })
        .sum()
}

fn deep(s: &mut Session) {
    let check = run(s, "document.check", json!({"deep": true}));
    assert_eq!(check["ok"], true, "{check}");
}

fn can(s: &mut Session, k: f64) {
    run(s, "file.new", json!({"units": "mm", "tolerance": 1e-6 * k}));
    built(
        s,
        "feature.cylinder",
        json!({"radius": 20.0 * k, "height": 40.0 * k, "name": "Can"}),
    );
    run(s, "sketch.create", json!({"name": "Label"}));
    run(
        s,
        "sketch.text",
        json!({"sketch": "Label", "text": "HOB", "size": 8.0 * k, "at": [-9.0 * k, 16.0 * k]}),
    );
}

#[test]
fn text_wrapped_onto_a_cylinder_has_the_closed_form_volume() {
    for k in [1e-2, 1.0, 1e2] {
        let (r, a) = (20.0 * k, text_area("HOB", 8.0 * k));
        for d in [1.0 * k, -0.8 * k] {
            let mut s = session();
            can(&mut s, k);
            built(
                &mut s,
                "feature.extrude",
                json!({"sketch": "Label", "wrap": "F1:face(0)", "distance": d, "name": "Wrapped"}),
            );
            // The wrapped body reaches |d| to both sides of the surface
            // (the tool a join or a cut takes): A · 2|d| exactly, the
            // curvature's terms cancelling.
            let _ = r;
            let expected = 2.0 * a * d.abs();
            let v = volume_of(&mut s, "Wrapped");
            assert!(
                (v - expected).abs() <= 1e-5 * expected,
                "k {k}, d {d}: {v} vs {expected}"
            );
            deep(&mut s);
        }
    }
}

#[test]
fn embossed_and_engraved_on_a_cylinder() {
    let (r, h) = (20.0, 40.0);
    let can_volume = std::f64::consts::PI * r * r * h;
    let a = text_area("HOB", 8.0);
    // Raised.
    let mut s = session();
    can(&mut s, 1.0);
    built(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Label", "wrap": "F1:face(0)", "distance": 1, "join": "Can", "name": "Raised"}),
    );
    let v = volume_of(&mut s, "Raised");
    let expected = can_volume + a * (1.0 + 1.0 / (2.0 * r));
    assert!((v - expected).abs() <= 1e-5 * expected, "{v} vs {expected}");
    deep(&mut s);
    // Engraved.
    let mut s = session();
    can(&mut s, 1.0);
    built(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Label", "wrap": "F1:face(0)", "distance": -0.5, "cut": "Can", "name": "Cut"}),
    );
    let v = volume_of(&mut s, "Cut");
    let expected = can_volume - a * (0.5 - 0.25 / (2.0 * r));
    assert!((v - expected).abs() <= 1e-5 * expected, "{v} vs {expected}");
    deep(&mut s);
}

#[test]
fn text_wrapped_onto_a_cone_keeps_its_area() {
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    built(
        &mut s,
        "feature.cone",
        json!({"radius": 30, "height": 60, "name": "Cone"}),
    );
    run(&mut s, "sketch.create", json!({"name": "Label"}));
    run(
        &mut s,
        "sketch.text",
        json!({"sketch": "Label", "text": "CONE", "size": 6, "at": [-10, 8]}),
    );
    built(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Label", "wrap": "F1:face(0)", "distance": 0.5, "name": "OnCone"}),
    );
    deep(&mut s);
    // The body reaches the depth to both sides of the cone: A · 2d
    // exactly (one principal curvature is zero, the other's term cancels).
    let a = text_area("CONE", 6.0);
    let v = volume_of(&mut s, "OnCone");
    assert!((v - 2.0 * a * 0.5).abs() <= 1e-5 * a, "{v} vs {}", a);
}

#[test]
fn other_surfaces_and_bad_wraps_are_refused_by_name() {
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    built(
        &mut s,
        "feature.sphere",
        json!({"radius": 20, "name": "Ball"}),
    );
    run(&mut s, "sketch.create", json!({"name": "L"}));
    run(
        &mut s,
        "sketch.text",
        json!({"sketch": "L", "text": "X", "size": 5}),
    );
    let why = failed(
        &mut s,
        "feature.extrude",
        json!({"sketch": "L", "wrap": "F1:face(0)", "distance": 1}),
    );
    assert!(why.contains("not built yet"), "{why}");
    // Too long for the cylinder: past half a turn.
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    built(
        &mut s,
        "feature.cylinder",
        json!({"radius": 2, "height": 10}),
    );
    run(&mut s, "sketch.create", json!({"name": "L"}));
    run(
        &mut s,
        "sketch.text",
        json!({"sketch": "L", "text": "far too long", "size": 3, "at": [0, 2]}),
    );
    let why = failed(
        &mut s,
        "feature.extrude",
        json!({"sketch": "L", "wrap": "F1:face(0)", "distance": 0.2}),
    );
    assert!(why.contains("back of the surface"), "{why}");
    // Parameters that do not go together.
    for params in [
        json!({"sketch": "L", "wrap": "F1:face(0)"}),
        json!({"sketch": "L", "wrap": "F1:face(0)", "distance": 1, "throughAll": true}),
        json!({"sketch": "L", "wrap": "F1:face(0)", "distance": 1, "symmetric": true}),
        json!({"sketch": "L", "wrap": "nonsense", "distance": 1}),
    ] {
        common::checked::error(&mut s, "feature.extrude", params);
    }
}

/// "3DCraft" on a cylinder of radius 20 (the evidence case of 2026-10-30):
/// the wrapped body had rims facing inward (their loops bound no region),
/// which passed the checks then; `topo::check` now refuses such a face.
#[test]
fn three_d_craft_on_a_cylinder_is_rendered_and_joined() {
    let dir = common::workdir("wrap", "three-d-craft");
    let mut s = common::session_in(&dir);
    run(&mut s, "file.new", json!({"units": "mm"}));
    built(
        &mut s,
        "feature.cylinder",
        json!({"radius": 20, "height": 40, "name": "Body"}),
    );
    run(&mut s, "sketch.create", json!({"name": "Label"}));
    run(
        &mut s,
        "sketch.text",
        json!({"sketch": "Label", "text": "3DCraft", "size": 6, "at": [0, 12], "align": "center"}),
    );
    built(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Label", "wrap": "F1:face(0)", "distance": 1, "join": "Body", "name": "Text"}),
    );
    deep(&mut s);
    run(&mut s, "file.export", json!({"path": "text.stl"}));
    assert!(dir.join("text.stl").metadata().unwrap().len() > 0);
}

/// The wrapped feature `name`'s body, regenerated from the saved document.
fn body_of(
    s: &mut Session,
    dir: &std::path::Path,
    name: &str,
) -> Option<std::sync::Arc<craft3d_topo::Body>> {
    let path = dir.join("dump.3dcraft");
    common::plain::run(s, "file.save", json!({"path": "dump.3dcraft"}));
    let doc = craft3d_format::load(&path).ok()?;
    let regen = craft3d_regen::regenerate(&doc);
    let f = doc.features.iter().find(|f| f.name == name)?;
    regen.feature(f.id)?.body.clone()
}

/// Every face of `body`: its surface, and each loop's coedges with their
/// curves, lengths and ends in the surface's parameters, and the loop's
/// signed area there (times the face's sense: positive for an outer loop).
fn dump(body: &craft3d_topo::Body, only: Option<usize>) {
    use craft3d_geom::Surface;
    for (i, face) in body.faces.iter().enumerate() {
        if only.is_some_and(|o| o != i) {
            continue;
        }
        let kind = match &face.surface {
            Surface::Cylinder(c) => format!("cylinder r {:.6}", c.radius()),
            Surface::Cone(c) => format!("cone r {:.6} a {:.6}", c.radius(), c.half_angle()),
            other => other.kind().to_string(),
        };
        let sense = if face.reversed { -1.0 } else { 1.0 };
        println!("FACE {i}: {kind}, reversed {}", face.reversed);
        for l in &face.loops {
            let lp = body.loop_(*l).unwrap();
            let mut poly: Vec<(f64, f64)> = Vec::new();
            let pu = face.surface.period_u();
            for c in &lp.coedges {
                let e = body.edge(c.edge).unwrap();
                let (t0, t1) = e.range;
                let mut len = 0.0;
                let mut prev = e.curve.eval(t0);
                let n = 64;
                let mut uvs = Vec::new();
                for k in 0..=n {
                    let a = f64::from(k) / f64::from(n);
                    let t = if c.reversed {
                        t1 + (t0 - t1) * a
                    } else {
                        t0 + (t1 - t0) * a
                    };
                    let p = e.curve.eval(t);
                    len += p.distance(prev);
                    prev = p;
                    let (mut u, v) = face.surface.project(p).unwrap_or((f64::NAN, f64::NAN));
                    if let (Some(per), Some(&(lu, _))) = (pu, poly.last()) {
                        u += ((lu - u) / per).round() * per;
                    }
                    uvs.push((u, v));
                    if k < n {
                        poly.push((u, v));
                    }
                }
                let (a, b) = (uvs[0], uvs[n as usize]);
                println!(
                    "  edge {} {} rev {} len {:.6e} uv ({:.6},{:.6}) -> ({:.6},{:.6})",
                    c.edge.0,
                    e.curve.kind(),
                    c.reversed,
                    len,
                    a.0,
                    a.1,
                    b.0,
                    b.1
                );
            }
            let mut area = 0.0;
            for k in 0..poly.len() {
                let (p, q) = (poly[k], poly[(k + 1) % poly.len()]);
                area += p.0 * q.1 - q.0 * p.1;
            }
            println!(
                "  LOOP {}: {} coedges, signed area x sense {:.6e}",
                l.0,
                lp.coedges.len(),
                0.5 * area * sense
            );
        }
    }
}

/// Diagnosis: each glyph of "3DCraft" wrapped alone, where the text puts
/// it, on a cylinder and on a cone; prints which ones fail to mesh (a
/// malformed face) or to join, and dumps the face that fails.
#[test]
#[ignore = "diagnosis of the open bug: run with --ignored --nocapture"]
fn diagnose_three_d_craft() {
    for surface in ["cylinder", "cone"] {
        for (i, c) in "3DCraft".chars().enumerate() {
            let dir = common::workdir("wrap", &format!("diagnose-{surface}-{i}"));
            let mut s = common::session_in(&dir);
            run(&mut s, "file.new", json!({"units": "mm"}));
            if surface == "cylinder" {
                built(
                    &mut s,
                    "feature.cylinder",
                    json!({"radius": 20, "height": 40, "name": "Body"}),
                );
            } else {
                built(
                    &mut s,
                    "feature.cone",
                    json!({"radius": 30, "height": 60, "name": "Body"}),
                );
            }
            run(&mut s, "sketch.create", json!({"name": "Label"}));
            // The whole text, every other glyph a space: the glyph at its own place.
            let text: String = "3DCraft"
                .chars()
                .enumerate()
                .map(|(k, d)| if k == i { d } else { ' ' })
                .collect();
            run(
                &mut s,
                "sketch.text",
                json!({"sketch": "Label", "text": text, "size": 6, "at": [0, 12], "align": "center"}),
            );
            let alone = common::plain::run(
                &mut s,
                "feature.extrude",
                json!({"sketch": "Label", "wrap": "F1:face(0)", "distance": 1, "name": "Alone"}),
            );
            let meshed = body_of(&mut s, &dir, "Alone").map(|b| {
                let r = craft3d_tess::tessellate_body(&b, &craft3d_tess::TessParams::default());
                if let Err(craft3d_tess::TessError::Face { face, .. }) = &r {
                    dump(&b, Some(*face));
                }
                r.err().map_or("ok".to_string(), |e| e.to_string())
            });
            let joined = common::plain::run(
                &mut s,
                "feature.boolean",
                json!({"target": "Body", "tool": "Alone", "op": "union"}),
            );
            println!(
                "DIAG {surface} {c}: alone {} | mesh {:?} | join {}",
                alone.get("error").unwrap_or(&alone["status"]),
                meshed,
                joined
                    .get("error")
                    .or(joined.get("status"))
                    .map_or(String::new(), |e| e.to_string())
            );
        }
    }
}

/// Every printable glyph of the sans and mono fonts, at two sizes, wraps
/// onto a cylinder and a cone into a body that meshes (21 of the sans
/// glyphs at 2 mm had rims facing inward, 2026-10-30); each that fails is
/// printed.
#[test]
fn every_glyph_wraps_and_meshes() {
    let mut failures = 0;
    let mut total = 0;
    for surface in ["cylinder", "cone"] {
        for font in ["sans", "mono"] {
            for size in [2.0, 6.0] {
                let dir = common::workdir("wrap", &format!("every-{surface}-{font}-{size}"));
                let mut s = common::session_in(&dir);
                run(&mut s, "file.new", json!({"units": "mm"}));
                if surface == "cylinder" {
                    built(
                        &mut s,
                        "feature.cylinder",
                        json!({"radius": 20, "height": 40, "name": "Body"}),
                    );
                } else {
                    built(
                        &mut s,
                        "feature.cone",
                        json!({"radius": 30, "height": 60, "name": "Body"}),
                    );
                }
                for c in '!'..='~' {
                    total += 1;
                    let sk = format!("S{}", c as u32);
                    common::plain::run(&mut s, "sketch.create", json!({"name": sk}));
                    common::plain::run(
                        &mut s,
                        "sketch.text",
                        json!({"sketch": sk, "text": match c { '{' => "{{".to_string(), '}' => "}}".to_string(), _ => c.to_string() }, "size": size, "font": font, "at": [-3, 12]}),
                    );
                    let name = format!("W{}", c as u32);
                    let out = s
                        .execute(
                            "feature.extrude",
                            &json!({"sketch": sk, "wrap": "F1:face(0)", "distance": 0.5, "name": name}),
                        )
                        .unwrap_or_else(|e| json!({"error": e.to_string()}));
                    let mesh = body_of(&mut s, &dir, &name).map(|b| {
                        let r =
                            craft3d_tess::tessellate_body(&b, &craft3d_tess::TessParams::default());
                        if let Err(craft3d_tess::TessError::Face { face, .. }) = &r {
                            dump(&b, Some(*face));
                        }
                        r.err().map(|e| e.to_string())
                    });
                    let bad = out.get("error").is_some() || !matches!(mesh, Some(None));
                    if bad {
                        failures += 1;
                        println!(
                            "EVERY {surface} {font} {size} {c:?}: extrude {} | mesh {:?}",
                            out.get("error").unwrap_or(&out["status"]),
                            mesh
                        );
                    }
                    let _ = s.execute("feature.delete", &json!({"feature": name}));
                }
            }
        }
    }
    println!("EVERY: {failures} of {total} failed");
    assert_eq!(failures, 0, "{failures} of {total} glyphs failed");
}
