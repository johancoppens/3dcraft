//! One rule for a feature that is off (inactive by its condition, or
//! suppressed), for every kind that reads it: it behaves as if it were
//! not in the history (docs/design/regeneration.md › *Switched off*). A
//! face or an edge of what is off is nothing to a fillet, a chamfer or a
//! face operation: the others are changed, and with none left the feature
//! passes on the body it would have changed; a union whose target is off
//! passes its tool on; the appearance, the material and the face colours
//! of a feature that is off apply to the body it passes on only where
//! that body has none of its own (decided by the developer on
//! 2026-10-26). Every command is followed by a regeneration from
//! nothing (`checked`), and the range is proven with `document.sample`,
//! which regenerates every point fresh.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::{
    assert_close, built, feature, inspect, new_document, run, session, session_in, workdir,
};
use craft3d_engine::Session;
use serde_json::{Value, json};

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

/// A feature under test: the command and its params (lengths for scale
/// 1), which name the faces of `Boss` (`F2`: its side `face(0)`, its top
/// `face(2)`) and, for `partial`, an edge of `Base` (`F1`) too.
struct Kind {
    what: &'static str,
    command: &'static str,
    params: fn(f64) -> Value,
    /// The same feature without the faces of the boss, when it is left
    /// with some (`None`: it has none left, and passes its body on).
    without_boss: Option<fn(f64) -> Value>,
    /// Makes a body of its own beside the one it is made from.
    makes: bool,
    /// Is given the body it changes as `source` (a shell).
    source: bool,
}

fn kinds() -> Vec<Kind> {
    vec![
        Kind {
            what: "fillet",
            command: "feature.fillet",
            params: |k| json!({"edges": [["F2:face(0)", "F2:face(2)"]], "radius": k}),
            without_boss: None,
            makes: false,
            source: false,
        },
        Kind {
            what: "fillet, partly of the boss",
            command: "feature.fillet",
            params: |k| {
                json!({"edges": [["F2:face(0)", "F2:face(2)"], ["F1:face(1)", "F1:face(2)"]],
                       "radius": k})
            },
            without_boss: Some(|k| json!({"edges": [["F1:face(1)", "F1:face(2)"]], "radius": k})),
            makes: false,
            source: false,
        },
        Kind {
            what: "chamfer",
            command: "feature.chamfer",
            params: |k| json!({"edges": [["F2:face(0)", "F2:face(2)"]], "distance": k}),
            without_boss: None,
            makes: false,
            source: false,
        },
        Kind {
            what: "chamfer, partly of the boss",
            command: "feature.chamfer",
            params: |k| {
                json!({"edges": [["F1:face(1)", "F1:face(3)"], ["F2:face(0)", "F2:face(2)"]],
                       "distance": k})
            },
            without_boss: Some(|k| json!({"edges": [["F1:face(1)", "F1:face(3)"]], "distance": k})),
            makes: false,
            source: false,
        },
        Kind {
            what: "offset face",
            command: "feature.offsetFace",
            params: |k| json!({"faces": ["F2:face(2)"], "distance": 2.0 * k}),
            without_boss: None,
            makes: false,
            source: false,
        },
        Kind {
            what: "offset face, partly of the boss",
            command: "feature.offsetFace",
            params: |k| json!({"faces": ["F2:face(2)", "F1:face(0)"], "distance": 2.0 * k}),
            without_boss: Some(|k| json!({"faces": ["F1:face(0)"], "distance": 2.0 * k})),
            makes: false,
            source: false,
        },
        Kind {
            what: "move face",
            command: "feature.moveFace",
            params: |k| json!({"faces": ["F2:face(2)"], "translate": [0, 0, 2.0 * k]}),
            without_boss: None,
            makes: false,
            source: false,
        },
        Kind {
            what: "thread",
            command: "feature.thread",
            params: |k| json!({"face": "F2:face(0)", "start": "F2:face(2)", "size": format!("{} mm", 8.0 * k), "pitch": k, "length": 5.0 * k}),
            without_boss: None,
            makes: false,
            source: false,
        },
        Kind {
            what: "thicken",
            command: "feature.thicken",
            params: |k| json!({"faces": ["F2:face(2)"], "thickness": k}),
            without_boss: None,
            makes: true,
            source: false,
        },
        Kind {
            // None of its open faces there: it passes its body on, and
            // never seals a hollow nobody asked for.
            what: "shell, opened on the boss",
            command: "feature.shell",
            params: |k| json!({"open": ["F2:face(2)"], "thickness": k}),
            without_boss: None,
            makes: false,
            source: true,
        },
        Kind {
            what: "shell, opened on the boss and under the base",
            command: "feature.shell",
            params: |k| json!({"open": ["F2:face(2)", "F1:face(0)"], "thickness": k}),
            without_boss: Some(|k| json!({"open": ["F1:face(0)"], "thickness": k})),
            makes: false,
            source: true,
        },
    ]
}

