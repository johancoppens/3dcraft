//! Opt-in: FreeCAD itself, headless, agrees with `tests/data/expected.json`
//! (what `make_fixtures.py` recorded when it made the documents), so the
//! numbers the other tests hold this crate to are FreeCAD's.
//!
//! ```sh
//! CRAFT3D_FREECAD=/path/to/FreeCADCmd cargo test -p craft3d-fcstdread --test freecad -- --ignored
//! ```
//!
//! Part of `cargo xtask freecad`. When opted in, a missing or failing
//! FreeCAD is a test failure, never a skip. FreeCAD runs as a separate
//! program, with a user home of its own; nothing of it is linked.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::PathBuf;
use std::process::Command;

#[test]
#[ignore = "needs CRAFT3D_FREECAD"]
fn freecad_measures_the_documents_as_expected_json_says() {
    let freecad = std::env::var("CRAFT3D_FREECAD")
        .expect("set CRAFT3D_FREECAD to a FreeCAD AppImage or FreeCADCmd executable");
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("fcstd-freecad");
    let home = dir.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let output = dir.join("verify.report");
    let _ = std::fs::remove_file(&output);
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/verify_fixtures.py");
    let mut command = Command::new(&freecad);
    if freecad.ends_with(".AppImage") {
        command.arg("freecadcmd");
    }
    let run = command
        .arg(&script)
        .env("CRAFT3D_FC_OUT", &output)
        .env("FREECAD_USER_HOME", &home)
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap_or_else(|e| panic!("cannot run {freecad}: {e}"));
    let text = std::fs::read_to_string(&output).unwrap_or_else(|_| {
        panic!(
            "FreeCAD wrote no report ({}):\n{}\n{}",
            run.status,
            String::from_utf8_lossy(&run.stdout),
            String::from_utf8_lossy(&run.stderr)
        )
    });
    let lines: Vec<&str> = text.lines().filter(|l| !l.is_empty()).collect();
    assert!(lines.len() >= 11, "{text}");
    assert!(lines.iter().all(|l| l.starts_with("ok ")), "{text}");
}

/// Runs a headless FreeCAD script with `CRAFT3D_FC_OUT` set to `out`.
fn freecad(script: &str, out: &std::path::Path) {
    let freecad = std::env::var("CRAFT3D_FREECAD")
        .expect("set CRAFT3D_FREECAD to a FreeCAD AppImage or FreeCADCmd executable");
    let home = out.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(script);
    let mut command = Command::new(&freecad);
    if freecad.ends_with(".AppImage") {
        command.arg("freecadcmd");
    }
    let run = command
        .arg(&script)
        .env("CRAFT3D_FC_OUT", out)
        .env("FREECAD_USER_HOME", &home)
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap_or_else(|e| panic!("cannot run {freecad}: {e}"));
    assert!(
        out.join("oracles.json").exists(),
        "FreeCAD wrote nothing ({}):\n{}\n{}",
        run.status,
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
}

#[test]
#[ignore = "needs CRAFT3D_FREECAD"]
fn periodic_splines_and_a_spindle_torus_read_as_freecad_makes_them() {
    use craft3d_fcstdread::{Options, parse};
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("fcstd-brep-oracles");
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    freecad("brep_oracles.py", &out);
    let oracles: serde_json::Value =
        serde_json::from_slice(&std::fs::read(out.join("oracles.json")).unwrap()).unwrap();
    assert!(oracles.get("error").is_none(), "{oracles}");
    let options = Options {
        metres_per_unit: 0.001,
        tolerance: craft3d_math::Tolerance::new(1e-6, 1e-9).unwrap(),
        loosest: 1e-3,
    };
    let mut curves = 0;
    for (name, row) in oracles.as_object().unwrap() {
        if !name.starts_with("periodic_") {
            continue;
        }
        curves += 1;
        let bytes = std::fs::read(out.join(format!("{name}.brp"))).unwrap();
        let read = parse(&bytes)
            .and_then(|p| p.build(0, &options))
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        let edge = read.body.edges.first().unwrap();
        for p in row["points"].as_array().unwrap() {
            let t = p[0].as_f64().unwrap();
            let q: Vec<f64> = p[1]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_f64().unwrap())
                .collect();
            let ours = edge.curve.eval(t);
            let d = ours.distance(craft3d_math::Vec3::new(q[0], q[1], q[2]));
            assert!(d < 1e-9, "{name} at {t}: {ours:?} against {q:?} ({d:e})");
        }
    }
    assert_eq!(curves, 7, "{oracles}");
    let apple = &oracles["apple.brp"];
    assert_eq!(apple["valid"], true, "{oracles}");
    let bytes = std::fs::read(out.join("apple.brp")).unwrap();
    let read = parse(&bytes)
        .and_then(|p| p.build(0, &options))
        .unwrap_or_else(|e| panic!("apple: {e}"));
    let mass = craft3d_topo::mass_properties(&read.body).unwrap();
    for (ours, theirs, what) in [
        (mass.volume, apple["volume"].as_f64().unwrap(), "volume"),
        (mass.area, apple["area"].as_f64().unwrap(), "area"),
    ] {
        assert!(
            (ours - theirs).abs() <= 1e-6 * theirs.abs(),
            "apple {what}: {ours} against FreeCAD's {theirs}"
        );
    }
}
