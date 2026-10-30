//! Surface analysis through the engine (docs/design/surfacing.md, stage
//! 1): the curvature map's values from the exact surfaces (a sphere's are
//! 1/R² and 1/R everywhere, a torus's the closed form at every vertex);
//! the measures `curvature` at a point and `draft` against closed forms;
//! zebra stripes that kink at a seam that is only G1 and do not at a G2
//! seam, measured in the picture; `view.render` with an analysis; bad
//! parameters are errors.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::{body, built, error, new_document, run, session_in, workdir};
use craft3d_engine::Session;
use craft3d_engine::analysis::{CurvatureKind, vertex_values};
use craft3d_math::Vec3;
use craft3d_render::analysis::{Analysis, AnalysisMode};
use craft3d_render::{Camera, FaceColors, Projection, RenderParams, Viewpoint, render_analysed};
use serde_json::{Value, json};

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

fn mm(x: f64) -> String {
    format!("{x} mm")
}

fn number(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("not a number: {v}"))
}

fn near(actual: f64, expected: f64, relative: f64, what: &str) {
    assert!(
        (actual - expected).abs() <= relative * expected.abs().max(f64::MIN_POSITIVE),
        "{what}: {actual}, expected {expected}"
    );
}

/// A query for the face named.
fn faces(names: &[String]) -> Value {
    json!({"kind": "face", "where": {"name": names.first().unwrap()}})
}

fn scene(s: &mut Session, tolerance: f64) -> std::sync::Arc<craft3d_engine::Scene> {
    s.scene(Some(tolerance)).unwrap().unwrap()
}

/// A sphere's curvature map is 1/R² (Gaussian) and 1/R (mean, positive:
/// convex; and the largest) at every vertex: the values come from the
/// exact surface, not from the mesh.
#[test]
fn a_spheres_curvature_map_is_constant() {
    for k in SCALES {
        let mut s = session_in(&workdir("analysis", &format!("sphere-{k}")));
        new_document(&mut s, k);
        let r = 7.0 * k;
        built(
            &mut s,
            "feature.sphere",
            json!({"radius": mm(r), "name": "Ball"}),
        );
        let sc = scene(&mut s, r * 1e-3);
        for (kind, expected) in [
            (CurvatureKind::Gaussian, 1.0 / (r * r)),
            (CurvatureKind::Mean, 1.0 / r),
            (CurvatureKind::Max, 1.0 / r),
        ] {
            let values = vertex_values(&mut s, &sc, kind).unwrap();
            assert_eq!(values.len(), sc.mesh.positions.len());
            let finite: Vec<f32> = values.iter().copied().filter(|v| v.is_finite()).collect();
            // The poles may have no normal in the sphere's parameters.
            assert!(finite.len() * 100 >= values.len() * 99, "{kind:?}");
            for v in finite {
                near(f64::from(v), expected, 1e-5, &format!("{kind:?}"));
            }
        }
    }
}