/// A base block (`F1`, 40 × 20 × 10), a boss on it (`F2`) united with it
/// (`Joined`, `F3`), the feature under test (`X`, `F4`) on the boss's
/// faces, and a hole drilled through what comes out of it (`Drill`,
/// `Drilled`). With `boss` false the boss, the union and `X` are left
/// out (or, with `x`, `X` is built without the boss's faces), and the
/// hole is drilled in what is there: what switching the boss off has to
/// give.
fn model(s: &mut Session, kind: &Kind, k: f64, boss: bool, x: bool) {
    new_document(s, k);
    built(
        s,
        "feature.block",
        json!({"size": [40.0 * k, 20.0 * k, 10.0 * k], "name": "Base"}),
    );
    let mut last = "Base";
    if boss {
        built(
            s,
            "feature.cylinder",
            json!({"radius": 4.0 * k, "height": 10.0 * k, "position": [10.0 * k, 10.0 * k, 10.0 * k],
                   "name": "Boss"}),
        );
        built(
            s,
            "feature.boolean",
            json!({"op": "union", "target": "Base", "tool": "Boss", "name": "Joined"}),
        );
        last = "Joined";
    }
    let params = match (boss, kind.without_boss) {
        (true, _) => Some((kind.params)(k)),
        (false, Some(without)) if x => Some(without(k)),
        (false, Some(_)) => None,
        (false, None) => None,
    };
    if let Some(mut params) = params {
        params["name"] = json!("X");
        if kind.source {
            params["source"] = json!(last);
        }
        built(s, kind.command, params);
        if !kind.makes {
            last = "X";
        }
    }
    built(
        s,
        "feature.cylinder",
        json!({"radius": 3.0 * k, "height": 30.0 * k, "position": [30.0 * k, 10.0 * k, -5.0 * k],
               "name": "Drill"}),
    );
    built(
        s,
        "feature.boolean",
        json!({"op": "subtract", "target": last, "tool": "Drill", "name": "Drilled"}),
    );
}

/// The bodies of the document, each as `(volume, area, faces)`, in
/// history order.
fn bodies(s: &mut Session) -> Vec<(f64, f64, u64)> {
    inspect(s)["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| {
            assert_eq!(b["valid"], true, "{b}");
            (
                b["volume"].as_f64().unwrap(),
                b["area"].as_f64().unwrap(),
                b["faces"].as_u64().unwrap(),
            )
        })
        .collect()
}

fn same_bodies(got: &[(f64, f64, u64)], expected: &[(f64, f64, u64)], context: &str) {
    assert_eq!(got.len(), expected.len(), "{context}: {got:?} {expected:?}");
    for (a, b) in got.iter().zip(expected) {
        assert!(
            (a.0 - b.0).abs() <= 1e-9 * b.0.abs()
                && (a.1 - b.1).abs() <= 1e-9 * b.1.abs()
                && a.2 == b.2,
            "{context}: {got:?} {expected:?}"
        );
    }
}

/// The features with their states and the bodies, every measure and name.
fn whole(s: &mut Session) -> Value {
    let document = inspect(s);
    json!({"features": document["features"], "bodies": document["bodies"]})
}

/// Every feature built (none failed).
fn nothing_failed(s: &mut Session, context: &str) {
    let document = inspect(s);
    for f in document["features"].as_array().unwrap() {
        assert_ne!(f["status"], "failed", "{context}: {f}");
    }
}

