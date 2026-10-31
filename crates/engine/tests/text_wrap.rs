//! Text wrapped onto a cylinder and a cone (`feature.extrude {wrap}`),
//! embossed and engraved, against closed forms: the development is an
//! isometry, so the wrapped region has the glyphs' own area: the wrapped
//! text a join adds to a cylinder of radius `r` is `A (d + d² / 2r)`, the
//! one a cut takes `A (d − d² / 2r)`; on a cone their sum is `2 A d` and
//! their difference the curvature's integral over the text, `K d²`.

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
use craft3d_math::Portable;
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

/// The lap of the wrapped plate over the surface (`kernel::wrap`): five
/// hundredths of the depth.
const LAP: f64 = 0.05;

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
            // The body alone runs along the outward normal over the levels
            // (lo, hi): out to d with a lap of LAP·d into the can for a
            // raised text, in to d with the lap outside for an engraved
            // one. A shell at radius r + t has the area A (1 + t / r).
            let lap = LAP * d.abs();
            let (lo, hi) = if d > 0.0 { (-lap, d) } else { (d, lap) };
            let expected = a * ((hi - lo) + (hi * hi - lo * lo) / (2.0 * r));
            let v = volume_of(&mut s, "Wrapped");
            assert!(
                (v - expected).abs() <= 1e-5 * expected,
                "k {k}, d {d}: {v} vs {expected}"
            );
            deep(&mut s);
        }
    }
}

/// The volume a wrapped text adds (join) or takes (cut), against the
/// closed form of a cylinder: `A (d + d² / 2r)` out, `A (d − d² / 2r)` in
/// (the curvature term is 2.5 % of it here), at three scales.
#[test]
fn embossed_and_engraved_on_a_cylinder() {
    for k in [1e-2, 1.0, 1e2] {
        let (r, h, d) = (20.0 * k, 40.0 * k, 1.0 * k);
        let can_volume = std::f64::consts::PI * r * r * h;
        let a = text_area("HOB", 8.0 * k);
        for (op, sign) in [("join", 1.0), ("cut", -1.0)] {
            let mut s = session();
            can(&mut s, k);
            let mut params = json!({"sketch": "Label", "wrap": "F1:face(0)", "distance": sign * d, "name": "Text"});
            params[op] = json!("Can");
            built(&mut s, "feature.extrude", params);
            let v = volume_of(&mut s, "Text");
            let term = sign * (v - can_volume);
            let expected = a * (d + sign * d * d / (2.0 * r));
            assert!(
                (term - expected).abs() <= 1e-5 * expected,
                "k {k}, {op}: the text's term {term} vs {expected}"
            );
            deep(&mut s);
        }
    }
}

/// A cone's face and the half-angle of its surface, for the wraps below:
/// `feature.cone` and a revolved frustum widening the other way.
fn cone_part(s: &mut Session, dir: &std::path::Path, widening: bool) -> (String, f64, f64) {
    if widening {
        run(
            s,
            "sketch.create",
            json!({"plane": "xy", "name": "Frustum"}),
        );
        run(
            s,
            "sketch.polyline",
            json!({"sketch": "Frustum", "closed": true, "points": [[0, 0], [20, 0], [30, 40], [0, 40]]}),
        );
        built(
            s,
            "feature.revolve",
            json!({"sketch": "Frustum", "axis": "y", "name": "Body"}),
        );
    } else {
        built(
            s,
            "feature.cone",
            json!({"radius": 30, "height": 60, "name": "Body"}),
        );
    }
    let body = body_of(s, dir, "Body").unwrap();
    let (i, half) = body
        .faces
        .iter()
        .enumerate()
        .find_map(|(i, f)| match &f.surface {
            craft3d_geom::Surface::Cone(c) => Some((i, c.half_angle())),
            _ => None,
        })
        .unwrap();
    let names = common::checked::body(s, "Body")["faceNames"].clone();
    let name = names[i].as_str().unwrap().to_string();
    (name, half, half.abs().pcos())
}

