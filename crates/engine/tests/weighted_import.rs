//! Bodies with faces on rational surfaces whose weights are far apart,
//! through the commands: read from a STEP file (no command draws weights
//! that far apart: such a face comes in through import), regenerated,
//! drawn, exported as STL and 3MF, written as STEP and read back, and
//! cut by a boolean.
//!
//! Until 2026-11-16 such a body was read and could be neither drawn nor
//! exported (*"patch … needs about … segments per side for this
//! tolerance"*: the tessellator sized the face's grid by the pace its
//! surface is drawn at, `docs/design/tessellator.md` › *A grid graded to
//! the shape*), and a boolean with it failed (*"the result cannot be
//! meshed to see whether it passes through itself"*).
//!
//! The bodies are shapes with closed forms, drawn unevenly (as
//! `crates/kernel/tests/rational_mesh.rs` has them): a block 30 by 900 by
//! 20 whose three outer sides are planes drawn as bicubic sheets with
//! seeded weights from 1e-3 to 1e3; a quarter of a rod whose curved side
//! is the cylinder as a rational patch, each span redrawn so that its end
//! weights are ten thousand apart both ways; and that rod, drawn a
//! thousand apart, less a block through its curved side, cut by the
//! kernel before it is written (the curves of that cut have knots a
//! billionth of the size apart, which the tessellator takes for points).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::Portable;
mod common;
#[path = "../../kernel/tests/rational_common/mod.rs"]
mod rational_common;

use common::{session_in, workdir};
use craft3d_engine::Session;
use craft3d_geom::{NurbsSurface, Surface};
use craft3d_io::step::{StepBody, StepOptions, export_step};
use craft3d_kernel::boolean::{BooleanOp, boolean};
use craft3d_kernel::primitives;
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_topo::{Body, check, mass_properties};
use rational_common::{rationalized, skewed_apart};
use serde_json::{Value, json};
use std::f64::consts::PI;
use std::path::Path;

fn tolerance() -> Tolerance {
    Tolerance::new(1e-6, 1e-9).unwrap()
}

/// The block of `classify_far.rs` and `rational_mesh.rs`: its sides
/// facing +x, +y and +z flat bicubic sheets of three spans each way with
/// seeded weights from 1e-3 to 1e3.
fn seeded_block(size: Vec3, seed: u64) -> Body {
    let mut body = primitives::block(&Frame::WORLD, size.x, size.y, size.z).unwrap();
    let mut state = seed;
    let knots = vec![0.0, 0.0, 0.0, 0.0, 1.0 / 3.0, 2.0 / 3.0, 1.0, 1.0, 1.0, 1.0];
    for index in 0..body.faces.len() {
        let Surface::Plane(plane) = &body.faces[index].surface else {
            continue;
        };
        let f = *plane.frame();
        if !(f.z().x > 0.99 || f.z().y > 0.99 || f.z().z > 0.99) {
            continue;
        }
        let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
        for vertex in &body.vertices {
            let (x, y, z) = f.local(vertex.point);
            if z.abs() <= 1e-9 * size.length() {
                lo = [lo[0].min(x), lo[1].min(y)];
                hi = [hi[0].max(x), hi[1].max(y)];
            }
        }
        let (mut ctrl, mut weights) = (Vec::new(), Vec::new());
        for i in 0..6 {
            for j in 0..6 {
                ctrl.push(f.point(
                    lo[0] + (hi[0] - lo[0]) * i as f64 / 5.0,
                    lo[1] + (hi[1] - lo[1]) * j as f64 / 5.0,
                    0.0,
                ));
                state = state
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                let next = (state >> 11) as f64 / (1u64 << 53) as f64;
                weights.push((6.0 * (next - 0.5) * std::f64::consts::LN_10).pexp());
            }
        }
        body.faces[index].surface = Surface::Nurbs(
            NurbsSurface::new(3, 3, knots.clone(), knots.clone(), 6, 6, ctrl, weights).unwrap(),
        );
    }
    body
}

