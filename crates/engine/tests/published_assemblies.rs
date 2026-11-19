//! The published example assemblies the program carries
//! (`docs/examples/published`), pinned: each instance's pose and placed
//! box, each mate's status and value (turns included), the solve's degrees
//! of freedom, error and note, each assertion's status, measure and proven
//! bound at rest, and over a motion of 73 samples each assertion's status,
//! least and greatest value and every sample's status; and the walker's leg
//! unpacked and opened alone at three leg scales. The golden file was
//! recorded from release/0.1 at 1090fe93 (before the assembly-trust fixes)
//! with the same commands through `craft3d-cli`. A-030 changes nothing
//! here. What A-016 and A-036 change (for 0.2) is declared case by case,
//! entry by entry, with its reason: every other entry must match the
//! parent's golden, and the declared ones must match
//! `published-assemblies-0.2.json`, recorded on the branch that made them
//! (assembly-trust, 2026-11-14) and checked there against what is
//! independent of the solver (the walker's own closed-form gait, the leg's
//! joints at the values they were placed at).
//!
//! Numbers are held to 1e-6 (poses, boxes, mate values) and 1e-5 (measures,
//! which a search finds: a clearance found moves by 1e-7 when a pose moves
//! by 1e-14); statuses, notes and texts exactly.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::plain::run;
use common::{session_in, workdir};
use craft3d_engine::Session;
use serde_json::{Value, json};
use std::path::PathBuf;

fn published() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/examples/published")
}

fn golden_file(name: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/published_goldens")
        .join(name);
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// The parent's digests.
fn golden() -> Value {
    golden_file("published-assemblies.json")
}

/// The declared entries' digests, as A-016 and A-036 make them.
fn golden_02() -> Value {
    golden_file("published-assemblies-0.2.json")
}

/// The digest of one case, as the golden holds it.
fn digest(file: &str, setup: &[(&str, Value)], motion: bool) -> Value {
    let mut s = session_in(&published());
    run(
        &mut s,
        "file.open",
        json!({ "path": format!("{file}.3dcraftasm") }),
    );
    digest_open(&mut s, setup, motion)
}

/// [`digest`] of the assembly open in `s`.
fn digest_open(s: &mut Session, setup: &[(&str, Value)], motion: bool) -> Value {
    for (id, params) in setup {
        run(s, id, params.clone());
    }
    let inspect = run(s, "assembly.inspect", json!({ "measures": true }));
    let solve = run(s, "assembly.solve", json!({}));
    let asserts = run(s, "assert.list", json!({}));
    let instances = inspect["instances"].as_array().unwrap();
    let pose = |i: &Value| {
        json!([
            i["name"],
            i["pose"]["origin"],
            i["pose"]["x"],
            i["pose"]["y"],
            i["pose"]["z"]
        ])
    };
    let at_least = |message: &str| -> Value {
        message
            .split("at least ")
            .nth(1)
            .and_then(|rest| {
                let number: String = rest
                    .chars()
                    .take_while(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | 'e' | 'E' | '+'))
                    .collect();
                number.parse::<f64>().ok()
            })
            .map_or(Value::Null, |v| json!(v))
    };
    let mut d = json!({
        "poses": instances.iter().filter(|i| !i["pose"].is_null()).map(pose).collect::<Vec<_>>(),
        "boxes": instances.iter().map(|i| {
            let b = &i["measured"]["box"];
            json!([i["name"], b.get(0).cloned().unwrap_or(Value::Null), b.get(1).cloned().unwrap_or(Value::Null)])
        }).collect::<Vec<_>>(),
        "mates": solve["mates"].as_array().unwrap().iter()
            .map(|m| json!([m["name"], m["status"], m["value"]])).collect::<Vec<_>>(),
        "dof": solve["dof"],
        "error": solve["error"],
        "note": solve["note"],
        "asserts": asserts["assertions"].as_array().unwrap().iter().map(|a| {
            json!([a["name"], a["status"], a["measured"], at_least(a["message"].as_str().unwrap_or(""))])
        }).collect::<Vec<_>>(),
    });
    if motion {
        let m = run(s, "motion.check", json!({ "samples": 73, "details": true }));
        d["motion"] = json!({
            "ok": m["ok"],
            "deadPoints": m["deadPoints"].as_array().map_or(0, Vec::len),
            "assertions": m["assertions"].as_array().unwrap().iter().map(|a| {
                json!([
                    a["name"], a["status"], a["min"]["value"], a["max"]["value"],
                    a["samples"].as_array().map(|ss| ss.iter().map(|s| s["status"].clone()).collect::<Vec<_>>())
                        .unwrap_or_default()
                ])
            }).collect::<Vec<_>>(),
        });
    }
    d
}