/// A torus's Gaussian curvature at every vertex is the closed form
/// `cos v / (r (R + r cos v))`, `cos v = (ρ − R) / r` with ρ the distance
/// from the axis; and the measure at three points.
#[test]
fn a_torus_has_its_known_gaussian_curvature() {
    for k in SCALES {
        let mut s = session_in(&workdir("analysis", &format!("torus-{k}")));
        new_document(&mut s, k);
        let (big, small) = (10.0 * k, 3.0 * k);
        built(
            &mut s,
            "feature.torus",
            json!({"major": mm(big), "minor": mm(small), "name": "Ring"}),
        );
        let sc = scene(&mut s, small * 1e-3);
        let values = vertex_values(&mut s, &sc, CurvatureKind::Gaussian).unwrap();
        let mut checked = 0;
        for (p, v) in sc.mesh.positions.iter().zip(&values) {
            let rho = (p.x * p.x + p.y * p.y).sqrt();
            let cos = ((rho - big) / small).clamp(-1.0, 1.0);
            let expected = cos / (small * (big + small * cos));
            assert!(
                (f64::from(*v) - expected).abs() <= 1e-4 / (small * small),
                "at {p:?}: {v}, expected {expected}"
            );
            checked += 1;
        }
        assert!(checked > 100);
        for (at, expected) in [
            ([big + small, 0.0, 0.0], 1.0 / (small * (big + small))),
            ([big - small, 0.0, 0.0], -1.0 / (small * (big - small))),
            ([0.0, big, small], 0.0),
        ] {
            let m = run(&mut s, "measure.curvature", json!({"at": at}));
            let kg = number(&m["gaussian"]);
            assert!(
                (kg - expected).abs() <= 1e-9 / (small * small),
                "at {at:?}: {m}"
            );
            assert!(number(&m["distance"]) <= 1e-9 * k, "{m}");
        }
        // The mean curvature on the outer equator: (1/r + 1/(R + r)) / 2,
        // positive (convex).
        let m = run(
            &mut s,
            "measure.curvature",
            json!({"at": [big + small, 0.0, 0.0]}),
        );
        near(
            number(&m["mean"]),
            0.5 * (1.0 / small + 1.0 / (big + small)),
            1e-9,
            "mean",
        );
        near(number(&m["k1"]), 1.0 / small, 1e-9, "k1");
    }
}

/// A block drafted 7° from its bottom: its sides' draft along +z is 7°,
/// exactly (planes), the bottom's −90°; an assertion on it passes and
/// fails as it should.
#[test]
fn the_draft_of_a_drafted_block() {
    for k in SCALES {
        let mut s = session_in(&workdir("analysis", &format!("draft-{k}")));
        new_document(&mut s, k);
        built(
            &mut s,
            "feature.block",
            json!({"name": "Base", "size": [10.0 * k, 10.0 * k, 5.0 * k]}),
        );
        built(
            &mut s,
            "feature.draft",
            json!({"faces": ["F1:face(2)", "F1:face(3)", "F1:face(4)", "F1:face(5)"],
                   "neutral": "F1:face(0)", "angle": "7 deg", "name": "Tapered"}),
        );
        // The sides by their names on the drafted body (faces 2 to 5).
        let names: Vec<String> = body(&mut s, "Tapered")["faceNames"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n.as_str().unwrap().to_string())
            .collect();
        // A query's list of filters is all of them at once: one face
        // each.
        for side in &names[2..6] {
            let m = run(
                &mut s,
                "measure.draft",
                json!({"faces": faces(std::slice::from_ref(side)), "direction": "z"}),
            );
            near(number(&m["draft"]), 7.0, 1e-9, side);
        }
        let sides = faces(&names[2..3]);
        let m = run(
            &mut s,
            "measure.draft",
            json!({"faces": sides, "direction": "z", "minDraft": 5}),
        );
        near(number(&m["draft"]), 7.0, 1e-9, "the sides' draft");
        assert_eq!(m["proven"], true, "{m}");
        assert_eq!(number(&m["shareBelow"]), 0.0, "{m}");
        let m = run(
            &mut s,
            "measure.draft",
            json!({"faces": sides, "direction": "z", "minDraft": 8}),
        );
        near(number(&m["shareBelow"]), 1.0, 1e-12, "all below 8°");
        // Against the pull the sides are drafted −7°.
        let m = run(
            &mut s,
            "measure.draft",
            json!({"faces": sides, "direction": "-z"}),
        );
        near(number(&m["draft"]), -7.0, 1e-9, "against");
        // The whole body: the bottom faces straight against the pull.
        let m = run(&mut s, "measure.draft", json!({"faces": "Tapered"}));
        near(number(&m["draft"]), -90.0, 1e-9, "the bottom");
        // As assertions.
        for (name, bound, ok) in [("enough", "5 deg", true), ("tooMuch", "8 deg", false)] {
            run(
                &mut s,
                "assert.add",
                json!({"name": name, "measure": "draft", "faces": sides, "direction": "z",
                       "min": bound}),
            );
            let list = run(&mut s, "assert.list", json!({}));
            let found = list["assertions"]
                .as_array()
                .unwrap()
                .iter()
                .find(|a| a["name"] == name)
                .cloned()
                .unwrap();
            let expected = if ok { "passed" } else { "failed" };
            assert_eq!(found["status"], expected, "{found}");
        }
    }
}

