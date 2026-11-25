//! Random design 114519 through its commands: a shell whose hollow's floor
//! comes tangent to its hollowed rod, mirrored in the plane through the
//! rod's axis and combined with its mirror image (docs/design/booleans.md ›
//! *Two operands' edges placed apart on one tangency line*; the kernel's
//! reconstruction, at more thicknesses and turned, is
//! `crates/kernel/tests/boolean_mirrored_tangency.rs`).
//!
//! On `main` until 2026-11-25 the union, the difference and the common
//! part were refused as *"boolean produced an invalid body"* at every
//! scale: each floor's edge lies on the other's rod face, a little off the
//! tangency, and nothing divided that face there. Held to here, at scales
//! 1e-3, 1, 1e3 and 1e6 (the document's tolerance a millionth of the
//! scale): each of the three built and through the deep check, and the
//! identities of sets between them and the shell (the mirror image has
//! the shell's volume): `V(A ∪ B) + V(A ∩ B) = 2 V(A)` and
//! `V(A \ B) = V(A) − V(A ∩ B)`, within the tolerance times the shell's
//! area.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::print_stdout
)]

use craft3d_engine::{Session, Workspace};
use serde_json::{Value, json};

/// The design at scale `s`, up to the shell, and the boolean `op` of the
/// shell's mirror image with the shell where `op` is given.
fn steps(s: f64, op: Option<&str>) -> Vec<(&'static str, Value)> {
    let mut steps = vec![
        ("file.new", json!({"units": "mm", "tolerance": 1e-6 * s})),
        (
            "feature.block",
            json!({"name": "N0", "position": [0, 5.0 * s, -7.5 * s], "size": [2.5 * s, 15.0 * s, 7.5 * s]}),
        ),
        (
            "feature.sphere",
            json!({"name": "N1", "position": [5.0 * s, 10.0 * s, -2.5 * s], "radius": 5.0 * s}),
        ),
        (
            "feature.boolean",
            json!({"name": "N3", "op": "subtract", "target": "N1", "tool": "N0"}),
        ),
        (
            "sketch.create",
            json!({"face": "F3:copy(1)/F1:face(5)", "name": "S5"}),
        ),
        (
            "sketch.circle",
            json!({"center": [2.5 * s, -5.0 * s], "radius": 5.0 * s, "sketch": "S5"}),
        ),
        (
            "feature.extrude",
            json!({"distance": 12.5 * s, "name": "N5", "sketch": "S5"}),
        ),
        (
            "feature.boolean",
            json!({"name": "N6", "op": "union", "target": "N5", "tool": "N3"}),
        ),
        (
            "feature.mirror",
            json!({"name": "N7", "source": "N6", "normal": [0, 0, 1], "point": [0, 0, 5.0 * s], "keep": true}),
        ),
        (
            "feature.shell",
            json!({"name": "N8", "source": "N7", "thickness": 1.25 * s, "open": ["F7:copy(0)/F6:copy(1,1)/F1:face(5)", "F7:copy(0)/F6:copy(1)/F2:face(0)"]}),
        ),
    ];
    if let Some(op) = op {
        steps.push((
            "feature.mirror",
            json!({"name": "N9", "source": "N8", "normal": [0, 1, 0], "point": [0, 7.5 * s, 0], "keep": true}),
        ));
        steps.push((
            "feature.boolean",
            json!({"name": "N10", "op": op, "target": "N9", "tool": "N8"}),
        ));
    }
    steps
}

/// The steps run: the volume and area of the body of `feature`, or why a
/// step failed or the deep check did not pass.
fn built(steps: &[(&str, Value)], feature: &str) -> Result<(f64, f64), String> {
    let mut s = Session::new(Workspace::unrestricted());
    for (id, params) in steps {
        let made = s.execute(id, params).unwrap();
        if made["status"] == "failed" {
            return Err(format!("{id}: {}", made["error"]));
        }
    }
    let check = s.execute("document.check", &json!({"deep": true})).unwrap();
    if check["ok"] != true {
        return Err(format!("the deep check: {check}"));
    }
    let measured = s
        .execute(
            "document.measure",
            &json!({"feature": feature, "inertia": false}),
        )
        .unwrap();
    match (measured["volume"].as_f64(), measured["area"].as_f64()) {
        (Some(volume), Some(area)) => Ok((volume, area)),
        _ => Err(format!("not measured: {measured}")),
    }
}

#[test]
fn the_designs_shell_and_its_mirror_image_through_the_commands() {
    let mut wrong = Vec::new();
    for s in [1e-3, 1.0, 1e3, 1e6] {
        let shell = built(&steps(s, None), "N8");
        let mut volumes = Vec::new();
        for op in ["union", "subtract", "intersect"] {
            let got = built(&steps(s, Some(op)), "N10");
            println!("114519 s={s:e} {op}: {got:?}");
            match got {
                Ok((volume, _)) => volumes.push(volume),
                Err(why) => wrong.push(format!("s={s:e} {op}: {why}")),
            }
        }
        println!("114519 s={s:e} shell: {shell:?}");
        let (Ok((a, area)), [union, difference, common]) = (shell, volumes.as_slice()) else {
            wrong.push(format!("s={s:e}: the shell or a boolean not built"));
            continue;
        };
        let allowed = 1e-6 * s * area;
        for (name, off) in [
            ("union + common - 2 shell", union + common - 2.0 * a),
            ("difference - (shell - common)", difference - (a - common)),
        ] {
            if off.is_nan() || off.abs() > allowed {
                wrong.push(format!("s={s:e}: {name} = {off:e}, allowed {allowed:e}"));
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}