/// On a cone (both signs of the half-angle): the join adds `A d + K d²/2`
/// and the cut takes `A d − K d²/2`, with `K` the integral of the
/// curvature `cos α / R` of the parallel circles over the text, so their
/// sum is `2 A d` exactly and their difference `K d²` lies between
/// `A d² cos α / R` at the text's largest and smallest radius `R`.
#[test]
fn embossed_and_engraved_on_a_cone() {
    let d = 0.6;
    for widening in [false, true] {
        let mut terms = Vec::new();
        let mut radii = (f64::INFINITY, 0.0_f64);
        let mut halves = Vec::new();
        for (op, sign) in [("join", 1.0), ("cut", -1.0)] {
            let dir = common::workdir("wrap", &format!("cone-{widening}-{op}"));
            let mut s = common::session_in(&dir);
            run(&mut s, "file.new", json!({"units": "mm"}));
            let (face, half, _) = cone_part(&mut s, &dir, widening);
            halves.push(half);
            let base = volume_of(&mut s, "Body");
            run(&mut s, "sketch.create", json!({"name": "Label"}));
            run(
                &mut s,
                "sketch.text",
                json!({"sketch": "Label", "text": "CONE", "size": 5, "at": [-8, 4]}),
            );
            let mut params =
                json!({"sketch": "Label", "wrap": face, "distance": sign * d, "name": "Text"});
            params[op] = json!("Body");
            built(&mut s, "feature.extrude", params);
            deep(&mut s);
            terms.push(sign * (volume_of(&mut s, "Text") - base));
            // The text's radii about the axis, from the wrapped body.
            run(&mut s, "edit.undo", json!({}));
            let mut alone =
                json!({"sketch": "Label", "wrap": face, "distance": sign * d, "name": "Alone"});
            alone.as_object_mut().unwrap().remove(op);
            built(&mut s, "feature.extrude", alone);
            let wrapped = body_of(&mut s, &dir, "Alone").unwrap();
            let cone = body_of(&mut s, &dir, "Body").unwrap();
            let craft3d_geom::Surface::Cone(c) = &cone
                .faces
                .iter()
                .find(|f| matches!(f.surface, craft3d_geom::Surface::Cone(_)))
                .unwrap()
                .surface
            else {
                panic!("no cone")
            };
            // On the skin (the text on the surface itself): each vertex
            // of the plate taken to the surface along its normal, not the
            // corners at the depth or the lap.
            let surface = craft3d_geom::Surface::Cone(*c);
            for v in &wrapped.vertices {
                let (u, w) = surface.project(v.point).unwrap();
                let (x, y, _) = c.frame().local(surface.eval(u, w));
                radii.0 = radii.0.min(x.phypot(y));
                radii.1 = radii.1.max(x.phypot(y));
            }
        }
        let a = text_area("CONE", 5.0);
        let cos = halves[0].abs().pcos();
        let (join, cut) = (terms[0], terms[1]);
        assert!(
            (join + cut - 2.0 * a * d).abs() <= 1e-5 * a * d,
            "widening {widening}: {join} + {cut} vs {}",
            2.0 * a * d
        );
        let k = (join - cut) / (d * d);
        let (low, high) = (a * cos / radii.1, a * cos / radii.0);
        assert!(
            k >= low * (1.0 - 1e-6) && k <= high * (1.0 + 1e-6),
            "widening {widening}: K {k} not in [{low}, {high}]"
        );
    }
}

