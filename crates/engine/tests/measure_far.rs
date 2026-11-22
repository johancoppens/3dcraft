//! `measure.below` and `measure.section` on a section small against its
//! distance from the origin (fault M, fuzz seed 138822; 2026-11-22;
//! docs/design/sections-and-levels.md).
//!
//! The seed's body, reduced: a torus R 15, r 2.5 about z, cut by the
//! planes x − y = 0.01 and y = −12.378, leaves a sliver at the tube's
//! outer equator 17.5 from the torus's axis, some 0.14 long and 0.0014
//! thick; cut by the plane through (−12.368, −12.378, 0) with the normal
//! (−1, 0, 1), moved by 41 offsets from −2e-3 to 2e-3. Its volume below
//! and its section's area by quadrature of the exact region (independent
//! of the kernel: `plan/measure-far/sliver.py` in the workshop), at the
//! origin and moved by 100 and by 1e4 along (1, 1, 0).
//!
//! And today's answers of the commands kept to the bit: the sea kayak's
//! hull (below at three levels, its level, its floating equilibrium and
//! its section), the notched block of `plane_measures` at its floor
//! (section with its torsion, level, floating) and the sliver at the
//! origin where it was measured before (a hash of every number each
//! command gives).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use serde_json::{Value, json};
use std::path::PathBuf;

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// Every number in `value`, in order (objects by their keys' order);
/// `null` as NaN, a flag as 0 or 1, text left out.
fn numbers(value: &Value, out: &mut Vec<f64>) {
    match value {
        Value::Null => out.push(f64::NAN),
        Value::Bool(b) => out.push(f64::from(u8::from(*b))),
        Value::Number(n) => out.push(n.as_f64().unwrap_or(f64::NAN)),
        Value::String(_) => {}
        Value::Array(items) => items.iter().for_each(|v| numbers(v, out)),
        Value::Object(map) => map.values().for_each(|v| numbers(v, out)),
    }
}