/// Every difference between `got` and `want` beyond the tolerances, as
/// paths: numbers under `poses`, `boxes` and `mates` to 1e-6, other
/// numbers to 1e-5, everything else exactly.
fn differences(path: &str, got: &Value, want: &Value, out: &mut Vec<String>) {
    match (got, want) {
        (Value::Number(a), Value::Number(b)) => {
            let (a, b) = (a.as_f64().unwrap(), b.as_f64().unwrap());
            let fine = ["poses", "boxes", "mates"]
                .iter()
                .any(|k| path.starts_with(k));
            let tol = if fine { 1e-6 } else { 1e-5 };
            if (a - b).abs() > tol {
                out.push(format!("{path}: {a} (golden {b})"));
            }
        }
        (Value::Array(a), Value::Array(b)) => {
            if a.len() != b.len() {
                out.push(format!("{path}: {} items (golden {})", a.len(), b.len()));
                return;
            }
            for (k, (x, y)) in a.iter().zip(b).enumerate() {
                differences(&format!("{path}[{k}]"), x, y, out);
            }
        }
        (Value::Object(a), Value::Object(b)) => {
            for (k, y) in b {
                differences(
                    &format!("{path}.{k}"),
                    a.get(k).unwrap_or(&Value::Null),
                    y,
                    out,
                );
            }
        }
        (a, b) if a != b => out.push(format!("{path}: {a} (golden {b})")),
        _ => {}
    }
}

/// What a change makes differ in a case: the entries of `poses`, `boxes`
/// and `mates` named here (by their first item, an instance's or a mate's
/// name), the assertions at rest and over the motion named here, and the
/// keys (`note`, `motion.ok`).
#[derive(Default)]
struct Declared<'a> {
    names: &'a [&'a str],
    asserts: &'a [&'a str],
    motion_asserts: &'a [&'a str],
    keys: &'a [&'a str],
}

/// `d` split into what `dec` does not declare and what it does.
fn split(d: &Value, dec: &Declared) -> (Value, Value) {
    let mut kept = d.clone();
    let mut declared = json!({});
    let named = |e: &Value, names: &[&str]| names.contains(&e[0].as_str().unwrap_or(""));
    let part = |kept: &mut Value, declared: &mut Value, at: &[&str], names: &[&str]| {
        let mut from = d;
        for k in at {
            from = &from[k];
        }
        let Some(entries) = from.as_array() else {
            return;
        };
        let (mine, rest): (Vec<Value>, Vec<Value>) =
            entries.iter().cloned().partition(|e| named(e, names));
        let (mut into_kept, mut into_declared) = (&mut *kept, &mut *declared);
        for k in at {
            into_kept = &mut into_kept[k];
            into_declared = &mut into_declared[k];
        }
        *into_kept = Value::Array(rest);
        *into_declared = Value::Array(mine);
    };
    for k in ["poses", "boxes", "mates"] {
        part(&mut kept, &mut declared, &[k], dec.names);
    }
    part(&mut kept, &mut declared, &["asserts"], dec.asserts);
    if d.get("motion").is_some() {
        part(
            &mut kept,
            &mut declared,
            &["motion", "assertions"],
            dec.motion_asserts,
        );
    }
    for key in dec.keys {
        if let Some(sub) = key.strip_prefix("motion.") {
            declared["motion"][sub] = d["motion"][sub].clone();
            kept["motion"].as_object_mut().unwrap().remove(sub);
        } else {
            declared[*key] = d[*key].clone();
            kept.as_object_mut().unwrap().remove(*key);
        }
    }
    (kept, declared)
}

