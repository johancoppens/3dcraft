//! Booleans the showcase models found refused (2026-10-15), each from the
//! steps that made it, with what the result has to satisfy.
//!
//! - The lamp: a shell of revolution with thin walls less a union of ten
//!   lofted cells was refused, "an intersection curve could not be fitted
//!   within the tolerance", though every nine of them and each alone
//!   passed. Every face pair is intersected inside the box both bodies
//!   share, so the union decided the box; the box cut a branch of the
//!   curve of the shell and one cell, and the span to the new end was a
//!   new cubic nobody held to the tolerance. The marcher now refines the
//!   pieces a box cuts.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use craft3d_geom::intersect::{SurfaceMeeting, meet_surfaces};
use craft3d_kernel::boolean::{BooleanOp, boolean};
use craft3d_math::{Bbox3, Transform, Vec3};
use craft3d_topo::{Body, mass_properties};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;

fn run(session: &mut Session, id: &str, params: &Value) -> Value {
    session
        .execute(id, params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// A session after the steps of `file` (a JSON array of `[id, params]`).
fn steps(file: &str) -> Session {
    let text = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/boolean_findings")
            .join(file),
    )
    .unwrap();
    let steps: Vec<(String, Value)> = serde_json::from_str(&text).unwrap();
    let mut s = Session::new(Workspace::unrestricted());
    run(&mut s, "file.new", &json!({"units": "mm"}));
    for (id, params) in &steps {
        run(&mut s, id, params);
    }
    s
}

fn status(session: &mut Session, name: &str) -> Value {
    let document = run(session, "document.inspect", &json!({}));
    document["features"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == name)
        .unwrap_or_else(|| panic!("no feature {name}: {document}"))
        .clone()
}

fn assert_ok(session: &mut Session, name: &str) {
    let feature = status(session, name);
    assert_eq!(feature["status"], "ok", "{feature}");
    let check = run(session, "document.check", &json!({"deep": true}));
    assert_eq!(check["ok"], true, "{check}");
}

/// The bodies of the session's features by name (those taken by later
/// features too), regenerated from the
/// document saved to a file of its own, and the document's tolerance.
fn bodies(
    session: &mut Session,
    file: &str,
) -> (Vec<(String, Arc<Body>)>, craft3d_math::Tolerance) {
    let dir = std::env::temp_dir().join(format!("craft3d-boolean-findings-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(file);
    run(
        session,
        "file.save",
        &json!({"path": path.to_str().unwrap()}),
    );
    let doc = craft3d_format::load(&path).unwrap();
    let _ = std::fs::remove_file(&path);
    let regen = craft3d_regen::regenerate(&doc);
    let bodies = doc
        .features
        .iter()
        .filter_map(|f| {
            // Operands too: a body taken by a later feature stays here.
            let body = regen.feature(f.id)?.body.as_ref()?;
            Some((f.name.clone(), body.clone()))
        })
        .collect();
    let tolerance = craft3d_math::Tolerance {
        linear: doc.tolerance.linear,
        angular: doc.tolerance.angular,
    };
    (bodies, tolerance)
}

fn body<'a>(bodies: &'a [(String, Arc<Body>)], name: &str) -> &'a Body {
    &bodies.iter().find(|(n, _)| n == name).unwrap().1
}

fn volume(body: &Body) -> f64 {
    mass_properties(body).unwrap().volume
}

#[test]
fn the_lamp_shell_less_a_lofted_cell() {
    let mut s = steps("lamp_cell.steps.json");
    let cut = run(
        &mut s,
        "feature.boolean",
        &json!({"op": "subtract", "target": "Shell", "tool": "Cell66", "keepTool": true, "name": "Cut"}),
    );
    assert_ne!(cut["status"], "failed", "{cut}");
    assert_ok(&mut s, "Cut");
    let (bodies, tol) = bodies(&mut s, "lamp.3dcraft");
    let (shell, cell) = (body(&bodies, "Shell"), body(&bodies, "Cell66"));

    // The intersection of the shell's outer wall and a side of the cell
    // in the box the ten cells of the lamp gave it: the box cuts a branch
    // of their curve, and every curve is within the tolerance of both
    // surfaces (it was "could not be fitted").
    let within = Bbox3::from_points([
        Vec3::new(-168.04316601773542, -168.04316601773542, 104.38877107830017),
        Vec3::new(167.06945264388725, 168.0431660177354, 198.75929163002164),
    ]);
    let (wall, side) = (&shell.faces[1].surface, &cell.faces[2].surface);
    let met = meet_surfaces(wall, side, &within, &tol).unwrap();
    let SurfaceMeeting::Meets(meeting) = met else {
        panic!("the wall and the side meet: {met:?}");
    };
    assert!(!meeting.curves.is_empty());
    for curve in &meeting.curves {
        assert!(curve.deviation <= tol.linear, "{}", curve.deviation);
        let (t0, t1) = curve.range;
        for k in 0..=400 {
            let p = curve.curve.eval(t0 + (t1 - t0) * f64::from(k) / 400.0);
            for surface in [wall, side] {
                let (u, v) = surface.project(p).unwrap();
                let off = surface.eval(u, v).distance(p);
                assert!(off <= tol.linear, "{off} off a surface at {p:?}");
            }
        }
    }

    // The identities of the three booleans, in place and turned.
    let turn =
        Transform::rotation_about(Vec3::new(3.0, -7.0, 11.0), Vec3::new(0.3, -0.5, 0.8), 0.7)
            .unwrap();
    for t in [None, Some(turn)] {
        let place = |b: &Body| t.map_or_else(|| b.clone(), |t| b.transformed(&t).unwrap());
        let (a, b) = (place(shell), place(cell));
        let (va, vb) = (volume(&a), volume(&b));
        let of = |op| volume(&boolean(&a, &b, op, &tol).unwrap().body);
        let (rest, common, all) = (
            of(BooleanOp::Subtract),
            of(BooleanOp::Intersect),
            of(BooleanOp::Union),
        );
        assert!(
            common > 1e-3 * vb,
            "the cell goes through the wall: {common}"
        );
        assert!(
            (rest + common - va).abs() <= 1e-8 * va,
            "{rest} + {common} != {va}"
        );
        assert!(
            (all + common - va - vb).abs() <= 1e-8 * va,
            "{all} + {common} != {va} + {vb}"
        );
    }
}
