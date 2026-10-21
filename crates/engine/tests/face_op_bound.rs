//! The work bound of a face operation (`craft3d_kernel::offset_face::
//! FACE_OP_WORK`, `craft3d_geom::allowance`) gives the same verdict on the
//! same geometry turned: the start cap of each of the rotor's three blades
//! (the showcase's wind turbine, `bench_models/rotor.steps.json`, a
//! circular pattern) moved out 0.17 mm. The blades are one geometry turned
//! about the hub's axis, but the work is not the same (82, 275 and 293
//! million units: the classification casts its rays along fixed axes and
//! splits the pieces of a surface at a slant to them finer); the bound
//! must let every copy through. A minute or two each in release:
//! `cargo test --release -p craft3d-engine --test face_op_bound -- --ignored`.

#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use craft3d_engine::{Session, Workspace};
use serde_json::{Value, json};
use std::path::PathBuf;

fn rotor() -> Session {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/bench_models/rotor.steps.json");
    let steps: Vec<(String, Value)> =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let mut s = Session::new(Workspace::unrestricted());
    s.execute("file.new", &json!({"units": "mm"})).unwrap();
    for (id, params) in &steps {
        s.execute(id, params)
            .unwrap_or_else(|e| panic!("{id}: {e}"));
    }
    s
}

#[test]
#[ignore = "a minute or two per blade in release"]
fn the_start_cap_of_every_blade_moves_alike() {
    use craft3d_geom::allowance;
    let mut verdicts = Vec::new();
    for cap in [
        "F13:copy(1)/F9:startCap(e5,e8)",
        "F13:copy(1,1)/F9:startCap(e5,e8)",
        "F13:copy(1,2)/F9:startCap(e5,e8)",
    ] {
        let mut s = rotor();
        let (made, work) = allowance::measured(|| {
            s.execute(
                "feature.offsetFace",
                &json!({"faces": [cap], "distance": "0.17 mm"}),
            )
            .unwrap()
        });
        eprintln!("{cap}: {work} units, {}", made["status"]);
        verdicts.push(made["status"].clone());
        assert_eq!(made["status"], "ok", "{cap}: {made}");
    }
    assert!(verdicts.windows(2).all(|w| w[0] == w[1]), "{verdicts:?}");
}