/// Checks one case: what `dec` does not declare against the parent's
/// golden, what it declares against the 0.2 golden (with nothing declared,
/// the whole case against the parent's).
fn check(file: &str, case: &str, got: &Value, dec: Option<&Declared>) {
    let want = &golden()[file][case];
    assert!(!want.is_null(), "no golden for {file} / {case}");
    let none = Declared::default();
    let dec = dec.unwrap_or(&none);
    let (got_kept, got_declared) = split(got, dec);
    let (want_kept, _) = split(want, dec);
    let mut out = Vec::new();
    for (k, w) in want_kept.as_object().unwrap() {
        differences(k, &got_kept[k], w, &mut out);
    }
    if dec.names.len() + dec.asserts.len() + dec.motion_asserts.len() + dec.keys.len() > 0 {
        let new = &golden_02()[file][case];
        assert!(!new.is_null(), "no 0.2 golden for {file} / {case}");
        for (k, w) in new.as_object().unwrap() {
            differences(&format!("(0.2) {k}"), &got_declared[k], w, &mut out);
        }
    }
    assert!(
        out.is_empty(),
        "{file} / {case} differs from its golden:\n{}",
        out.join("\n")
    );
}

#[test]
fn the_toggle_clamp_is_as_pinned() {
    let got = digest("toggle-clamp", &[], true);
    check("toggle-clamp", "default", &got, None);
}

#[test]
fn the_scissor_jack_is_as_pinned() {
    let got = digest("scissor-jack", &[], true);
    check("scissor-jack", "default", &got, None);
}

#[test]
fn the_radial_engine_is_as_pinned() {
    let got = digest("radial-engine", &[], true);
    check("radial-engine", "default", &got, None);
}

/// The walker at six crank starts: the published range's steps of 60°
/// (as published) and 180°, outside it. Only 180° differs from the parent.
/// There (A-016) the rest solve from where the legs are placed landed two
/// legs, Aft[2] and Fore[6], on their mirrored loop, and three assertions
/// failed at rest and over the motion ("Enough feet stand on the floor",
/// "Centre of mass has a floor foot in every quadrant", "Every foot stays
/// inside the closed-form gait", the walker's own closed form). Carried
/// from the drive the placement holds, every foot is on the gait and every
/// assertion passes; the two legs' boxes move, two measures over the motion
/// that their feet decide change their least or greatest value ("Feet slip
/// at most the bound while on the floor", "No foot goes below the floor
/// level", both passing before and after), and the note says the branch is
/// a reading: the legs' parts are not placed where their mates meet. Every
/// other entry is the parent's: the 45 poses, the 43 other boxes, the
/// mates, the 30 assertions that passed and the 5 inactive at rest, the 33
/// other assertions over the motion.
#[test]
fn the_jansen_walker_is_as_pinned_at_its_crank_starts() {
    for start in [-120, -60, 0, 60, 120, 180] {
        let setup = [(
            "variable.set",
            json!({ "name": "crank_start", "expr": format!("{start} deg") }),
        )];
        let got = digest("jansen-walker", &setup, start == 0 || start == 180);
        let a016 = Declared {
            names: &["Aft[2]", "Fore[6]"],
            asserts: &[
                "Enough feet stand on the floor",
                "Centre of mass has a floor foot in every quadrant",
                "Every foot stays inside the closed-form gait",
            ],
            motion_asserts: &[
                "Enough feet stand on the floor",
                "Feet slip at most the bound while on the floor",
                "Centre of mass has a floor foot in every quadrant",
                "No foot goes below the floor level",
                "Every foot stays inside the closed-form gait",
            ],
            keys: &["note", "motion.ok"],
        };
        let declared = (start == 180).then_some(&a016);
        check(
            "jansen-walker",
            &format!("crank_start {start}"),
            &got,
            declared,
        );
        if start == 180 {
            for a in got["asserts"].as_array().unwrap() {
                assert!(a[1] == "passed" || a[1] == "inactive", "at rest: {a}");
            }
            assert_eq!(got["motion"]["ok"], true, "{}", got["motion"]);
            assert!(
                got["note"]
                    .as_str()
                    .unwrap_or("")
                    .starts_with("its parts as placed do not meet their mates"),
                "{}",
                got["note"]
            );
        }
    }
}