/// A quarter of a rod of radius 2 and height 3, with its curved side on
/// the cylinder, and the same with that side on the rational patch that
/// is the cylinder.
fn quarter_rod() -> (Body, Body) {
    let rod = primitives::cylinder(&Frame::WORLD, 2.0, 3.0).unwrap();
    let corner = primitives::block(
        &Frame::WORLD.at(Vec3::new(0.0, 0.0, -1.0)).unwrap(),
        3.0,
        3.0,
        5.0,
    )
    .unwrap();
    let plain = boolean(&rod, &corner, BooleanOp::Intersect, &tolerance())
        .unwrap()
        .body;
    let (twin, replaced) = rationalized(&plain, 0.2, true);
    assert_eq!(replaced, 1);
    (plain, twin)
}

/// The block through the rod's curved side, in the kernel and as the
/// parameters of `feature.block`.
fn tool() -> (Body, Value) {
    let at = Vec3::new(1.0, 0.7, 1.1);
    let size = Vec3::new(1.6, 1.6, 1.1);
    (
        primitives::block(&Frame::WORLD.at(at).unwrap(), size.x, size.y, size.z).unwrap(),
        json!({"name": "Tool", "size": [size.x, size.y, size.z], "position": [at.x, at.y, at.z]}),
    )
}

/// Writes `body` as `<name>.step` in `dir`, in millimetres.
fn written(dir: &Path, name: &str, body: &Body) {
    assert_eq!(check(body, &tolerance()), [], "{name}");
    let options = StepOptions {
        tolerance: tolerance(),
        ..StepOptions::default()
    };
    let (bytes, summary) = export_step(&[StepBody::new(name, body)], &options).unwrap();
    assert_eq!(summary.bodies, 1, "{name}");
    std::fs::write(dir.join(format!("{name}.step")), bytes).unwrap();
}

/// What a command answers, or its error in a few words.
fn tried(session: &mut Session, id: &str, params: Value) -> Result<Value, String> {
    session.execute(id, &params).map_err(|e| e.to_string())
}

/// A document in `dir` with `<name>.step` imported: the session and the
/// one body the import made, as `file.import` answers it.
fn imported(dir: &Path, name: &str) -> (Session, Value) {
    let mut s = session_in(dir);
    tried(
        &mut s,
        "file.new",
        json!({"units": "mm", "tolerance": 1e-6}),
    )
    .unwrap();
    let result = tried(
        &mut s,
        "file.import",
        json!({"path": format!("{name}.step")}),
    )
    .unwrap_or_else(|e| panic!("{name}: {e}"));
    let made = result["bodies"].as_array().unwrap();
    assert_eq!(made.len(), 1, "{name}: {result}");
    (s, made[0].clone())
}

fn inspected(session: &mut Session) -> Vec<Value> {
    tried(session, "document.inspect", json!({})).unwrap()["bodies"]
        .as_array()
        .unwrap()
        .clone()
}

/// The first few words of an error: enough to tell which refusal it is.
fn brief(error: &str) -> String {
    error.chars().take(110).collect()
}

/// One line of what the commands did with a body, and whether it is what
/// they must do: `facts` is printed whole before anything is held, so
/// that a run on a build without the graded grid shows every line.
struct Facts {
    lines: Vec<String>,
    wrong: Vec<String>,
}

impl Facts {
    fn hold(&mut self, what: String, said: String, right: bool) {
        self.lines.push(format!(
            "weighted {what}: {said}{}",
            if right { "" } else { "   <-- NOT AS HELD" }
        ));
        if !right {
            self.wrong.push(format!("{what}: {said}"));
        }
    }
}