/// A cone's draft is sampled, not proven: a pin drafted 5° from its top.
#[test]
fn a_cone_is_sampled() {
    let mut s = session_in(&workdir("analysis", "cone"));
    new_document(&mut s, 1.0);
    built(
        &mut s,
        "feature.cylinder",
        json!({"name": "Rod", "radius": 2.0, "height": 6.0}),
    );
    built(
        &mut s,
        "feature.draft",
        json!({"faces": ["F1:face(0)"], "neutral": "F1:face(1)", "angle": "5 deg", "name": "Pin"}),
    );
    let pin = body(&mut s, "Pin");
    let wall = pin["faceNames"][0].as_str().unwrap().to_string();
    let m = run(
        &mut s,
        "measure.draft",
        json!({"faces": faces(&[wall]), "direction": "-z"}),
    );
    assert_eq!(m["proven"], false, "{m}");
    assert!((number(&m["draft"]).abs() - 5.0).abs() < 0.01, "{m}");
}

/// Builds two quarter cylinders of radius `r` and height `h` (a Coons
/// patch each, `feature.surface.coons`): one from 0° to 90°, and beside it
/// along the line at 90° either the next quarter of the same cylinder (the
/// seam is G2: one smooth surface) or an arc of radius `3 r` tangent to it
/// there (the seam is G1 only: the curvature jumps from 1/r to 1/(3r)).
fn seam_pair(s: &mut Session, r: f64, h: f64, g2: bool) {
    let arc =
        |s: &mut Session, name: &str, z: f64, center: [f64; 2], start: [f64; 2], end: [f64; 2]| {
            built(
                s,
                "sketch.create",
                json!({"plane": "xy", "offset": mm(z), "name": name}),
            );
            run(
                s,
                "sketch.arc",
                json!({"sketch": name, "center": center, "start": start, "end": end}),
            );
        };
    let line =
        |s: &mut Session, name: &str, plane: &str, offset: f64, from: [f64; 2], to: [f64; 2]| {
            built(
                s,
                "sketch.create",
                json!({"plane": plane, "offset": mm(offset), "name": name}),
            );
            run(
                s,
                "sketch.line",
                json!({"sketch": name, "from": from, "to": to}),
            );
        };
    arc(s, "LowA", 0.0, [0.0, 0.0], [r, 0.0], [0.0, r]);
    arc(s, "HighA", h, [0.0, 0.0], [r, 0.0], [0.0, r]);
    // zx: sketch (x, y) is world (y, 0, x); yz: world (0, x, y).
    line(s, "Zero", "zx", 0.0, [0.0, r], [h, r]);
    line(s, "Seam", "yz", 0.0, [r, 0.0], [r, h]);
    built(
        s,
        "feature.surface.coons",
        json!({"curves": ["LowA", "Seam", "HighA", "Zero"], "name": "A"}),
    );
    let (center, radius) = if g2 {
        ([0.0, 0.0], r)
    } else {
        ([0.0, r - 3.0 * r], 3.0 * r)
    };
    // The arc from the seam over 1.2 r of its length, turning towards -x.
    let turn = 1.2 * r / radius;
    let end = [
        center[0] - radius * turn.sin(),
        center[1] + radius * turn.cos(),
    ];
    arc(s, "LowB", 0.0, center, [0.0, r], end);
    arc(s, "HighB", h, center, [0.0, r], end);
    // The far line at the arc's end: on a vertical plane through it.
    built(
        s,
        "sketch.create",
        json!({"plane": "yz", "offset": mm(end[0]), "name": "Far"}),
    );
    run(
        s,
        "sketch.line",
        json!({"sketch": "Far", "from": [end[1], 0.0], "to": [end[1], h]}),
    );
    built(
        s,
        "feature.surface.coons",
        json!({"curves": ["LowB", "Far", "HighB", "Seam"], "name": "B"}),
    );
}

