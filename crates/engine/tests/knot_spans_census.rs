//! How many knot spans the spline faces of the project's models have, in
//! u and in v (review of 2026-11-02: the unfold lays a face of at most
//! 16384 spans in u and 1024 in v; this shows no model the tests,
//! showcases and fixtures build comes near them). Ignored: run with
//! `--ignored --nocapture`; it prints the largest and asserts the limits.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use craft3d_geom::Surface;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn files(dir: &Path, suffix: &str, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            files(&path, suffix, out);
        } else if path.to_string_lossy().ends_with(suffix) {
            out.push(path);
        }
    }
}

/// The largest inner-knot span counts `(u, v)` of the spline faces of the
/// bodies `doc` regenerates to.
fn spans(doc: &craft3d_doc::Document) -> (usize, usize) {
    let regen = craft3d_regen::regenerate(doc);
    let (mut most_u, mut most_v) = (0, 0);
    for (_, body) in regen.bodies() {
        for face in &body.faces {
            if let Surface::Nurbs(s) = &face.surface {
                let distinct = |knots: &[f64]| {
                    let mut k = knots.to_vec();
                    k.dedup();
                    k.len().saturating_sub(1)
                };
                most_u = most_u.max(distinct(s.knots_u()));
                most_v = most_v.max(distinct(s.knots_v()));
            }
        }
    }
    (most_u, most_v)
}

#[test]
#[ignore = "a census, run on the Strix"]
fn the_models_spline_faces_have_few_enough_knot_spans_to_unfold() {
    let scratch = std::env::temp_dir().join(format!("knot-census-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).unwrap();
    let mut documents = Vec::new();
    files(&root().join("docs/showcase"), ".3dcraft", &mut documents);
    files(
        &root().join("crates/format/tests/fixtures"),
        ".3dcraft",
        &mut documents,
    );
    let mut scripts = Vec::new();
    files(
        &root().join("crates/engine/tests"),
        ".steps.json",
        &mut scripts,
    );
    let (mut most_u, mut most_v) = ((0, String::new()), (0, String::new()));
    let mut note = |name: String, (u, v): (usize, usize)| {
        eprintln!("SPANS {name}: u {u}, v {v}");
        if u > most_u.0 {
            most_u = (u, name.clone());
        }
        if v > most_v.0 {
            most_v = (v, name);
        }
    };
    for path in &documents {
        match craft3d_format::load(path) {
            Ok(doc) => note(path.display().to_string(), spans(&doc)),
            Err(e) => eprintln!("SKIP {}: {e}", path.display()),
        }
    }
    for (n, path) in scripts.iter().enumerate() {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        let Ok(steps) = serde_json::from_str::<Vec<(String, Value)>>(&text) else {
            eprintln!("SKIP {}: not a list of steps", path.display());
            continue;
        };
        let mut s = Session::new(Workspace::unrestricted());
        s.execute("file.new", &json!({"units": "mm"})).unwrap();
        for (id, params) in &steps {
            let _ = s.execute(id, params);
        }
        let saved = scratch.join(format!("{n}.3dcraft"));
        if s.execute("file.save", &json!({"path": saved})).is_err() {
            eprintln!("SKIP {}: not saved", path.display());
            continue;
        }
        match craft3d_format::load(&saved) {
            Ok(doc) => note(path.display().to_string(), spans(&doc)),
            Err(e) => eprintln!("SKIP {}: {e}", path.display()),
        }
    }
    eprintln!(
        "MOST u {} ({}), v {} ({})",
        most_u.0, most_u.1, most_v.0, most_v.1
    );
    assert!(most_u.0 <= 1 << 14, "{most_u:?}");
    assert!(most_v.0 <= 1024, "{most_v:?}");
    let _ = std::fs::remove_dir_all(&scratch);
}