/// The body `name`, read from its file: its status, its counts against
/// the body written, a picture, its STL and 3MF, and its STEP read back.
/// `folded` is how many triangles of its mesh the STL may fold to a line
/// in single precision: `Some(0)` for none, `None` where the count is
/// only printed and the file must say so.
fn through_the_commands(
    dir: &Path,
    name: &str,
    body: &Body,
    volume: Option<f64>,
    facts: &mut Facts,
) -> Session {
    let (mut s, made) = imported(dir, name);
    facts.hold(
        format!("{name}, file.import"),
        format!("status {}", made["status"]),
        made["status"] == "ok",
    );
    let bodies = inspected(&mut s);
    let found = &bodies[0];
    let counts = |v: &Value| {
        (
            v["faces"].as_u64(),
            v["edges"].as_u64(),
            v["vertices"].as_u64(),
        )
    };
    let expected = (
        Some(body.faces.len() as u64),
        Some(body.edges.len() as u64),
        Some(body.vertices.len() as u64),
    );
    facts.hold(
        format!("{name}, document.inspect"),
        format!(
            "faces, edges, vertices {:?}; volume {}",
            counts(found),
            found["volume"]
        ),
        counts(found) == expected
            && match volume {
                // Held to the closed form where the measure gives one.
                Some(volume) => found["volume"]
                    .as_f64()
                    .is_some_and(|v| (v - volume).abs() <= 1e-9 * volume),
                None => true,
            },
    );

    // A picture: drawn at the picture's own tolerance, not coarsened, and
    // something of the body in it.
    let picture = tried(
        &mut s,
        "view.render",
        json!({"path": format!("{name}.png"), "width": 640, "height": 480, "renderer": "cpu"}),
    );
    match &picture {
        Ok(out) => facts.hold(
            format!("{name}, view.render"),
            format!(
                "coverage {:.3}, tolerance {}, coarsened {}",
                out["coverage"].as_f64().unwrap_or(0.0),
                out["tolerance"],
                out["coarsened"]
            ),
            out["coverage"].as_f64().is_some_and(|c| c > 0.02) && out["coarsened"] == false,
        ),
        Err(e) => facts.hold(format!("{name}, view.render"), brief(e), false),
    }

    // STL and 3MF, at the automatic tolerance and at the fine quality:
    // the body written, its mesh closed as `file.export` counts it.
    for format in ["stl", "3mf"] {
        for quality in ["normal", "fine"] {
            let out = tried(
                &mut s,
                "file.export",
                json!({"path": format!("{name}-{quality}.{format}"), "quality": quality}),
            );
            let what = format!("{name}, file.export {format} {quality}");
            match &out {
                Ok(out) => facts.hold(
                    what,
                    format!(
                        "bodies {}, closed {}, triangles {}, foldedTriangles {}, mergedPoints {}, warnings {}",
                        out["bodies"],
                        out["closed"],
                        out["triangles"],
                        out["foldedTriangles"],
                        out["mergedPoints"],
                        out["warnings"]
                    ),
                    out["bodies"] == 1 && out["closed"] == 1,
                ),
                Err(e) => facts.hold(what, brief(e), false),
            }
        }
    }

    // STEP, and the file read back: the same body.
    let back = format!("{name}-back");
    match tried(
        &mut s,
        "file.export",
        json!({"path": format!("{back}.step")}),
    ) {
        Ok(_) => {
            let (mut again, made) = imported(dir, &back);
            let read = inspected(&mut again);
            facts.hold(
                format!("{name}, STEP written and read back"),
                format!(
                    "status {}, faces, edges, vertices {:?}, volume {}",
                    made["status"],
                    counts(&read[0]),
                    read[0]["volume"]
                ),
                made["status"] == "ok"
                    && counts(&read[0]) == expected
                    && read[0]["volume"] == found["volume"],
            );
        }
        Err(e) => facts.hold(format!("{name}, STEP written"), brief(&e), false),
    }
    s
}