/// The kinks of the zebra stripes where they cross the seam, in degrees:
/// every boundary between a dark and a light stripe near the seam is
/// traced on each side (the pixels of one face that change colour, joined
/// into curves), the curve's direction where it meets the seam is fitted
/// (a quadratic of the position along the seam against the distance from
/// it, within 14 pixels), and the two sides' directions of one stripe
/// edge are compared.
fn stripe_kinks(frame: &craft3d_render::Frame, ids: [u32; 2], seam: [[f64; 2]; 2]) -> Vec<f64> {
    let (w, h) = (frame.width as usize, frame.height as usize);
    let dark = |i: usize| frame.rgb[3 * i] < 128;
    let side = |id: u32| ids.iter().position(|&f| f == id);
    let mut edge = vec![false; w * h];
    for y in 0..h - 1 {
        for x in 0..w - 1 {
            let i = y * w + x;
            for j in [i + 1, i + w] {
                if side(frame.ids[i]).is_some()
                    && frame.ids[i] == frame.ids[j]
                    && dark(i) != dark(j)
                {
                    edge[i] = true;
                }
            }
        }
    }
    let [p0, p1] = seam;
    let length = (p1[0] - p0[0]).hypot(p1[1] - p0[1]);
    let u = [(p1[0] - p0[0]) / length, (p1[1] - p0[1]) / length];
    let n = [-u[1], u[0]];
    let mut seen = vec![false; w * h];
    let mut ends: [Vec<(f64, f64)>; 2] = [Vec::new(), Vec::new()];
    for start in 0..w * h {
        if !edge[start] || seen[start] {
            continue;
        }
        let id = frame.ids[start];
        let mut stack = vec![start];
        seen[start] = true;
        let mut points = Vec::new();
        while let Some(i) = stack.pop() {
            let (x, y) = (i % w, i / w);
            points.push((x as f64 + 0.5, y as f64 + 0.5));
            for dy in -1_i64..=1 {
                for dx in -1_i64..=1 {
                    let (xx, yy) = (x as i64 + dx, y as i64 + dy);
                    if xx < 0 || yy < 0 || xx >= w as i64 || yy >= h as i64 {
                        continue;
                    }
                    let j = yy as usize * w + xx as usize;
                    if edge[j] && !seen[j] && frame.ids[j] == id {
                        seen[j] = true;
                        stack.push(j);
                    }
                }
            }
        }
        let st: Vec<(f64, f64)> = points
            .iter()
            .map(|&(x, y)| {
                let (dx, dy) = (x - p0[0], y - p0[1]);
                (dx * u[0] + dy * u[1], dx * n[0] + dy * n[1])
            })
            .filter(|&(s, t)| t.abs() <= 14.0 && s > 0.15 * length && s < 0.85 * length)
            .collect();
        if st.len() < 6 || st.iter().map(|p| p.1.abs()).fold(f64::INFINITY, f64::min) > 2.5 {
            continue;
        }
        // Least squares of s = a + b t + c t².
        let mut m = [[0.0_f64; 3]; 3];
        let mut v = [0.0_f64; 3];
        for &(s, t) in &st {
            let row = [1.0, t, t * t];
            for a in 0..3 {
                v[a] += row[a] * s;
                for b in 0..3 {
                    m[a][b] += row[a] * row[b];
                }
            }
        }
        let Some([a, b, _]) = solve3(m, v) else {
            continue;
        };
        if let Some(k) = side(id) {
            ends[k].push((a, b));
        }
    }
    let mut kinks = Vec::new();
    for &(sa, ba) in &ends[0] {
        let best = ends[1]
            .iter()
            .min_by(|x, y| (x.0 - sa).abs().total_cmp(&(y.0 - sa).abs()));
        if let Some(&(sb, bb)) = best
            && (sb - sa).abs() <= 2.0
        {
            kinks.push((ba.atan() - bb.atan()).abs().to_degrees());
        }
    }
    kinks
}