/// The walker's leg, unpacked and opened on its own (`Fore_1`, five free
/// joints: nothing drives it alone), at its own leg scale and at 1.6 and
/// 2.4. At its own scale it is the parent's. At the others (A-036) its
/// parts no longer meet their mates where they are placed, and the solve
/// that closes them turned every free joint a share (Hip_D to −2.4076 and
/// −2.4115 rad, Foot_knee to −2.3488 at 2.4); now each free joint keeps
/// the value it is placed at, the same at every scale (Hip_D −2.40720),
/// and the parts it holds take their poses from there: Foot, Bar_K, Bar_C
/// and Bar_F move (up to 1.6 mm at 2.4). Every other entry is the parent's.
#[test]
fn the_walkers_leg_opened_alone_is_as_pinned_at_its_leg_scales() {
    let dir = workdir("published", "leg-alone");
    std::fs::copy(
        published().join("jansen-walker.3dcraftasm"),
        dir.join("walker.3dcraftasm"),
    )
    .unwrap();
    let mut s = session_in(&dir);
    run(&mut s, "file.open", json!({ "path": "walker.3dcraftasm" }));
    run(&mut s, "assembly.unpack", json!({ "folder": "unpacked" }));
    let a036 = Declared {
        names: &["Foot", "Bar_K", "Bar_C", "Bar_F", "Foot_knee", "Hip_D"],
        ..Declared::default()
    };
    let mut own_values = Vec::new();
    for (case, scale) in [
        ("leg_scale default", None),
        ("leg_scale 1.6", Some(1.6)),
        ("leg_scale 2.4", Some(2.4)),
    ] {
        run(
            &mut s,
            "file.open",
            json!({ "path": "unpacked/Fore_1.3dcraftasm" }),
        );
        let setup: Vec<(&str, Value)> = scale
            .map(|v| {
                (
                    "param.set",
                    json!({ "values": { "leg_scale": v }, "clamp": true }),
                )
            })
            .into_iter()
            .collect();
        let got = digest_open(&mut s, &setup, false);
        let values: Vec<f64> = got["mates"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m[2].as_f64().unwrap())
            .collect();
        if scale.is_none() {
            check("jansen-walker leg Fore_1", case, &got, None);
            own_values = values;
        } else {
            check("jansen-walker leg Fore_1", case, &got, Some(&a036));
            // Each free joint at the value it is placed at, as at the
            // leg's own scale, where the parts meet their mates.
            assert_eq!(values.len(), own_values.len());
            for (v, w) in values.iter().zip(&own_values) {
                assert!(
                    (v - w).abs() < 1e-9,
                    "{case}: {values:?} against {own_values:?}"
                );
            }
            assert_eq!(got["note"], Value::Null, "{case}");
        }
    }
}

/// Each published assembly is what it says, and its check answers as the
/// release 0.2.0 answered it (`craft3d-cli run <file> --cmd
/// document.check`, measured on its build): every placed body checked, no
/// problem, its assertions as they stand. The check asks about the
/// instances, mates, couplings, solve and subassemblies of an assembly
/// since it gave a wrong ok for one with an instance that was not placed;
/// an assembly that is right answers the same, to the byte.
#[test]
fn the_published_assemblies_check_ok_as_before() {
    for (file, count, inactive, passed, bodies) in [
        ("toggle-clamp", 20, 4, 16, 15),
        ("jansen-walker", 38, 5, 33, 250),
        ("scissor-jack", 26, 11, 15, 24),
        ("radial-engine", 140, 4, 136, 237),
    ] {
        let mut s = session_in(&published());
        run(
            &mut s,
            "file.open",
            json!({ "path": format!("{file}.3dcraftasm") }),
        );
        let checked = run(&mut s, "document.check", json!({}));
        assert_eq!(
            checked,
            json!({
                "assertionSummary": {
                    "broken": [], "count": count, "inactive": inactive,
                    "passed": passed, "pending": 0, "unproven": [],
                },
                "bodiesChecked": bodies,
                "ok": true,
                "problems": [],
            }),
            "{file}"
        );
    }
}