#[test]
fn bodies_drawn_unevenly_are_read_drawn_exported_and_cut() {
    let dir = workdir("weighted", "import");
    let mut facts = Facts {
        lines: Vec::new(),
        wrong: Vec::new(),
    };

    let block = seeded_block(Vec3::new(30.0, 900.0, 20.0), 1);
    let (plain, twin) = quarter_rod();
    let rod = skewed_apart(&twin, 1e4, 1e4);
    let milder = skewed_apart(&twin, 1e3, 1e3);
    let (tool_body, tool_params) = tool();
    let cut = boolean(&milder, &tool_body, BooleanOp::Subtract, &tolerance())
        .unwrap()
        .body;
    let rod_volume = PI * 4.0 * 3.0 / 4.0;
    let cut_volume = mass_properties(
        &boolean(&plain, &tool_body, BooleanOp::Subtract, &tolerance())
            .unwrap()
            .body,
    )
    .unwrap()
    .volume;
    for (name, body) in [
        ("block", &block),
        ("rod", &rod),
        ("milder", &milder),
        ("cut", &cut),
    ] {
        written(&dir, name, body);
    }

    // The block: not measured yet (its weights are further apart than the
    // measure's own parameters take), so its volume is not held here.
    through_the_commands(&dir, "block", &block, None, &mut facts);
    through_the_commands(&dir, "rod", &rod, Some(rod_volume), &mut facts);
    let mut with_rod = through_the_commands(&dir, "milder", &milder, Some(rod_volume), &mut facts);
    let mut with_cut = through_the_commands(&dir, "cut", &cut, Some(cut_volume), &mut facts);

    // A boolean by the commands, with the rod drawn a thousand apart: the
    // block through its curved side taken out. Its result is looked at
    // for passing through itself on a mesh, which could not be made
    // before; held to the same cut of the analytic rod, measured.
    tried(&mut with_rod, "feature.block", tool_params).unwrap();
    let target = inspected(&mut with_rod)[0]["name"]
        .as_str()
        .unwrap()
        .to_string();
    let what = "milder, feature.boolean subtract".to_string();
    match tried(
        &mut with_rod,
        "feature.boolean",
        json!({"op": "subtract", "target": target, "tool": "Tool", "name": "Less"}),
    ) {
        Ok(out) => {
            let less = inspected(&mut with_rod)
                .into_iter()
                .find(|b| b["name"] == "Less")
                .unwrap_or(Value::Null);
            let right = out["status"] == "ok"
                && less["volume"]
                    .as_f64()
                    .is_some_and(|v| (v - cut_volume).abs() <= 1e-6 * cut_volume);
            let said = if right {
                format!(
                    "status {}, volume {} for {cut_volume}",
                    out["status"], less["volume"]
                )
            } else {
                brief(&out.to_string())
            };
            facts.hold(what, said, right);
        }
        Err(e) => facts.hold(what, brief(&e), false),
    }
    let what = "milder less the block, document.check deep".to_string();
    match tried(&mut with_rod, "document.check", json!({"deep": true})) {
        Ok(out) => facts.hold(what, format!("ok {}", out["ok"]), out["ok"] == true),
        Err(e) => facts.hold(what, brief(&e), false),
    }

    // The cut's curves carry knots a billionth of the size apart, the
    // tessellator takes every knot of a spline edge for a point of it,
    // and the triangles between such points fold to lines in the single
    // precision of a binary STL. `file.export` says so: the file is a
    // closed surface with those triangles left out, counted, with a
    // warning; a 3MF, which writes every point as the mesh has it, has
    // none.
    let what = "cut, the binary STL's folded triangles".to_string();
    match tried(
        &mut with_cut,
        "file.export",
        json!({"path": "cut-pinned.stl", "quality": "normal"}),
    ) {
        Ok(out) => {
            let warnings = out["warnings"].to_string();
            facts.hold(
                what,
                format!(
                    "closed {}, foldedTriangles {}, mergedPoints {}",
                    out["closed"], out["foldedTriangles"], out["mergedPoints"]
                ),
                out["closed"] == 1
                    && out["foldedTriangles"] == 46
                    && out["mergedPoints"] == 23
                    && warnings
                        .contains("the file is a closed surface with 46 triangles of no area"),
            );
        }
        Err(e) => facts.hold(what, brief(&e), false),
    }
    let what = "cut, the 3MF".to_string();
    match tried(
        &mut with_cut,
        "file.export",
        json!({"path": "cut-pinned.3mf", "quality": "normal"}),
    ) {
        Ok(out) => facts.hold(
            what,
            format!(
                "closed {}, foldedTriangles {}, warnings {}",
                out["closed"], out["foldedTriangles"], out["warnings"]
            ),
            out["closed"] == 1 && out["foldedTriangles"].is_null() && out["warnings"].is_null(),
        ),
        Err(e) => facts.hold(what, brief(&e), false),
    }

    for line in &facts.lines {
        println!("{line}");
    }
    assert!(facts.wrong.is_empty(), "{:#?}", facts.wrong);
}