fn solve3(m: [[f64; 3]; 3], v: [f64; 3]) -> Option<[f64; 3]> {
    let det = |m: [[f64; 3]; 3]| {
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
            - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    };
    let d = det(m);
    if d.abs() < 1e-12 {
        return None;
    }
    let mut out = [0.0; 3];
    for (k, o) in out.iter_mut().enumerate() {
        let mut mk = m;
        for r in 0..3 {
            mk[r][k] = v[r];
        }
        *o = det(mk) / d;
    }
    Some(out)
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

/// Zebra stripes kink where they cross a seam that is G1 only and do not
/// where it is G2: the median kink of the stripe edges across the seam is
/// below 4° at the G2 seam and above 8° at the G1 one (and three times
/// that of the G2 seam). The view looks down along the seam in
/// perspective, so that the stripes cross it at a slant.
#[test]
fn zebra_stripes_kink_at_a_g1_seam_and_not_at_a_g2_seam() {
    let (r, h) = (40.0, 60.0);
    let mut medians = Vec::new();
    for g2 in [true, false] {
        let mut s = session_in(&workdir("analysis", &format!("seam-{g2}")));
        new_document(&mut s, 1.0);
        seam_pair(&mut s, r, h, g2);
        let sc = scene(&mut s, 2e-3);
        let camera = Camera {
            eye: Vec3::new(-15.0, 75.0, 80.0),
            target: Vec3::new(0.0, r, 30.0),
            up: Vec3::Z,
            projection: Projection::Perspective { fov_y_deg: 40.0 },
        };
        let mut analysis = Analysis::new(AnalysisMode::Zebra);
        analysis.stripes = 64.0;
        let params = RenderParams {
            width: 500,
            height: 500,
            ssaa: 1,
            analysis: Some(analysis),
            ..RenderParams::default()
        };
        let frame = render_analysed(&sc.mesh, &FaceColors::new(), &[], &camera, &params).unwrap();
        let view = Viewpoint::new(&camera, 500, 500).unwrap();
        let at = |z: f64| {
            let p = view.project(Vec3::new(0.0, r, z)).unwrap();
            [p[0], p[1]]
        };
        let ids = [sc.bodies[0].first_face_id, sc.bodies[1].first_face_id];
        let kinks = stripe_kinks(&frame, ids, [at(0.0), at(h)]);
        assert!(
            kinks.len() >= 4,
            "g2 {g2}: {} stripe edges across",
            kinks.len()
        );
        let m = median(kinks.clone());
        println!("g2 {g2}: kinks {kinks:?}, median {m}");
        let png = craft3d_render::encode_png(frame.width, frame.height, &frame.rgb).unwrap();
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/evidence/analysis");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(format!("seam-{}.png", if g2 { "g2" } else { "g1" })),
            png,
        )
        .unwrap();
        medians.push(m);
    }
    let (g2, g1) = (medians[0], medians[1]);
    assert!(g2 < 4.0, "the G2 seam kinks the stripes by {g2}°");
    assert!(g1 > 8.0, "the G1 seam kinks the stripes by only {g1}°");
    assert!(g1 > 3.0 * g2, "G1 {g1}° against G2 {g2}°");
}