/// The two cones above have half-angles of both signs.
#[test]
fn the_cones_have_half_angles_of_both_signs() {
    let mut signs = Vec::new();
    for widening in [false, true] {
        let dir = common::workdir("wrap", &format!("cone-sign-{widening}"));
        let mut s = common::session_in(&dir);
        run(&mut s, "file.new", json!({"units": "mm"}));
        signs.push(cone_part(&mut s, &dir, widening).1.signum());
    }
    assert_ne!(signs[0], signs[1], "{signs:?}");
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

/// The apex: a profile that reaches a cone's apex is refused by name.
#[test]
fn a_profile_past_the_apex_is_refused() {
    let mut reasons = Vec::new();
    for y in [80.0, -80.0] {
        let mut s = session();
        run(&mut s, "file.new", json!({"units": "mm"}));
        built(&mut s, "feature.cone", json!({"radius": 30, "height": 60}));
        run(&mut s, "sketch.create", json!({"name": "L"}));
        run(
            &mut s,
            "sketch.text",
            json!({"sketch": "L", "text": "I", "size": 5, "at": [0, y]}),
        );
        reasons.push(failed(
            &mut s,
            "feature.extrude",
            json!({"sketch": "L", "wrap": "F1:face(0)", "distance": 0.3}),
        ));
    }
    // Past the apex one way, off the face the other.
    assert!(reasons.iter().any(|r| r.contains("apex")), "{reasons:?}");
    assert!(
        reasons.iter().any(|r| r.contains("leaves the face")),
        "{reasons:?}"
    );
}

/// `feature.edit` with a distance alone changes the depth and keeps the
/// wrap; with `wrap` it moves it to another face.
#[test]
fn editing_the_depth_keeps_the_wrap() {
    let mut s = session();
    can(&mut s, 1.0);
    built(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Label", "wrap": "F1:face(0)", "distance": 1, "name": "Wrapped"}),
    );
    let (r, a) = (20.0, text_area("HOB", 8.0));
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Wrapped", "distance": 0.5}),
    );
    let feature = common::checked::feature(&mut s, "Wrapped");
    assert!(feature.to_string().contains("wrap"), "{feature}");
    let lap = LAP * 0.5;
    let expected = a * ((0.5 + lap) + (0.25 - lap * lap) / (2.0 * r));
    let v = volume_of(&mut s, "Wrapped");
    assert!((v - expected).abs() <= 1e-5 * expected, "{v} vs {expected}");
    // Wrapped onto the same face by name again: the same body.
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Wrapped", "wrap": "F1:face(0)"}),
    );
    assert!((volume_of(&mut s, "Wrapped") - expected).abs() <= 1e-5 * expected);
    deep(&mut s);
}

/// Seen from outside the body, a text reads as in the sketch: on a tube's
/// outer wall the first glyph comes first round the axis the way the
/// surface's u runs, on its bore (a reversed face) the other way.
#[test]
fn wrapped_text_reads_from_outside_on_a_wall_and_in_a_bore() {
    let dir = common::workdir("wrap", "handedness");
    let mut s = common::session_in(&dir);
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "sketch.create",
        json!({"plane": "xy", "name": "Tube"}),
    );
    run(
        &mut s,
        "sketch.polyline",
        json!({"sketch": "Tube", "closed": true, "points": [[10, 0], [20, 0], [20, 40], [10, 40]]}),
    );
    built(
        &mut s,
        "feature.revolve",
        json!({"sketch": "Tube", "axis": "y", "name": "Body"}),
    );
    let tube = body_of(&mut s, &dir, "Body").unwrap();
    let names = common::checked::body(&mut s, "Body")["faceNames"].clone();
    for reversed in [false, true] {
        let (i, face) = tube
            .faces
            .iter()
            .enumerate()
            .find(|(_, f)| {
                matches!(f.surface, craft3d_geom::Surface::Cylinder(_)) && f.reversed == reversed
            })
            .unwrap();
        let name = names[i].as_str().unwrap();
        // The text in the middle of the face's height, in its own
        // parameters (each face's frame has an origin of its own).
        let vs: Vec<f64> = tube
            .vertices
            .iter()
            .filter_map(|v| face.surface.project(v.point).ok())
            .map(|(_, v)| v)
            .collect();
        let middle = 0.5
            * (vs.iter().copied().fold(f64::INFINITY, f64::min)
                + vs.iter().copied().fold(f64::NEG_INFINITY, f64::max));
        let label = format!("Label{reversed}");
        run(&mut s, "sketch.create", json!({"name": label}));
        run(
            &mut s,
            "sketch.text",
            json!({"sketch": label, "text": "HI", "size": 5, "at": [-4, middle - 2.5]}),
        );
        let wrapped = format!("Wrapped{reversed}");
        built(
            &mut s,
            "feature.extrude",
            json!({"sketch": label, "wrap": name, "distance": 0.5, "name": wrapped}),
        );
        let body = body_of(&mut s, &dir, &wrapped).unwrap();
        // Each glyph is a lump, in the text's order.
        let u: Vec<f64> = body
            .shells
            .iter()
            .map(|shell| {
                let mut sum = craft3d_math::Vec3::ZERO;
                let mut n = 0.0;
                for f in &shell.faces {
                    for l in &body.face(*f).unwrap().loops {
                        for c in &body.loop_(*l).unwrap().coedges {
                            sum += body.vertex(body.edge(c.edge).unwrap().start).unwrap().point;
                            n += 1.0;
                        }
                    }
                }
                face.surface.project(sum * (1.0 / n)).unwrap().0
            })
            .collect();
        assert_eq!(u.len(), 2, "{u:?}");
        // From outside the wall u runs to the right; from inside the bore
        // to the left.
        let tau = std::f64::consts::TAU;
        let ahead = (u[1] - u[0]).rem_euclid(tau) < 0.5 * tau;
        assert_eq!(ahead, !reversed, "reversed {reversed}: {u:?}");
        deep(&mut s);
    }
}