/// FNV-1a over the bits of every number in `value`.
fn hash(value: &Value) -> u64 {
    let mut all = Vec::new();
    numbers(value, &mut all);
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for v in all {
        for byte in v.to_bits().to_le_bytes() {
            h ^= u64::from(byte);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    h
}

// ------------------------------------------------------------ the sliver

const TOLERANCE: f64 = 1e-6;

/// The offset, the volume below and the section's area of the sliver, by
/// quadrature of the exact region (30-node Gauss–Legendre over pieces
/// split where the plane meets the tube, the tube's square-root edge taken
/// out by a change of variable; converged to 1e-20 in the volume and
/// 1e-15 in the area at 2e-4, doubling the pieces and taking 40 nodes).
const QUADRATURE: [(f64, f64, f64); 41] = [
    (-0.002, 9.029548358801467e-08, 2.6331770425651205e-06),
    (-0.0019, 9.055883848834164e-08, 2.6339173569438185e-06),
    (-0.0018, 9.082226634013184e-08, 2.634636069395202e-06),
    (-0.0017, 9.108576498227963e-08, 2.6353331662828305e-06),
    (-0.0016, 9.134933225239519e-08, 2.6360086333925868e-06),
    (-0.0015, 9.161296598688917e-08, 2.636662456467251e-06),
    (-0.0014, 9.187666402104414e-08, 2.6372946216572006e-06),
    (-0.0013, 9.214042418906143e-08, 2.6379051187596775e-06),
    (-0.0012, 9.24042443235473e-08, 2.6384939524711217e-06),
    (-0.0011, 9.266812225613552e-08, 2.639061091125981e-06),
    (-0.001, 9.293205581776289e-08, 2.639606523346425e-06),
    (-0.0009, 9.319604283800687e-08, 2.640130256803472e-06),
    (-0.0008, 9.346008114495911e-08, 2.6406322672338413e-06),
    (-0.0007, 9.372416856651224e-08, 2.641112539541304e-06),
    (-0.0006, 9.398830292907508e-08, 2.6415710835692176e-06),
    (-0.0005, 9.425248205788978e-08, 2.642007871883621e-06),
    (-0.0004, 9.451670377794925e-08, 2.6424228990628232e-06),
    (-0.0003, 9.478096591252281e-08, 2.6428161645175245e-06),
    (-0.0002, 9.504526628454261e-08, 2.643187646676901e-06),
    (-0.0001, 9.530960271598358e-08, 2.6435373486646826e-06),
    (0.0, 9.557397302767265e-08, 2.6438652546140372e-06),
    (0.0001, 9.583837504011483e-08, 2.6441713587399228e-06),
    (0.0002, 9.610280657254518e-08, 2.6444556553763164e-06),
    (0.0003, 9.636726544382074e-08, 2.6447181327526353e-06),
    (0.0004, 9.663174947184828e-08, 2.6449587893008233e-06),
    (0.0005, 9.68962564739059e-08, 2.645177613172495e-06),
    (0.0006, 9.716078426661151e-08, 2.645374600568097e-06),
    (0.0007, 9.742533066587425e-08, 2.6455497438006346e-06),
    (0.0008, 9.768989348704178e-08, 2.6457030373461415e-06),
    (0.0009, 9.795447054482176e-08, 2.645834475211491e-06),
    (0.001, 9.821905965337562e-08, 2.6459440518812583e-06),
    (0.0011, 9.848365862631314e-08, 2.6460317620120643e-06),
    (0.0012, 9.874826527673563e-08, 2.6460976007447373e-06),
    (0.0013, 9.901287741723812e-08, 2.6461415629807275e-06),
    (0.0014, 9.927749285999395e-08, 2.6461636447796824e-06),
    (0.0015, 9.954210941666755e-08, 2.646163841436629e-06),
    (0.0016, 9.980672489863583e-08, 2.6461421489481064e-06),
    (0.0017, 1.0007133711672755e-07, 2.646098564768249e-06),
    (0.0018, 1.0033594388160745e-07, 2.646033082264393e-06),
    (0.0019, 1.0060054300345497e-07, 2.6459457044198494e-06),
    (0.002, 1.008651322921919e-07, 2.6458364206215525e-06),
];

/// How far the sliver is moved along (1, 1, 0): at the origin it is 17.5
/// from it, moved by 100 some 125, by 1e4 some 1.4e4.
const SHIFTS: [f64; 3] = [0.0, 100.0, 1e4];

/// The reduced seed 138822, moved by `shift` along x and along y.
fn sliver(shift: f64) -> Session {
    let mut s = Session::new(Workspace::none());
    run(
        &mut s,
        "file.new",
        json!({"units": "mm", "tolerance": TOLERANCE}),
    );
    for (command, params) in [
        (
            "feature.torus",
            json!({"axis": [0, 0, 1], "major": 15.0, "minor": 2.5, "name": "N0", "position": [shift, shift, 0.0]}),
        ),
        (
            "feature.planeCut",
            json!({"name": "N1", "target": "N0", "planes": [{"normal": [1.0, -1.0, 0.0], "point": [shift + 0.01, shift, 0.0], "side": "below"}]}),
        ),
        (
            "feature.planeCut",
            json!({"name": "N4", "target": "N1", "planes": [{"normal": [0.0, 1.0, 0.0], "point": [shift, shift - 12.378, 0.0], "side": "below"}]}),
        ),
    ] {
        let made = run(&mut s, command, params.clone());
        assert_eq!(made["status"], "ok", "{command} {params}: {made}");
    }
    s
}

fn sliver_plane(shift: f64) -> Value {
    json!({"point": [shift - 12.368, shift - 12.378, 0.0], "normal": [-1.0, 0.0, 1.0]})
}

/// `measure.below` of the sliver at each offset: the answer or the
/// refusal.
fn sliver_rows(shift: f64) -> Vec<(f64, Result<Value, String>)> {
    let mut s = sliver(shift);
    QUADRATURE
        .iter()
        .map(|&(offset, _, _)| {
            let found = s
                .execute(
                    "measure.below",
                    &json!({"of": "N4", "plane": sliver_plane(shift), "offset": offset}),
                )
                .map_err(|e| e.message().to_string());
            (offset, found)
        })
        .collect()
}

#[test]
fn sliver_table() {
    for shift in SHIFTS {
        let rows = sliver_rows(shift);
        for ((offset, found), (_, volume, area)) in rows.iter().zip(QUADRATURE) {
            match found {
                Ok(m) => {
                    let v = m["volume"].as_f64().unwrap();
                    let a = m["section"]["area"].as_f64().unwrap();
                    println!(
                        "SLIVER shift={shift:e} offset={offset} | answered | volume {v:e} off {:.2e} ({:.1e} of it) | area {a:e} off {:.2e} ({:.1e} of it) | surface {:e} | hash {:016x}",
                        v - volume,
                        (v - volume) / volume,
                        a - area,
                        (a - area) / area,
                        m["surface"].as_f64().unwrap(),
                        hash(m)
                    );
                }
                Err(e) => println!("SLIVER shift={shift:e} offset={offset} | REFUSED: {e}"),
            }
        }
        let refused = rows.iter().filter(|(_, f)| f.is_err()).count();
        println!(
            "SLIVER shift={shift:e}: {refused} of {} refused",
            rows.len()
        );
        // The section with its torsion left out, and the level for the
        // volume below at offset 0.
        let mut s = sliver(shift);
        let section = s.execute(
            "measure.section",
            &json!({"of": "N4", "plane": sliver_plane(shift), "offset": 0.0, "torsion": false}),
        );
        println!("SLIVER shift={shift:e} measure.section: {section:?}");
        let level = s.execute(
            "measure.levelFor",
            &json!({"of": "N4", "plane": sliver_plane(shift), "volume": QUADRATURE[20].1}),
        );
        println!("SLIVER shift={shift:e} measure.levelFor: {level:?}");
    }
}

/// The seed's own commands: `measure.below` by the plane the fuzzer asked
/// for, against what `feature.planeCut` keeps by that plane (the
/// fuzzer's `cut-below` oracle).
#[test]
fn the_seed_itself() {
    let mut s = Session::new(Workspace::none());
    run(
        &mut s,
        "file.new",
        json!({"units": "mm", "tolerance": TOLERANCE}),
    );
    let steps = [
        (
            "feature.torus",
            json!({"angle": "0 deg", "axis": [0.0, 0.0, 1.0], "major": 15.0, "minor": 2.5, "name": "N0", "position": [0.0, 0.0, 0.0]}),
        ),
        (
            "feature.planeCut",
            json!({"name": "N1", "planes": [{"normal": [1.0, -1.0, 0.0], "point": [0.009617199500024043, 0.0, 0.0], "side": "below"}], "target": "N0"}),
        ),
        ("feature.edit", json!({"feature": "N0", "minor": 2.5})),
        (
            "sketch.create",
            json!({"name": "S3", "offset": 0.0, "plane": "zx"}),
        ),
        (
            "sketch.polyline",
            json!({"closed": true, "points": [[0.0, 5.0], [2.5, 5.0], [2.5, 7.5], [0.0, 7.5]], "sketch": "S3"}),
        ),
        (
            "feature.revolve",
            json!({"angle": "360 deg", "axis": "y", "cut": "N1", "name": "N3", "sketch": "S3"}),
        ),
        (
            "feature.planeCut",
            json!({"name": "N4", "planes": {"offset": -12.377815446967205, "plane": "zx"}, "target": "N3"}),
        ),
    ];
    for (command, params) in steps {
        run(&mut s, command, params);
    }
    let plane = json!({"point": [-12.368198247467179, -12.378836115306143, 0.0778737124994808], "normal": [-3.0, 3.0, 3.0]});
    let below = s.execute(
        "measure.below",
        &json!({"of": "N4", "plane": plane, "offset": 0}),
    );
    println!("SEED measure.below: {below:?}");
    run(
        &mut s,
        "feature.planeCut",
        json!({"name": "N7", "planes": {"normal": [-3.0, 3.0, 3.0], "point": [-12.368198247467179, -12.378836115306143, 0.0778737124994808], "side": "below"}, "target": "N4"}),
    );
    let kept = run(&mut s, "document.measure", json!({"feature": "N7"}));
    println!("SEED N7: {kept}");
}

// ------------------------------------------------------ today's answers

fn kayak() -> Session {
    let text = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/kayak_findings/sea_kayak_9.steps.json"),
    )
    .unwrap();
    let steps: Vec<(String, Value)> = serde_json::from_str(&text).unwrap();
    let mut s = Session::new(Workspace::unrestricted());
    run(&mut s, "file.new", json!({"units": "mm"}));
    for (id, params) in &steps {
        s.execute(id, params).unwrap();
    }
    s
}

fn notched() -> Session {
    let mut s = Session::new(Workspace::none());
    run(
        &mut s,
        "file.new",
        json!({"units": "mm", "tolerance": 1e-6}),
    );
    for (command, params) in [
        (
            "feature.block",
            json!({"size": [20, 10, 20], "name": "Big"}),
        ),
        (
            "feature.block",
            json!({"size": [11, 12, 11], "position": [10, -1, 10], "name": "Notch"}),
        ),
        (
            "feature.boolean",
            json!({"op": "subtract", "target": "Big", "tool": "Notch", "name": "Notched"}),
        ),
        (
            "feature.block",
            json!({"size": [2000, 600, 400], "position": [-1000, -300, 0], "name": "Barge"}),
        ),
    ] {
        let made = run(&mut s, command, params.clone());
        assert_eq!(made["status"], "ok", "{command} {params}: {made}");
    }
    s
}

/// Each command's name and the hash of its answer (or its refusal).
fn todays_answers() -> Vec<(String, Result<u64, String>)> {
    let mut out = Vec::new();
    let mut keep =
        |name: String, found: Result<Value, String>| out.push((name, found.map(|v| hash(&v))));
    let mut s = kayak();
    for offset in ["30.2 mm", "30.28 mm", "30.3 mm"] {
        let found = s
            .execute("measure.below", &json!({"of": "Hull", "offset": offset}))
            .map_err(|e| e.message().to_string());
        keep(format!("kayak below {offset}"), found);
    }
    let found = s
        .execute(
            "measure.section",
            &json!({"of": "Hull", "offset": "30.28 mm", "torsion": false}),
        )
        .map_err(|e| e.message().to_string());
    keep("kayak section 30.28 mm".into(), found);
    let level = run(
        &mut s,
        "measure.levelFor",
        json!({"of": "Hull", "volume": "98865474"}),
    );
    keep("kayak level".into(), Ok(level.clone()));
    let b = level["centroid"].clone();
    let found = s
        .execute(
            "measure.floating",
            &json!({"of": "Hull", "volume": "98865474", "centerOfGravity": b, "heel": 4, "trim": 1}),
        )
        .map_err(|e| e.message().to_string());
    keep("kayak floating".into(), found);
    let mut s = notched();
    let floor = json!({"point": [0.0, 0.0, 10.0], "normal": [0.0, 0.0, 1.0]});
    for (name, command, params) in [
        (
            "notched section",
            "measure.section",
            json!({"of": "Notched", "plane": floor}),
        ),
        (
            "notched below",
            "measure.below",
            json!({"of": "Notched", "offset": 7.5}),
        ),
        (
            "notched level",
            "measure.levelFor",
            json!({"of": "Notched", "volume": 2000}),
        ),
        (
            "notched floating",
            "measure.floating",
            json!({"of": "Notched", "volume": 2000, "centerOfGravity": [10, 5, 4]}),
        ),
        (
            "barge floating",
            "measure.floating",
            json!({"of": "Barge", "volume": 2000.0 * 600.0 * 250.0, "centerOfGravity": [0, 0, 300]}),
        ),
        (
            "barge floating trimmed",
            "measure.floating",
            json!({"of": "Barge", "volume": 2000.0 * 600.0 * 250.0, "centerOfGravity": [20, 0, 200]}),
        ),
    ] {
        let found = s
            .execute(command, &params)
            .map_err(|e| e.message().to_string());
        keep(name.into(), found);
    }
    for (offset, found) in sliver_rows(0.0) {
        keep(format!("sliver {offset}"), found);
    }
    out
}

#[test]
fn todays_answers_table() {
    for (name, found) in todays_answers() {
        match found {
            Ok(h) => println!("PIN (\"{name}\", 0x{h:016x}),"),
            Err(e) => println!("REFUSED {name}: {e}"),
        }
    }
}