/// `view.render` with each analysis: a picture, the analysis reported with
/// its range; the same bytes twice; bad analyses are errors.
#[test]
fn view_render_draws_an_analysis_and_refuses_bad_ones() {
    let mut s = session_in(&workdir("analysis", "render"));
    new_document(&mut s, 1.0);
    built(
        &mut s,
        "feature.torus",
        json!({"major": "10 mm", "minor": "3 mm", "name": "Ring"}),
    );
    for analysis in [
        json!({"mode": "zebra", "stripes": 24}),
        json!({"mode": "highlight"}),
        json!({"mode": "curvature", "curvature": "mean"}),
        json!({"mode": "draft", "pull": [0, 0, 1], "minDraft": 3}),
        json!({"mode": "none", "combs": {"edges": true}}),
        json!({"mode": "zebra", "combs": {"iso": [{"face": "F1:face(0)", "u": 0.3}], "density": 30}}),
    ] {
        let params = json!({"path": "a.png", "width": 160, "height": 120, "analysis": analysis});
        let one = run(&mut s, "view.render", params.clone());
        let a = &one["analysis"];
        assert!(a.is_object(), "{analysis}: {one}");
        assert!(a["range"].is_array(), "{one}");
        let first = std::fs::read(s.workspace().resolve_read("a.png").unwrap()).unwrap();
        run(&mut s, "view.render", params);
        let second = std::fs::read(s.workspace().resolve_read("a.png").unwrap()).unwrap();
        assert_eq!(first, second, "{analysis}: deterministic");
    }
    let curvature = run(
        &mut s,
        "view.render",
        json!({"path": "c.png", "width": 160, "height": 120, "analysis": {"mode": "curvature"}}),
    );
    // A torus's Gaussian curvature runs from −1/(r(R−r)) to 1/(r(R+r)):
    // the range is symmetric and within the largest magnitude.
    let hi = number(&curvature["analysis"]["range"][1]);
    assert!(hi > 0.0 && hi <= 1.0 / (3.0 * 7.0) + 1e-6, "{curvature}");
    for (bad, words) in [
        (json!({"mode": "rainbow"}), "unknown mode"),
        (json!({"stripes": 0}), "stripes"),
        (json!({"width": 3}), "width"),
        (json!({"pull": [0, 0, 0]}), "pull"),
        (json!({"range": [2, 1]}), "range"),
        (json!({"curvature": "wobbly"}), "curvature"),
        (json!({"combs": {"edges": "all"}}), "edges"),
        (json!({"combs": {"iso": [{"face": "F1:face(0)"}]}}), "iso"),
        (json!({"combs": {"density": 1.5}}), "density"),
        (json!({"zebra": true}), "unknown key"),
        (json!("zebra"), "object"),
    ] {
        let why = error(
            &mut s,
            "view.render",
            json!({"path": "b.png", "analysis": bad}),
        );
        assert!(why.contains(words), "{bad}: {why}");
    }
    let why = error(
        &mut s,
        "view.render",
        json!({"path": "b.png", "analysis": {"mode": "zebra", "combs": {"edges": ["F9:edge(0)"]}}}),
    );
    assert!(why.contains("no edge"), "{why}");
    let why = error(
        &mut s,
        "view.render",
        json!({"path": "b.png", "analysis": {"mode": "zebra", "combs": {"iso": [{"face": "F9:face(0)", "v": 0.5}]}}}),
    );
    assert!(why.contains("no face"), "{why}");
}