/// The faces of a wrapped body have names of their own, and a feature
/// that refers to a rim keeps it when the cylinder grows.
#[test]
fn rim_names_are_unique_and_survive_a_change_of_radius() {
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    built(
        &mut s,
        "feature.cylinder",
        json!({"radius": 20, "height": 40, "name": "Can"}),
    );
    run(&mut s, "sketch.create", json!({"name": "Label"}));
    run(
        &mut s,
        "sketch.text",
        json!({"sketch": "Label", "text": "3DCraft", "size": 6, "at": [-10, 12]}),
    );
    built(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Label", "wrap": "F1:face(0)", "distance": 1, "name": "Text"}),
    );
    let names: Vec<String> = common::checked::body(&mut s, "Text")["faceNames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_str().unwrap().to_string())
        .collect();
    let mut unique = names.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), names.len(), "{names:?}");
    // A plate on the cylinder with flat rims, and a face offset that
    // names one of them.
    run(&mut s, "sketch.create", json!({"name": "Block"}));
    run(
        &mut s,
        "sketch.polyline",
        json!({"sketch": "Block", "closed": true, "points": [[-3, 25], [3, 25], [3, 31], [-3, 31]]}),
    );
    built(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Block", "wrap": "F1:face(0)", "distance": 1, "name": "Plate"}),
    );
    let plate: Vec<String> = common::checked::body(&mut s, "Plate")["faceNames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_str().unwrap().to_string())
        .collect();
    let rim = plate.iter().find(|n| n.contains("side(")).unwrap().clone();
    built(
        &mut s,
        "feature.offsetFace",
        json!({"faces": [rim.clone()], "distance": 0.2, "name": "Pushed"}),
    );
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Can", "radius": 24}),
    );
    let pushed = common::checked::feature(&mut s, "Pushed");
    assert_eq!(pushed["status"], "ok", "{pushed}");
    assert!(pushed.to_string().contains(&rim), "{pushed}");
    deep(&mut s);
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

/// A face cut in two by a later feature (a groove round the can) keeps
/// one name for both pieces: wrapping onto that name is refused, naming
/// how many faces it names.
#[test]
fn a_wrap_name_of_several_faces_is_refused() {
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    built(
        &mut s,
        "feature.cylinder",
        json!({"radius": 20, "height": 40, "name": "Can"}),
    );
    built(
        &mut s,
        "feature.torus",
        json!({"major": 20, "minor": 2, "position": [0, 0, 20], "name": "Ring"}),
    );
    built(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Can", "tool": "Ring", "name": "Grooved"}),
    );
    let names = common::checked::body(&mut s, "Grooved")["faceNames"].clone();
    let names: Vec<&str> = names
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_str().unwrap())
        .collect();
    let split = names
        .iter()
        .find(|n| n.ends_with("F1:face(0)") && names.iter().filter(|m| m == n).count() == 2)
        .unwrap_or_else(|| panic!("the can's side is not in two pieces: {names:?}"))
        .to_string();
    run(&mut s, "sketch.create", json!({"name": "L"}));
    run(
        &mut s,
        "sketch.text",
        json!({"sketch": "L", "text": "X", "size": 5, "at": [0, 5]}),
    );
    let why = failed(
        &mut s,
        "feature.extrude",
        json!({"sketch": "L", "wrap": split, "distance": 0.5}),
    );
    assert!(why.contains("names 2 faces"), "{why}");
}

/// Half a turn on a steep cone (a narrow development sector): a text
/// that reaches round to the back of the surface is refused there too.
#[test]
fn past_half_a_turn_on_a_steep_cone_is_refused() {
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    built(
        &mut s,
        "feature.cone",
        json!({"radius": 3, "height": 80, "name": "Spike"}),
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
}
