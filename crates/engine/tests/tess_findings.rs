//! Exports that failed because the tessellator could not mesh a face
//! (2026-10-21, `docs/design/tessellator.md` › *Three faces the
//! tessellator could not mesh*): the Perseverance's grouser, a band
//! between two arcs with its top arcs filleted 2.5 and turned about y,
//! whose STL and 3MF failed at chords of 4 to 6.2 ("its refinement
//! cannot go on"), and with it the whole rover's at its default
//! tolerance. The tessellator's own tests are in
//! `crates/testkit/tests/tess_faces.rs`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

fn run(session: &mut Session, id: &str, params: &Value) -> Value {
    session
        .execute(id, params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// A directory of its own for this test's files.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("craft3d-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Exports the bodies of `session` as STL and 3MF into `dir`, at
/// `tolerance` or the automatic one: every body written, every mesh
/// closed.
fn exported(session: &mut Session, dir: &Path, tolerance: Option<f64>, what: &str) {
    for format in ["stl", "3mf"] {
        let path = dir.join(format!("out.{format}"));
        let mut params = json!({"path": path.to_string_lossy()});
        if let Some(tolerance) = tolerance {
            params["tolerance"] = json!(tolerance);
        }
        let out = run(session, "file.export", &params);
        assert!(
            out["bodies"].as_u64().unwrap() > 0,
            "{what}, {format}: {out}"
        );
        assert_eq!(out["closed"], out["bodies"], "{what}, {format}: {out}");
        assert!(
            std::fs::metadata(&path).unwrap().len() > 0,
            "{what}, {format}"
        );
    }
}

#[test]
fn the_turned_grouser_exports_at_coarse_tolerances() {
    let text = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/tess_findings/turned_grouser.steps.json"),
    )
    .unwrap();
    let steps: Vec<(String, Value)> = serde_json::from_str(&text).unwrap();
    let dir = scratch("turned-grouser");
    for angle in ["7.5 deg", "30 deg", "-7.5 deg", "0 deg"] {
        let mut s = Session::new(Workspace::unrestricted());
        for (id, params) in &steps {
            let mut params = params.clone();
            if id == "feature.transform" {
                params["angle"] = json!(angle);
            }
            run(&mut s, id, &params);
        }
        for tolerance in [Some(4.0), Some(6.2), None] {
            exported(
                &mut s,
                &dir,
                tolerance,
                &format!("turned {angle}, {tolerance:?}"),
            );
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// The whole Perseverance (55 bodies, 2026-10-21) at its default
/// tolerance, as `craft3d-cli convert` writes it. Ignored: the model is
/// a showcase file outside the repository, read from `CRAFT3D_ROVER` or
/// from `target/work/perseverance/perseverance.3dcraft`; the test says so
/// and passes when it is not there. Run it with
/// `cargo test --release -p craft3d-engine --test tess_findings -- --ignored`.
#[test]
#[ignore]
fn the_perseverance_exports_at_its_default_tolerance() {
    let path = std::env::var_os("CRAFT3D_ROVER").map_or_else(
        || {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/work/perseverance/perseverance.3dcraft")
        },
        PathBuf::from,
    );
    if !path.exists() {
        eprintln!("no rover at {}: nothing to export", path.display());
        return;
    }
    let mut s = Session::new(Workspace::unrestricted());
    run(
        &mut s,
        "file.open",
        &json!({"path": path.to_string_lossy()}),
    );
    let dir = scratch("perseverance");
    exported(&mut s, &dir, None, "the Perseverance");
    let _ = std::fs::remove_dir_all(&dir);
}