/// The measures refuse what they cannot take.
#[test]
fn the_measures_refuse_bad_parameters() {
    let mut s = session_in(&workdir("analysis", "measures"));
    new_document(&mut s, 1.0);
    built(
        &mut s,
        "feature.sphere",
        json!({"radius": "5 mm", "name": "Ball"}),
    );
    for (id, params) in [
        ("measure.draft", json!({"direction": [0, 0, 0]})),
        ("measure.draft", json!({"minDraft": 120})),
        ("measure.draft", json!({"minDraft": "lots"})),
        ("measure.draft", json!({"faces": "Nothing"})),
        ("measure.draft", json!({"min": 3})),
        ("measure.curvature", json!({})),
        ("measure.curvature", json!({"at": [1, 2]})),
        ("measure.curvature", json!({"at": "here"})),
        (
            "measure.curvature",
            json!({"at": [0, 0, 0], "of": "Nothing"}),
        ),
    ] {
        error(&mut s, id, params);
    }
    // At the sphere's pole: no normal in its parameters, said so, or its
    // curvature; never a crash.
    let pole = s.execute("measure.curvature", &json!({"at": [0.0, 0.0, 5.0]}));
    if let Ok(m) = pole {
        near(number(&m["gaussian"]), 1.0 / 25.0, 1e-6, "at the pole");
    }
}

/// The draft of curved faces against closed forms, with the bound below
/// holding the true least: a sphere's and a torus's least is −90° (the
/// bottom), a cylinder tilted 30° from the pull has its side's least at
/// exactly −30°, proven tight (the cylinder's bound is exact over the
/// triangles' angles), so an assertion on it is decided, not notProven,
/// also for a long cylinder (height twenty times the radius).
#[test]
fn the_draft_of_curved_faces_is_bounded() {
    for k in SCALES {
        let mut s = session_in(&workdir("analysis", &format!("curved-{k}")));
        new_document(&mut s, k);
        built(
            &mut s,
            "feature.sphere",
            json!({"radius": mm(5.0 * k), "name": "Ball"}),
        );
        let m = run(&mut s, "measure.draft", json!({"faces": "Ball"}));
        let (least, lower) = (number(&m["draft"]), number(&m["atLeast"]));
        assert!(lower <= -90.0 + 1e-9 && least >= -90.0 - 1e-9, "{m}");
        assert!(least <= -85.0 && lower >= -110.0, "{m}");
        assert_eq!(m["proven"], true, "{m}");

        built(
            &mut s,
            "feature.torus",
            json!({"major": mm(10.0 * k), "minor": mm(3.0 * k), "position": [mm(40.0 * k), "0 mm", "0 mm"], "name": "Ring"}),
        );
        let m = run(&mut s, "measure.draft", json!({"faces": "Ring"}));
        let (least, lower) = (number(&m["draft"]), number(&m["atLeast"]));
        assert!(lower <= -90.0 + 1e-9 && least >= -90.0 - 1e-9, "{m}");
        assert!(least <= -85.0 && lower >= -110.0, "{m}");

        let (sin, cos) = 30f64.to_radians().sin_cos();
        built(
            &mut s,
            "feature.cylinder",
            json!({"radius": mm(2.0 * k), "height": mm(40.0 * k), "position": [mm(-40.0 * k), "0 mm", "0 mm"],
                   "axis": [0.0, sin, cos], "name": "Pole"}),
        );
        let pole = body(&mut s, "Pole");
        let wall = pole["faceNames"][0].as_str().unwrap().to_string();
        let m = run(
            &mut s,
            "measure.draft",
            json!({"faces": faces(std::slice::from_ref(&wall))}),
        );
        let (least, lower) = (number(&m["draft"]), number(&m["atLeast"]));
        assert!((lower + 30.0).abs() <= 1e-9, "the bound is exact: {m}");
        assert!((-30.0 - 1e-9..=-29.0).contains(&least), "{m}");
        assert_eq!(m["proven"], true, "{m}");
        run(
            &mut s,
            "assert.add",
            json!({"name": "pole", "measure": "draft", "faces": faces(&[wall]), "min": "-30.5 deg"}),
        );
        let list = run(&mut s, "assert.list", json!({}));
        let found = list["assertions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["name"] == "pole")
            .cloned()
            .unwrap();
        assert_eq!(found["status"], "passed", "{found}");
    }
}