fn boss_switch(s: &mut Session) {
    run(s, "variable.set", json!({"name": "boss", "expr": "1"}));
    run(
        s,
        "variable.publish",
        json!({"name": "boss", "label": "Boss", "kind": "boolean"}),
    );
    run(
        s,
        "feature.when",
        json!({"feature": "Boss", "expr": "boss"}),
    );
}

/// Items 13 and 16 of R1 for every kind that names faces or edges: with
/// the boss inactive, the feature under test (on, or inactive by the same
/// condition) is built on the faces that are left or, with none left,
/// passes on the body it would have changed: the union's, which is the
/// base. The bodies are those of the model built without the boss, at
/// three scales; nothing fails; switched on again the document is what
/// it was, to the bit; and the range is proven point by point.
#[test]
fn a_feature_on_faces_of_an_inactive_feature_changes_what_is_left() {
    for kind in kinds() {
        for k in SCALES {
            for conditional in [false, true] {
                let context = format!("{} at {k}, conditional {conditional}", kind.what);
                let mut s = session();
                model(&mut s, &kind, k, true, true);
                boss_switch(&mut s);
                if conditional {
                    run(
                        &mut s,
                        "feature.when",
                        json!({"feature": "X", "expr": "boss"}),
                    );
                }
                let on = whole(&mut s);
                let mut without = session();
                // An inactive `X` is not there either.
                model(&mut without, &kind, k, false, !conditional);
                let expected = bodies(&mut without);

                run(&mut s, "param.set", json!({"values": {"boss": false}}));
                nothing_failed(&mut s, &context);
                same_bodies(&bodies(&mut s), &expected, &context);
                let x = feature(&mut s, "X");
                let status = if conditional { "inactive" } else { "ok" };
                assert_eq!(x["status"], status, "{context}: {x}");
                match (kind.makes, kind.without_boss.is_some(), conditional) {
                    // A layer of nothing is nothing.
                    (true, _, _) => assert!(x.get("passesNothing").is_some(), "{context}: {x}"),
                    // Built on what is left.
                    (false, true, false) => assert!(x.get("passesOn").is_none(), "{context}: {x}"),
                    // The union's body, which is the base.
                    _ => assert_eq!(x["passesOn"], "F3", "{context}: {x}"),
                }
                let joined = feature(&mut s, "Joined");
                assert_eq!(joined["passesOn"], "F1", "{context}: {joined}");
                // What it left out, and why.
                let left = x["leftOut"]
                    .as_array()
                    .unwrap_or_else(|| panic!("{context}: {x}"));
                assert!(!left.is_empty(), "{context}: {x}");
                for entry in left {
                    assert!(
                        entry["name"].as_str().unwrap().contains("F2:"),
                        "{context}: {x}"
                    );
                    assert_eq!(
                        entry["why"], "`Boss` is inactive (its condition `boss` is 0)",
                        "{context}: {x}"
                    );
                }
                // The body is made from the base, through what passed it on.
                let made = run(
                    &mut s,
                    "select.query",
                    json!({"kind": "body", "where": {"madeBy": "Base"}}),
                );
                assert_eq!(made["count"], 1, "{context}: {made}");

                run(&mut s, "param.set", json!({"values": {"boss": true}}));
                assert_eq!(whole(&mut s), on, "{context}: switched on again");
                assert!(feature(&mut s, "X").get("leftOut").is_none(), "{context}");

                if k == 1.0 && !conditional {
                    let sampled = run(
                        &mut s,
                        "document.sample",
                        json!({"samples": 0, "timeLimitMs": null}),
                    );
                    assert_eq!(sampled["ok"], true, "{context}: {sampled}");
                }
            }
        }
    }
}

