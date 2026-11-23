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

/// Whether `gap` is a number no larger than `allowed` in size.
fn within(gap: f64, allowed: f64) -> bool {
    gap.abs() <= allowed
}

/// The sliver's section reaches 0.0014 along y and 0.0027 along x (and
/// √2 times that across the slanted plane): its perimeter is less than
/// this.
const SLIVER_PERIMETER: f64 = 0.01;

/// The sliver at the origin, moved by 100 and by 1e4: at each of the 41
/// offsets `measure.below` answers, its volume within the tolerance times
/// the surface below of the exact region's and its section's area within
/// the tolerance times the perimeter (what the module says of a section
/// bounded by curves where a plane meets a freeform or a torus face: the
/// B-rep is held to the tolerance; the errors found are printed, some
/// 3e-5 of the volume and 1e-6 of the area); `measure.section` gives the
/// same area as `measure.below` there, to the bit; and `measure.levelFor`
/// of the volume below at offset 0 lands within that volume's allowance
/// over the area of 0. Before 2026-11-22 the sliver at the origin was
/// refused at some offsets, and moved by 100 or 1e4 at every one: "the
/// section of body 0: its edges take more panels than a section may".
#[test]
fn the_sliver_is_measured_wherever_it_lies() {
    let mut wrong = Vec::new();
    for shift in SHIFTS {
        let rows = sliver_rows(shift);
        let mut at_zero = None;
        for ((offset, found), (_, volume, area)) in rows.iter().zip(QUADRATURE) {
            let m = match found {
                Ok(m) => m,
                Err(e) => {
                    println!("SLIVER shift={shift:e} offset={offset} | REFUSED: {e}");
                    wrong.push(format!("shift {shift}, offset {offset}: refused: {e}"));
                    continue;
                }
            };
            let v = m["volume"].as_f64().unwrap();
            let a = m["section"]["area"].as_f64().unwrap();
            let surface = m["surface"].as_f64().unwrap();
            println!(
                "SLIVER shift={shift:e} offset={offset} | answered | volume {v:e} off {:.2e} ({:.1e} of it) | area {a:e} off {:.2e} ({:.1e} of it) | surface {surface:e} | hash {:016x}",
                v - volume,
                (v - volume) / volume,
                a - area,
                (a - area) / area,
                hash(m)
            );
            if *offset == 0.0 {
                at_zero = Some((v, a, surface));
            }
            if !within(v - volume, TOLERANCE * surface) {
                wrong.push(format!(
                    "shift {shift}, offset {offset}: volume {v:e}, want {volume:e}"
                ));
            }
            if !within(a - area, TOLERANCE * SLIVER_PERIMETER) {
                wrong.push(format!(
                    "shift {shift}, offset {offset}: area {a:e}, want {area:e}"
                ));
            }
        }
        let Some((_, area_below, surface)) = at_zero else {
            continue;
        };
        let mut s = sliver(shift);
        let plane = sliver_plane(shift);
        match s.execute(
            "measure.section",
            &json!({"of": "N4", "plane": plane, "offset": 0.0, "torsion": false}),
        ) {
            Ok(section) => {
                let a = section["area"].as_f64().unwrap();
                println!("SLIVER shift={shift:e} measure.section area {a:e}");
                if a.to_bits() != area_below.to_bits() {
                    wrong.push(format!(
                        "shift {shift}: measure.section's area {a:e} is not measure.below's {area_below:e}"
                    ));
                }
            }
            Err(e) => wrong.push(format!("shift {shift}: measure.section refused: {e}")),
        }
        let volume = QUADRATURE[20].1;
        match s.execute(
            "measure.levelFor",
            &json!({"of": "N4", "plane": plane, "volume": volume}),
        ) {
            Ok(level) => {
                let offset = level["offset"].as_f64().unwrap();
                println!("SLIVER shift={shift:e} measure.levelFor offset {offset:e}");
                let allowed = TOLERANCE * surface / area_below;
                if !within(offset, allowed) {
                    wrong.push(format!(
                        "shift {shift}: the level for {volume:e} is {offset:e}, want 0 within {allowed:e}"
                    ));
                }
            }
            Err(e) => wrong.push(format!("shift {shift}: measure.levelFor refused: {e}")),
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// The seed's own commands: `measure.below` by the plane the fuzzer asked
/// for, refused before 2026-11-22, against what `feature.planeCut` keeps
/// by that plane, within the fuzzer's `cut-below` band.
#[test]
fn the_seed_is_measured_as_its_cut_keeps() {
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
    let below = run(
        &mut s,
        "measure.below",
        json!({"of": "N4", "plane": plane, "offset": 0}),
    );
    println!("SEED measure.below: {below}");
    let whole = run(&mut s, "document.measure", json!({"feature": "N4"}));
    run(
        &mut s,
        "feature.planeCut",
        json!({"name": "N7", "planes": {"normal": [-3.0, 3.0, 3.0], "point": [-12.368198247467179, -12.378836115306143, 0.0778737124994808], "side": "below"}, "target": "N4"}),
    );
    let kept = run(&mut s, "document.measure", json!({"feature": "N7"}));
    println!("SEED N4: {whole}; N7: {kept}");
    let number = |v: &Value| v.as_f64().unwrap();
    let (v, area) = (number(&whole["volume"]), number(&whole["area"]));
    let section = number(&below["section"]["area"]);
    let band = 4.0 * TOLERANCE * (area + section) + 2e-9 * v;
    let gap = number(&kept["volume"]) - number(&below["volume"]);
    assert!(
        gap.abs() <= band,
        "the cut keeps {kept}, measure.below gives {below}: off by {gap:e}, allowed {band:e}"
    );
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

/// What each command of [`todays_answers`] gave on `main` before the
/// change (5305636e, printed by the Check run 45024256884 on the probe
/// commit 31d39767, which is that commit and these tests alone): the hash
/// of its answer. One not listed was refused there.
const PINS: &[(&str, u64)] = &[
    ("kayak below 30.2 mm", 0xc497f4d03fe4d877),
    ("kayak below 30.28 mm", 0x480d69489ed1c0ad),
    ("kayak below 30.3 mm", 0xa420b4ba6135dc29),
    ("kayak section 30.28 mm", 0xd46292a7a4df5c60),
    ("kayak level", 0xd99450e973503dda),
    ("kayak floating", 0x90beca73ecd4f5a3),
    ("notched section", 0x7caaa6ffd54e0ff9),
    ("notched below", 0x48541ab7dcb1bb72),
    ("notched level", 0x299c1e477bfbc74a),
    ("notched floating", 0x92e1aacddaad957d),
    ("barge floating", 0xc2a58231d5e62918),
    ("barge floating trimmed", 0xbc6715354b9c081b),
    ("sliver -0.002", 0x1ceddf85f7b14465),
    ("sliver -0.0018", 0x8ce1ce4b47713641),
    ("sliver -0.0017", 0x32360e2537e4f249),
    ("sliver -0.0016", 0x81a99dd7894aae80),
    ("sliver -0.0015", 0x95e01a890de53c3f),
    ("sliver -0.0014", 0x3a1bbc23ae2c43cf),
    ("sliver -0.0013", 0x8fbd2bac4091b401),
    ("sliver -0.0012", 0x61229b52151291ac),
    ("sliver -0.0009", 0x36585c10ba9a1ea8),
    ("sliver -0.0008", 0x78cd2476220053a1),
    ("sliver -0.0007", 0x589ddb80a25a8428),
    ("sliver -0.0006", 0x46f0eaff6f945dc8),
    ("sliver -0.0005", 0xe84a7cea043b89e9),
    ("sliver -0.0004", 0x44c539af0962968a),
    ("sliver -0.0003", 0xc000321bb3bb0773),
    ("sliver -0.0002", 0x2e5bb5c171728f0d),
    ("sliver -0.0001", 0x35f50f9ad9bf8932),
    ("sliver 0", 0x173b57620e8e93d7),
    ("sliver 0.0001", 0x2b816e0b284da2e2),
    ("sliver 0.0002", 0x01f236538249d1ae),
    ("sliver 0.0003", 0xf6e6ec67d9350c56),
    ("sliver 0.0006", 0xcb9df3dfa37a79d6),
    ("sliver 0.0007", 0xeb58d97178ce7946),
    ("sliver 0.0008", 0x05cde3232712ac89),
    ("sliver 0.001", 0xd53aa611cedb8755),
    ("sliver 0.0011", 0x8482b86d57af2a99),
    ("sliver 0.0014", 0xb2c9a4e0d9c73169),
    ("sliver 0.0015", 0x371c807f845d537a),
    ("sliver 0.0017", 0x1229c0f9fc787d06),
    ("sliver 0.0018", 0x29bc185ad40067e5),
    ("sliver 0.0019", 0xbdcbb03e2e47c0d8),
];

/// Every answer of [`todays_answers`] given before the change is given the
/// same to the bit; every one refused before is answered.
#[test]
fn todays_answers_are_kept_to_the_bit() {
    let mut wrong = Vec::new();
    for (name, found) in todays_answers() {
        let pin = PINS.iter().find(|(n, _)| *n == name).map(|(_, h)| *h);
        match (found, pin) {
            (Ok(h), Some(pin)) if h == pin => {}
            (Ok(h), Some(pin)) => wrong.push(format!("{name}: 0x{h:016x}, before 0x{pin:016x}")),
            (Ok(h), None) => println!("ANSWERED NOW {name}: 0x{h:016x}"),
            (Err(e), _) => wrong.push(format!("{name}: refused: {e}")),
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

// ------------------------------------------------- a ball far from the origin

/// A ball of radius 1 whose centre `p` lies `far` (times (1, −0.8, 0.6))
/// from the origin, and the plane through `p` with the normal (0.3, −0.5,
/// 0.8).
fn ball(far: f64) -> (Session, [f64; 3], Value) {
    let p = [far, -0.8 * far, 0.6 * far];
    let mut s = Session::new(Workspace::none());
    run(
        &mut s,
        "file.new",
        json!({"units": "mm", "tolerance": TOLERANCE}),
    );
    run(
        &mut s,
        "feature.sphere",
        json!({"name": "Ball", "position": p, "radius": 1.0}),
    );
    (s, p, json!({"point": p, "normal": [0.3, -0.5, 0.8]}))
}

/// A ball of radius 1 with its centre 1.4e5 and 1.4e7 from the origin,
/// refused by every command before 2026-11-22 (`craft3d-cli` on `main`
/// 5305636e: "the section of body 0: its edges take more panels than a
/// section may"; answered at 1.4e3): its section through the centre is
/// the circle, area π, moments π/4, torsion constant π/2 (to the torsion's
/// tolerance, 1e-4) about the centre; below the plane 0.3 above the
/// centre the cap π s² (3 − s)/3 with s = 1.3; the level for half its
/// volume the centre's plane; and floating with 0.3 of its volume below
/// and its centre of gravity 0.2 and −0.3 off its centre along y and z,
/// turned until the centre of gravity is below the centre, the normal
/// along (0, −0.2, 0.3), stable with a metacentric height of √0.13 about
/// every axis. Each held to 1e-9 (1e-7 for the floating, as the module's
/// own oracles) and to 64 units of the last place of the distance (1e3 for
/// the floating's derived numbers).
#[test]
fn a_far_ball_is_measured_with_its_torsion_level_and_floating() {
    use std::f64::consts::PI;
    for far in [1e5, 1e7] {
        let (mut s, p, plane) = ball(far);
        let distance = far * (1.0_f64 + 0.64 + 0.36).sqrt();
        let ulps = f64::EPSILON * distance;
        let what = format!("the ball {distance:e} from the origin");
        let number = |v: &Value| v.as_f64().unwrap_or_else(|| panic!("{what}: {v}"));
        let near = |got: f64, want: f64, allowed: f64, name: &str| {
            assert!(
                (got - want).abs() <= allowed,
                "{what}: {name} {got:e}, want {want:e} (allowed {allowed:e})"
            );
        };
        let section = run(
            &mut s,
            "measure.section",
            json!({"of": "Ball", "plane": plane}),
        );
        println!("BALL {distance:e} section: {section}");
        let perimeter = 2.0 * PI;
        near(
            number(&section["area"]),
            PI,
            1e-9 * PI + 64.0 * ulps * perimeter,
            "area",
        );
        for key in ["ix", "iy"] {
            near(
                number(&section[key]),
                PI / 4.0,
                1e-9 + 64.0 * ulps * perimeter,
                key,
            );
        }
        for (k, c) in p.iter().enumerate() {
            near(
                number(&section["centroid"][k]),
                *c,
                1e-9 + 64.0 * ulps * 3.0,
                "centroid",
            );
            near(
                number(&section["shearCentre"][k]),
                *c,
                1e-4 * PI.sqrt() + 64.0 * ulps * 3.0,
                "shear centre",
            );
        }
        assert_eq!(
            section["torsionSolve"]["converged"], true,
            "{what}: {section}"
        );
        near(
            number(&section["torsion"]),
            PI / 2.0,
            1e-4 * PI / 2.0,
            "torsion constant",
        );
        let below = run(
            &mut s,
            "measure.below",
            json!({"of": "Ball", "plane": plane, "offset": 0.3}),
        );
        println!("BALL {distance:e} below: {below}");
        let cap = PI * 1.3 * 1.3 * (3.0 - 1.3) / 3.0;
        near(
            number(&below["volume"]),
            cap,
            1e-9 * cap + 64.0 * ulps * 4.0 * PI,
            "cap",
        );
        near(
            number(&below["section"]["area"]),
            PI * (1.0 - 0.09),
            1e-9 * PI + 64.0 * ulps * perimeter,
            "the cap's circle",
        );
        let level = run(
            &mut s,
            "measure.levelFor",
            json!({"of": "Ball", "plane": plane, "volume": 2.0 * PI / 3.0}),
        );
        println!("BALL {distance:e} level: {level}");
        near(
            number(&level["offset"]),
            0.0,
            1e-9 + 64.0 * ulps * 4.0,
            "the level of half",
        );
        let floated = run(
            &mut s,
            "measure.floating",
            json!({"of": "Ball", "plane": {"point": p, "normal": [0, 0, 1]}, "volume": 0.4 * PI, "centerOfGravity": [p[0], p[1] + 0.2, p[2] - 0.3]}),
        );
        println!("BALL {distance:e} floating: {floated}");
        let e = &floated["equilibria"][0];
        let down = 0.13_f64.sqrt();
        for (k, want) in [0.0, -0.2 / down, 0.3 / down].into_iter().enumerate() {
            near(
                number(&e["normal"][k]),
                want,
                1e-7 + 1e3 * ulps,
                "the normal",
            );
        }
        assert_eq!(e["stable"], true, "{what}: {floated}");
        for principal in e["principal"].as_array().unwrap() {
            near(number(&principal["gm"]), down, 1e-7 + 1e3 * ulps, "GM");
        }
    }
}

/// A block 1 × 1 × 1 at the origin cut by a cylinder of radius R (1e4 and
/// 1e5) along z whose rim passes through (0.5, 0.3): a section small
/// against the size of its curve, not against its distance from the
/// origin, whose points carry the rounding of the cylinder's centre R
/// away, as a far section's do: refused before 2026-11-22 (`craft3d-cli` on
/// `main` 5305636e: "its edges take more panels than a section may";
/// answered at R = 100, to 4e-15 of the closed form below). Its section
/// at z = 0.4 is the square up to the arc, x ≤ 0.5 − u²/(R + √(R² − u²))
/// with u = y − 0.3, so its area is 0.5 less the integral of that over u
/// from −0.3 to 0.7, (0.7³ + 0.3³)/(6R) + (0.7⁵ + 0.3⁵)/(40R³) and terms
/// 1e-14 of it smaller; the volume below 0.4 times that; and the level
/// for 0.2 the height 0.2 over the area.
#[test]
fn a_block_cut_by_a_large_cylinder_is_measured() {
    for radius in [1e4, 1e5] {
        let mut s = Session::new(Workspace::none());
        run(
            &mut s,
            "file.new",
            json!({"units": "mm", "tolerance": TOLERANCE}),
        );
        for (command, params) in [
            (
                "feature.cylinder",
                json!({"name": "Big", "radius": radius, "height": 4.0, "position": [0.5 - radius, 0.3, -2.0]}),
            ),
            (
                "feature.block",
                json!({"name": "Small", "size": [1.0, 1.0, 1.0]}),
            ),
            (
                "feature.boolean",
                json!({"op": "intersect", "target": "Small", "tool": "Big", "name": "Piece"}),
            ),
        ] {
            let made = run(&mut s, command, params.clone());
            assert_eq!(made["status"], "ok", "{command} {params}: {made}");
        }
        let area = 0.5
            - (0.343 + 0.027) / (6.0 * radius)
            - (0.16807 + 0.00243) / (40.0 * radius * radius * radius);
        // 1e-9 of it and 64 units in the last place of the arc's centre's
        // distance and radius, along the section's perimeter (under 4).
        let rounding = 64.0 * f64::EPSILON * 2.0 * radius * 4.0;
        let number = |v: &Value| v.as_f64().unwrap();
        let below = run(
            &mut s,
            "measure.below",
            json!({"of": "Piece", "offset": 0.4}),
        );
        println!("LARGE CYLINDER {radius:e} below: {below}");
        let a = number(&below["section"]["area"]);
        assert!(
            within(a - area, 1e-9 * area + rounding),
            "R {radius}: area {a}, want {area}"
        );
        let v = number(&below["volume"]);
        assert!(
            within(v - 0.4 * area, 1e-9 * area + rounding),
            "R {radius}: volume {v}, want {}",
            0.4 * area
        );
        let level = run(
            &mut s,
            "measure.levelFor",
            json!({"of": "Piece", "volume": 0.2}),
        );
        println!("LARGE CYLINDER {radius:e} level: {level}");
        let offset = number(&level["offset"]);
        assert!(
            within(offset - 0.2 / area, 1e-9 + rounding),
            "R {radius}: the level for 0.2 is {offset}, want {}",
            0.2 / area
        );
    }
}

// ------------------------------------------- floating far from the origin

/// A body made by `command` with `params` (named `Body`), floated with
/// `volume` below the plane through `p` square to z and its centre of
/// gravity moved from `p` by `across` in x and y and `rise` in z.
fn floated(
    command: &str,
    params: Value,
    p: [f64; 3],
    volume: f64,
    (across, rise): ([f64; 2], f64),
) -> Value {
    let mut s = Session::new(Workspace::none());
    run(
        &mut s,
        "file.new",
        json!({"units": "mm", "tolerance": TOLERANCE}),
    );
    let made = run(&mut s, command, params.clone());
    assert_eq!(made["status"], "ok", "{command} {params}: {made}");
    run(
        &mut s,
        "measure.floating",
        json!({"of": "Body", "plane": {"point": p, "normal": [0, 0, 1]}, "volume": volume, "centerOfGravity": [p[0] + across[0], p[1] + across[1], p[2] + rise]}),
    )
}

/// `ratio` times (−0.37, 0.81, 0.45): where a floating body lies.
fn far_at(ratio: f64) -> [f64; 3] {
    [-0.37 * ratio, 0.81 * ratio, 0.45 * ratio]
}

/// What `measure.floating` of the box gave on `main` (1f196da4, its
/// `craft3d-cli` in release: `plan/measure-far/round2/boxfloat.py` in the
/// workshop), hashed as [`hash`] hashes it, at 3e4 to 1e7 times (−0.37,
/// 0.81, 0.45). The branch as first built changed the four from 1e5 on
/// (the review of 2026-11-22: `floats` null → 1, `levels` 4 or 5 → 15 or
/// 17, the two searches from the upright equilibrium answered).
const BOX_PINS: [(f64, u64); 5] = [
    (3e4, 0x1a68_7f55_2a54_4725),
    (1e5, 0x5d31_44d0_fff3_bbd3),
    (3e5, 0x9adf_f5af_7e02_99fd),
    (1e6, 0x83c2_1289_3b05_5fb3),
    (1e7, 0x2992_7c2b_2ee5_cd5e),
];

/// A box 2 × 1.3 × 0.7 floating with 0.91 below (a draft of 0.35) and its
/// centre of gravity 0.6 above its floor, unstable upright (GM about
/// −0.0226 across), 3e4 to 1e7 times from the origin: the search from the
/// start, upright, settles strictly there, so `main` answered; from 1e5 on
/// the sections of the two searches from it, turned 0.1 rad, do not
/// settle strictly and were listed under `unfinished` in the strict
/// words, the call answered with no stable equilibrium (`floats` null).
/// Those searches are held strictly still where the search from the start
/// was, so every answer is `main`'s to the bit: at 1e5 one equilibrium,
/// `levels` 4, `floats` null and the two searches unfinished in the strict
/// words; at 3e4 the two angles of loll, ±18.536°, as on `main`.
#[test]
fn a_far_floating_box_gives_what_main_gave() {
    for (ratio, pin) in BOX_PINS {
        let p = far_at(ratio);
        let found = floated(
            "feature.block",
            json!({"name": "Body", "position": p, "size": [2, 1.3, 0.7]}),
            p,
            0.91,
            ([1.0, 0.65], 0.6),
        );
        println!("BOX {ratio:e}: 0x{:016x} {found}", hash(&found));
        assert_eq!(hash(&found), pin, "the box at {ratio:e}: {found}");
        if ratio == 1e5 {
            assert_eq!(found["floats"], Value::Null, "{found}");
            assert_eq!(found["levels"], 4, "{found}");
            assert_eq!(found["equilibria"].as_array().map(Vec::len), Some(1));
            let words = "the section of body 0: its edges take more panels than a section may";
            assert_eq!(found["unfinished"], json!([words, words]), "{found}");
        }
    }
}

/// An upright cylinder of radius 1 and height 3 floating at a draft of 1
/// with its centre of gravity 0.85 above its floor (KB 0.5, BM r²/4T =
/// 0.25, GM −0.1: unstable upright in every direction), at the origin and
/// 1e5 times (−0.37, 0.81, 0.45) from it. At the origin `main`'s answer to
/// the bit (its hash from `craft3d-cli` of `main` 1f196da4); 1e5 away
/// refused by `main` (the search from the start: "the section of body 0:
/// its edges take more panels than a section may"), so the searches after
/// it may be held to the rounding as well. Both give the upright
/// equilibrium, GM −0.1 both ways, and four angles of loll (one each way
/// along each direction), each tilted by tan² φ = −2 GM / BM = 0.8
/// (41.8103°: the waterline stays below the rim and the deck), neutral
/// about the vertical (a loll turned about it is one) and stable across
/// with GM −2 GM / cos φ = 0.2 √1.8; none unfinished.
#[test]
fn a_far_floating_cylinder_finds_its_angles_of_loll() {
    use craft3d_math::Portable;
    let loll = 0.8_f64.sqrt().patan().to_degrees();
    for ratio in [0.0, 1e5] {
        let p = far_at(ratio);
        let found = floated(
            "feature.cylinder",
            json!({"name": "Body", "position": p, "radius": 1.0, "height": 3.0}),
            p,
            std::f64::consts::PI,
            ([0.0, 0.0], 0.85),
        );
        println!("CYLINDER {ratio:e}: 0x{:016x} {found}", hash(&found));
        if ratio == 0.0 {
            assert_eq!(
                hash(&found),
                0xe40e_5693_7ff0_fbd1,
                "main's answer: {found}"
            );
        }
        assert_eq!(found["unfinished"], json!([]), "{found}");
        let all = found["equilibria"].as_array().unwrap();
        assert_eq!(all.len(), 5, "{found}");
        let number = |v: &Value| v.as_f64().unwrap_or_else(|| panic!("{v}"));
        for principal in all[0]["principal"].as_array().unwrap() {
            assert_eq!(principal["stability"], "unstable", "{found}");
            assert!((number(&principal["gm"]) + 0.1).abs() <= 1e-9, "{found}");
        }
        for e in &all[1..] {
            assert!(
                (number(&e["tilt"]) - loll).abs() <= 1e-6,
                "a loll of {} deg, want {loll}",
                e["tilt"]
            );
            let principal = e["principal"].as_array().unwrap();
            assert_eq!(principal[0]["stability"], "neutral", "{e}");
            assert_eq!(principal[1]["stability"], "stable", "{e}");
            assert!(
                (number(&principal[1]["gm"]) - 0.2 * 1.8_f64.sqrt()).abs() <= 1e-8,
                "{e}"
            );
        }
    }
}