/// Item 13: a fillet of the edges of a block that is inactive, with
/// nothing between, has no body to change: it passes nothing on (its
/// status stays ok), and what reads it goes on without it. Item 20: a
/// union whose target is that fillet passes its tool on, named as when
/// the union is built (`F4:copy(1)/…`), so a chamfer of the tool's faces
/// through the union is built either way.
#[test]
fn a_fillet_of_an_inactive_block_passes_nothing_and_the_union_its_tool() {
    for k in SCALES {
        let context = format!("at {k}");
        let mut s = session();
        new_document(&mut s, k);
        built(
            &mut s,
            "feature.block",
            json!({"size": [20.0 * k, 20.0 * k, 10.0 * k], "name": "A"}),
        );
        built(
            &mut s,
            "feature.fillet",
            json!({"edges": [["F1:face(1)", "F1:face(2)"]], "radius": k, "name": "Round"}),
        );
        built(
            &mut s,
            "feature.block",
            json!({"size": [10.0 * k, 10.0 * k, 10.0 * k], "position": [30.0 * k, 0, 0], "name": "B"}),
        );
        built(
            &mut s,
            "feature.boolean",
            json!({"op": "union", "target": "Round", "tool": "B", "name": "Both"}),
        );
        built(
            &mut s,
            "feature.chamfer",
            json!({"edges": [["F3:face(1)", "F3:face(3)"]], "distance": k, "name": "Edge"}),
        );
        let on = whole(&mut s);
        run(&mut s, "feature.when", json!({"feature": "A", "expr": "0"}));
        nothing_failed(&mut s, &context);
        let round = feature(&mut s, "Round");
        assert_eq!(round["status"], "ok", "{context}: {round}");
        assert_eq!(
            round["passesNothing"], "`A` is inactive (its condition `0` is 0)",
            "{context}: {round}"
        );
        let both = feature(&mut s, "Both");
        assert_eq!(both["passesOn"], "F3", "{context}: {both}");
        let names: Vec<String> = inspect(&mut s)["bodies"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b["name"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(names, ["Edge"], "{context}");
        // The chamfered block alone.
        let mut alone = session();
        new_document(&mut alone, k);
        built(
            &mut alone,
            "feature.block",
            json!({"size": [10.0 * k, 10.0 * k, 10.0 * k], "position": [30.0 * k, 0, 0], "name": "B"}),
        );
        built(
            &mut alone,
            "feature.chamfer",
            json!({"edges": [["F1:face(1)", "F1:face(3)"]], "distance": k, "name": "Edge"}),
        );
        same_bodies(&bodies(&mut s), &bodies(&mut alone), &context);
        run(
            &mut s,
            "feature.when",
            json!({"feature": "A", "expr": null}),
        );
        assert_eq!(whole(&mut s), on, "{context}: switched on again");
    }
}

/// Item 20 for the other operations and a kept tool: a subtraction or an
/// intersection from nothing passes nothing on and takes its tool as it
/// would (no cutter is left over), unless it keeps it; a union that keeps
/// its tool passes nothing on and leaves the tool as it is. Switched on
/// again, the document is what it was.
#[test]
fn a_boolean_whose_target_is_inactive() {
    // What the operation passes on, and the bodies left.
    for (op, keep, passes_tool, left) in [
        ("union", false, true, vec!["Op"]),
        ("union", true, false, vec!["T"]),
        ("subtract", false, false, vec![]),
        ("subtract", true, false, vec!["T"]),
        ("intersect", false, false, vec![]),
        ("intersect", true, false, vec!["T"]),
    ] {
        let context = format!("{op} keepTool {keep}");
        let mut s = session();
        new_document(&mut s, 1.0);
        built(
            &mut s,
            "feature.block",
            json!({"size": [20, 20, 10], "name": "A"}),
        );
        built(
            &mut s,
            "feature.cylinder",
            json!({"radius": 3, "height": 30, "position": [10, 10, -5], "name": "T"}),
        );
        built(
            &mut s,
            "feature.boolean",
            json!({"op": op, "target": "A", "tool": "T", "keepTool": keep, "name": "Op"}),
        );
        let on = whole(&mut s);
        run(&mut s, "feature.when", json!({"feature": "A", "expr": "0"}));
        nothing_failed(&mut s, &context);
        let done = feature(&mut s, "Op");
        let names: Vec<String> = inspect(&mut s)["bodies"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b["name"].as_str().unwrap().to_string())
            .collect();
        if passes_tool {
            assert_eq!(done["passesOn"], "F2", "{context}: {done}");
        } else {
            assert_eq!(
                done["passesNothing"], "`A` is inactive (its condition `0` is 0)",
                "{context}: {done}"
            );
        }
        assert_eq!(names, left, "{context}");
        run(
            &mut s,
            "feature.when",
            json!({"feature": "A", "expr": null}),
        );
        assert_eq!(whole(&mut s), on, "{context}: switched on again");
    }
}

/// Item 23 and the developer's decision of 2026-10-26: the colour and the
/// material of an inactive feature apply to the body it passes on only
/// where that body has none of its own (a transform chain, the second
/// transform inactive). The part's own red wins over the inactive
/// transform's blue; the part has no material, so the transform's steel
/// weighs it; given a material of its own, the part's wins. Switched on,
/// the transform's apply again. A face colour given in its body gives way
/// to the part's red the same way.
#[test]
fn an_inactive_feature_gives_its_colour_and_material_only_where_its_source_has_none() {
    let mut s = session();
    new_document(&mut s, 1.0);
    built(
        &mut s,
        "feature.block",
        json!({"size": [20, 20, 10], "name": "Part"}),
    );
    run(
        &mut s,
        "body.appearance",
        json!({"body": "Part", "color": "#c8102e"}),
    );
    built(
        &mut s,
        "feature.transform",
        json!({"source": "Part", "translate": [5, 0, 0], "name": "Moved"}),
    );
    built(
        &mut s,
        "feature.transform",
        json!({"source": "Moved", "angle": "90 deg", "name": "Turned"}),
    );
    run(
        &mut s,
        "body.appearance",
        json!({"body": "Turned", "color": "#2f6fd6"}),
    );
    run(
        &mut s,
        "body.material",
        json!({"body": "Turned", "name": "Steel", "density": 7850}),
    );
    run(
        &mut s,
        "face.appearance",
        json!({"faces": ["F3:copy(0)/F1:face(1)"], "body": "Turned", "color": "#00ff00"}),
    );
    let holder = |s: &mut Session| inspect(s)["bodies"][0].clone();
    let on = holder(&mut s);
    assert_eq!(on["appearance"]["color"], "#2f6fd6", "{on}");
    assert_eq!(on["material"]["name"], "Steel", "{on}");
    assert!(on["material"].get("off").is_none(), "{on}");

    run(
        &mut s,
        "feature.when",
        json!({"feature": "Turned", "expr": "0"}),
    );
    let off = holder(&mut s);
    assert_eq!(off["name"], "Turned", "{off}");
    assert_eq!(off["appearance"]["color"], "#c8102e", "{off}");
    // Expectation changed on 2026-10-26 (the developer's decision, R1
    // item 74): the part has no material, so the inactive transform's
    // steel follows the body it passes on (it had none before).
    assert_eq!(off["material"]["name"], "Steel", "{off}");
    assert_eq!(off["material"]["off"], true, "{off}");
    assert_close(&off["mass"], 7850.0 * 4000.0 * 1e-9, "mass");
    // The face colour given in its body: kept, and not shown.
    let face = run(
        &mut s,
        "face.appearance",
        json!({"faces": ["F3:copy(0)/F1:face(1)"], "body": "Turned", "color": "#00ff00"}),
    );
    assert_eq!(face["faces"][0]["shown"]["color"], "#c8102e", "{face}");

    // The part's own material wins over the inactive transform's.
    run(
        &mut s,
        "body.material",
        json!({"body": "Part", "name": "Aluminium", "density": 2700}),
    );
    assert_eq!(holder(&mut s)["material"]["name"], "Aluminium");
    run(&mut s, "edit.undo", json!({}));

    run(
        &mut s,
        "feature.when",
        json!({"feature": "Turned", "expr": null}),
    );
    assert_eq!(holder(&mut s), on);
    // Suppressed: the same.
    run(&mut s, "feature.suppress", json!({"feature": "Turned"}));
    let suppressed = holder(&mut s);
    assert_eq!(suppressed["appearance"]["color"], "#c8102e");
    assert_eq!(suppressed["material"]["name"], "Steel");
}

/// The colours of the body of `feature` in the scene the pictures draw,
/// face by face as `(name, colour)`.
fn scene_colours(s: &mut Session, feature: &str) -> Vec<(String, Option<[u8; 3]>)> {
    let scene = s.scene(None).unwrap().unwrap();
    let body = scene.bodies.iter().find(|b| b.name == feature).unwrap();
    body.face_names
        .iter()
        .enumerate()
        .map(|(k, name)| {
            let id = body.first_face_id + u32::try_from(k).unwrap();
            (name.clone(), scene.face_colors.get(id))
        })
        .collect()
}

const BLUE: [u8; 3] = [0x2f, 0x6f, 0xd6];
const GREEN: [u8; 3] = [0x14, 0xa0, 0x3c];

/// The CubeSat's spacers (R1 item 74, the developer's decision of
/// 2026-10-26): a material, a colour and a face colour given to a pattern
/// that is switched off follow the body it passes on, its source, which
/// has none of its own: the mass, the centre of mass, the scene, 3MF and
/// OBJ all have them, and nothing is said to be lost (the second
/// instance's face colour waits for the pattern). Given a material of its
/// own, the source's wins while the pattern is off.
#[test]
fn a_switched_off_pattern_passes_its_material_and_colours_on() {
    let dir = workdir("inactive-rule", "pattern-looks");
    let mut s = session_in(&dir);
    new_document(&mut s, 1.0);
    run(&mut s, "variable.set", json!({"name": "on", "expr": "1"}));
    built(
        &mut s,
        "feature.block",
        json!({"size": [10, 10, 10], "name": "Spacer"}),
    );
    built(
        &mut s,
        "feature.pattern",
        json!({"source": "Spacer", "count": 3, "step": [20, 0, 0], "name": "Spacers",
               "when": "on"}),
    );
    run(
        &mut s,
        "body.material",
        json!({"body": "Spacers", "name": "PLA", "density": 1250}),
    );
    run(
        &mut s,
        "body.appearance",
        json!({"body": "Spacers", "color": BLUE}),
    );
    let tops = ["F2:copy(0)/F1:face(1)", "F2:copy(1)/F1:face(1)"];
    run(
        &mut s,
        "face.appearance",
        json!({"faces": tops, "body": "Spacers", "color": GREEN}),
    );
    // The mass and the centre of mass, held by assertions that read the
    // switch (1.25 g a spacer; centred at x = 25 mm, or 5 mm alone).
    run(
        &mut s,
        "assert.add",
        json!({"name": "mass", "measure": "mass", "of": "Spacers",
               "eq": "0.00125 * (1 + 2 * on)"}),
    );
    run(
        &mut s,
        "assert.add",
        json!({"name": "centre", "measure": "centerOfMass", "of": "Spacers", "axis": "x",
               "eq": "5 mm + 20 mm * on"}),
    );
    for on in [1_u8, 0] {
        run(
            &mut s,
            "variable.set",
            json!({"name": "on", "expr": on.to_string()}),
        );
        let context = format!("on = {on}");
        let spacers = inspect(&mut s)["bodies"]
            .as_array()
            .unwrap()
            .iter()
            .find(|b| b["name"] == "Spacers")
            .unwrap()
            .clone();
        assert_eq!(spacers["material"]["name"], "PLA", "{context}: {spacers}");
        assert_close(&spacers["mass"], 0.00125 * f64::from(1 + 2 * on), "mass");
        let check = run(&mut s, "document.check", json!({}));
        assert_eq!(check["ok"], true, "{context}: {check}");
        assert!(check.get("warnings").is_none(), "{context}: {check}");
        assert!(
            inspect(&mut s).get("lostFaces").is_none(),
            "{context}: a face colour waiting for the pattern is not lost"
        );
        // The scene: the tops of the instances named are green, the rest
        // blue.
        let faces = scene_colours(&mut s, "Spacers");
        let green: Vec<&str> = faces
            .iter()
            .filter(|(_, c)| *c == Some(GREEN))
            .map(|(n, _)| n.as_str())
            .collect();
        if on == 1 {
            assert_eq!(green, tops, "{context}: {faces:?}");
        } else {
            assert_eq!(green, [tops[0]], "{context}: {faces:?}");
        }
        assert!(
            faces
                .iter()
                .all(|(_, c)| *c == Some(GREEN) || *c == Some(BLUE)),
            "{context}: {faces:?}"
        );
        // A picture, a 3MF and an OBJ file with the same colours.
        run(
            &mut s,
            "view.render",
            json!({"path": format!("spacers-{on}.png"), "width": 64, "height": 48}),
        );
        run(
            &mut s,
            "file.export",
            json!({"path": format!("spacers-{on}.3mf")}),
        );
        let objects = craft3d_testkit::readers::read_3mf_colors(
            &std::fs::read(dir.join(format!("spacers-{on}.3mf"))).unwrap(),
        )
        .unwrap();
        assert_eq!(objects.len(), 1, "{context}");
        let colours: std::collections::BTreeSet<Option<[u8; 3]>> =
            objects[0].colors.iter().copied().collect();
        assert_eq!(
            colours,
            [Some(BLUE), Some(GREEN)].into_iter().collect(),
            "{context}"
        );
        run(
            &mut s,
            "file.export",
            json!({"path": format!("spacers-{on}.obj")}),
        );
        let library = craft3d_testkit::readers::read_mtl(
            &std::fs::read_to_string(dir.join(format!("spacers-{on}.mtl"))).unwrap(),
        )
        .unwrap();
        let diffuse: std::collections::BTreeSet<[u8; 3]> = library
            .values()
            .filter_map(|m| m.diffuse)
            .map(|d| d.map(|x| (x * 255.0).round() as u8))
            .collect();
        assert_eq!(diffuse, [BLUE, GREEN].into_iter().collect(), "{context}");
    }
    // The spacer's own material wins while the pattern is off: what is
    // passed on is the spacer, made of what it is made of.
    run(
        &mut s,
        "body.material",
        json!({"body": "Spacer", "name": "Aluminium", "density": 2700}),
    );
    let check = run(&mut s, "document.check", json!({}));
    assert_eq!(check["ok"], false, "the mass is the aluminium's: {check}");
    let spacers = inspect(&mut s)["bodies"][0].clone();
    assert_eq!(spacers["material"]["name"], "Aluminium", "{spacers}");
    assert_close(&spacers["mass"], 0.0027, "mass");
    // Switched on, the pattern's material is the nearest again.
    run(&mut s, "variable.set", json!({"name": "on", "expr": "1"}));
    let check = run(&mut s, "document.check", json!({}));
    assert_eq!(check["ok"], true, "{check}");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Two features that are off one after the other, neither source with a
/// colour: the nearest to the body as it comes out gives it (the one whose
/// name it is listed under); one that is on goes first.
#[test]
fn of_several_switched_off_the_nearest_gives_the_colour() {
    let mut s = session();
    new_document(&mut s, 1.0);
    built(
        &mut s,
        "feature.block",
        json!({"size": [10, 10, 10], "name": "Part"}),
    );
    for (name, source, colour) in [("Moved", "Part", "#14a03c"), ("Turned", "Moved", "#2f6fd6")] {
        built(
            &mut s,
            "feature.transform",
            json!({"source": source, "translate": [5, 0, 0], "name": name}),
        );
        run(
            &mut s,
            "body.appearance",
            json!({"body": name, "color": colour}),
        );
    }
    let colour = |s: &mut Session| inspect(s)["bodies"][0]["appearance"]["color"].clone();
    assert_eq!(colour(&mut s), "#2f6fd6");
    run(
        &mut s,
        "feature.when",
        json!({"feature": "Turned", "expr": "0"}),
    );
    assert_eq!(colour(&mut s), "#14a03c", "Moved is on: its green");
    run(
        &mut s,
        "feature.when",
        json!({"feature": "Moved", "expr": "0"}),
    );
    assert_eq!(colour(&mut s), "#2f6fd6", "both off: the nearest, Turned");
    run(
        &mut s,
        "feature.when",
        json!({"feature": "Turned", "expr": null}),
    );
    assert_eq!(colour(&mut s), "#2f6fd6", "Turned on: its own");
}

/// R1 item 57 (the sundial): a face colour on the tool's face of a
/// subtraction that its condition switches off colours nothing while it
/// is off, is not lost (it waits for the cut), and pictures and exports
/// are made.
#[test]
fn a_face_colour_of_a_cut_switched_off_breaks_no_picture() {
    let dir = workdir("inactive-rule", "sundial");
    let mut s = session_in(&dir);
    new_document(&mut s, 1.0);
    run(&mut s, "variable.set", json!({"name": "on", "expr": "1"}));
    built(
        &mut s,
        "feature.block",
        json!({"size": [40, 40, 10], "name": "Plate"}),
    );
    built(
        &mut s,
        "feature.cylinder",
        json!({"radius": 5, "height": 20, "position": [20, 20, 5], "name": "Tool",
               "when": "on"}),
    );
    built(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Plate", "tool": "Tool", "name": "Cut",
               "when": "on"}),
    );
    let wall = "Cut:copy(1)/Tool:face(0)";
    run(
        &mut s,
        "face.appearance",
        json!({"faces": [wall], "body": "Cut", "color": "#7A5638"}),
    );
    let brown = [0x7a, 0x56, 0x38];
    for on in [1_u8, 0, 1] {
        run(
            &mut s,
            "variable.set",
            json!({"name": "on", "expr": on.to_string()}),
        );
        let context = format!("on = {on}");
        run(
            &mut s,
            "view.render",
            json!({"path": format!("sundial-{on}.png"), "width": 200, "height": 150}),
        );
        let scene = s.scene(None).unwrap().unwrap();
        assert!(scene.problems.is_empty(), "{context}: {:?}", scene.problems);
        assert!(scene.notices.is_empty(), "{context}: {:?}", scene.notices);
        let browns = scene_colours(&mut s, "Cut")
            .into_iter()
            .filter(|(_, c)| *c == Some(brown))
            .count();
        assert_eq!(browns, usize::from(on == 1), "{context}");
        assert!(inspect(&mut s).get("lostFaces").is_none(), "{context}");
        let check = run(&mut s, "document.check", json!({}));
        assert_eq!(check["ok"], true, "{context}: {check}");
        assert!(check.get("warnings").is_none(), "{context}: {check}");
        run(&mut s, "file.export", json!({"path": "sundial.3mf"}));
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A mass assertion follows a material that changes with a feature's
/// switch alone: a fillet whose edges are all on a boss that is off
/// passes on its source's very body, on or off, so switching the fillet
/// changes no body, only whose material counts (its own while it is on,
/// the source's while it is off). The assertion is measured again, never
/// taken from before the switch.
#[test]
fn a_mass_assertion_follows_a_material_switched_with_no_body_changed() {
    let mut s = session();
    new_document(&mut s, 1.0);
    run(&mut s, "variable.set", json!({"name": "boss", "expr": "0"}));
    built(
        &mut s,
        "feature.block",
        json!({"size": [20, 20, 10], "name": "Base"}),
    );
    built(
        &mut s,
        "feature.block",
        json!({"size": [5, 5, 5], "position": [5, 5, 10], "name": "Boss", "when": "boss"}),
    );
    built(
        &mut s,
        "feature.boolean",
        json!({"op": "union", "target": "Base", "tool": "Boss", "name": "Joined"}),
    );
    built(
        &mut s,
        "feature.fillet",
        json!({"edges": [["F2:face(0)", "F2:face(2)"]], "radius": 1, "name": "Rounded"}),
    );
    run(
        &mut s,
        "body.material",
        json!({"body": "Base", "name": "Light", "density": 1000}),
    );
    run(
        &mut s,
        "body.material",
        json!({"body": "Rounded", "name": "Heavy", "density": 2000}),
    );
    run(
        &mut s,
        "assert.add",
        json!({"name": "m", "measure": "mass", "of": "Rounded", "min": 0}),
    );
    let measured = |s: &mut Session| {
        run(s, "assert.list", json!({"details": true}))["assertions"][0]["measured"].clone()
    };
    for (when, mass) in [(None, 0.008), (Some("0"), 0.004), (None, 0.008)] {
        run(
            &mut s,
            "feature.when",
            json!({"feature": "Rounded", "expr": when}),
        );
        assert_close(&measured(&mut s), mass, "mass");
        let body = inspect(&mut s)["bodies"][0].clone();
        assert_close(&body["mass"], mass, "inspect's mass");
    }
}
