//! A spur gear drawn from formulas (an involute, the spur-gear Jig's
//! build) with a hub that covers the face its teeth end in: the union of
//! the hub takes every freeform flank, tip and root face of the teeth into
//! account, and holds the search tree of each (`PatchTree`). Each flank is
//! a strip along a fitted curve of 30 or more cubic spans, long the face's
//! width and a hundredth of a millimetre across a span. Quartered evenly,
//! each span became 1024 leaves, 22 MB a face: 2.2 GB for twenty teeth,
//! and the check of the Jig over its range was killed for its memory with
//! a hundred (2026-10-23). The trees now halve a piece only across the way
//! it is long, and an operation's trees are held to a budget together.
//!
//! The shelf of trees is one per process, so this file holds one test: no
//! other test in its process puts trees on it.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::plain::run;
use craft3d_engine::{Session, Workspace};
use serde_json::{Value, json};

/// The spur-gear Jig's build (`3dcraft-jigs/jigs/spur-gear/build.json`)
/// without its publishing, its hub `hub_d` across.
fn gear(hub_d: &str) -> Vec<(&'static str, Value)> {
    vec![
        ("file.new", json!({"units": "mm"})),
        (
            "table.set",
            json!({"name": "modules", "columns": ["size", {"name": "value", "unit": "mm"}],
                   "rows": [["0.5 mm", 0.5], ["0.8 mm", 0.8], ["1 mm", 1], ["1.25 mm", 1.25],
                            ["1.5 mm", 1.5], ["2 mm", 2], ["2.5 mm", 2.5], ["3 mm", 3]]}),
        ),
        (
            "table.set",
            json!({"name": "angles", "columns": ["angle", {"name": "value", "unit": "deg"}],
                   "rows": [["14.5 deg", 14.5], ["20 deg", 20], ["25 deg", 25]]}),
        ),
        ("variable.set", json!({"name": "moduleSize", "expr": "4"})),
        ("variable.set", json!({"name": "teeth", "expr": "20"})),
        ("variable.set", json!({"name": "pressure", "expr": "2"})),
        ("variable.set", json!({"name": "width", "expr": "6 mm"})),
        ("variable.set", json!({"name": "bore", "expr": "5 mm"})),
        ("variable.set", json!({"name": "hub", "expr": "1"})),
        ("variable.set", json!({"name": "hubD", "expr": hub_d})),
        ("variable.set", json!({"name": "hubL", "expr": "6 mm"})),
        (
            "variable.set",
            json!({"name": "module", "expr": "modules[moduleSize].value"}),
        ),
        (
            "variable.set",
            json!({"name": "alpha", "expr": "angles[pressure].value"}),
        ),
        (
            "variable.set",
            json!({"name": "pd", "expr": "module * teeth"}),
        ),
        ("variable.set", json!({"name": "rp", "expr": "pd / 2"})),
        (
            "variable.set",
            json!({"name": "rb", "expr": "rp * cos(alpha)"}),
        ),
        ("variable.set", json!({"name": "ra", "expr": "rp + module"})),
        (
            "variable.set",
            json!({"name": "rf", "expr": "rp - 1.25 * module"}),
        ),
        (
            "variable.set",
            json!({"name": "g", "expr": "-pi / (2 * teeth) - (tan(alpha) - alpha / 1 rad)"}),
        ),
        (
            "variable.set",
            json!({"name": "rs", "expr": "max(rb, rf + 0.1 * module)"}),
        ),
        (
            "variable.set",
            json!({"name": "t0", "expr": "sqrt((rs / rb)^2 - 1)"}),
        ),
        (
            "variable.set",
            json!({"name": "ta", "expr": "sqrt((ra / rb)^2 - 1)"}),
        ),
        (
            "variable.set",
            json!({"name": "phis", "expr": "g + t0 - atan(t0) / 1 rad"}),
        ),
        (
            "variable.set",
            json!({"name": "half", "expr": "pi / teeth"}),
        ),
        (
            "variable.set",
            json!({"name": "thetaA", "expr": "g + ta - atan(ta) / 1 rad"}),
        ),
        ("sketch.create", json!({"name": "Outline"})),
        (
            "sketch.point",
            json!({"sketch": "Outline", "at": ["rf * cos(phis * 1 rad)", "rf * sin(phis * 1 rad)"]}),
        ),
        (
            "sketch.curve",
            json!({"sketch": "Outline", "x": "rb * (cos(g + t) + t * sin(g + t))",
                   "y": "rb * (sin(g + t) - t * cos(g + t))", "t": ["t0", "ta"]}),
        ),
        (
            "sketch.curve",
            json!({"sketch": "Outline", "x": "rb * (cos(g + t) + t * sin(g + t))",
                   "y": "-rb * (sin(g + t) - t * cos(g + t))", "t": ["t0", "ta"]}),
        ),
        (
            "sketch.curve",
            json!({"sketch": "Outline", "x": "ra * cos(t)", "y": "ra * sin(t)",
                   "t": ["thetaA", "-thetaA"], "start": "e3", "end": "e6"}),
        ),
        (
            "sketch.curve",
            json!({"sketch": "Outline", "x": "rf * cos(t)", "y": "rf * sin(t)",
                   "t": ["-phis", "2 * half + phis"]}),
        ),
        (
            "sketch.line",
            json!({"sketch": "Outline", "from": "e1", "to": "e2"}),
        ),
        (
            "sketch.line",
            json!({"sketch": "Outline", "from": "e5", "to": "e9"}),
        ),
        (
            "sketch.pattern",
            json!({"sketch": "Outline", "entities": ["e12", "e4", "e8", "e7", "e13", "e11"],
                   "chain": ["e1", "e10"], "angle": "360 deg / teeth",
                   "center": ["0 mm", "0 mm"], "count": "teeth"}),
        ),
        (
            "feature.extrude",
            json!({"sketch": "Outline", "distance": "width", "name": "Rim"}),
        ),
        (
            "feature.cylinder",
            json!({"name": "Hub", "radius": "hubD / 2", "height": "hubL",
                   "position": ["0 mm", "0 mm", "width"]}),
        ),
        (
            "feature.boolean",
            json!({"op": "union", "target": "Rim", "tool": "Hub", "name": "WithHub"}),
        ),
        (
            "feature.cylinder",
            json!({"name": "BoreTool", "radius": "bore / 2", "height": "width + hubL + 2 mm",
                   "position": ["0 mm", "0 mm", "-1 mm"]}),
        ),
        (
            "feature.boolean",
            json!({"op": "subtract", "target": "WithHub", "tool": "BoreTool", "name": "Gear"}),
        ),
        ("variable.set", json!({"name": "u", "expr": "-g"})),
        (
            "variable.set",
            json!({"name": "iA",
                   "expr": "u * ta^2 / 2 - ta^3 / 3 + ((ta^2 + 1) * atan(ta) / 1 rad - ta) / 2"}),
        ),
        (
            "variable.set",
            json!({"name": "i0",
                   "expr": "u * t0^2 / 2 - t0^3 / 3 + ((t0^2 + 1) * atan(t0) / 1 rad - t0) / 2"}),
        ),
        (
            "variable.set",
            json!({"name": "toothArea",
                   "expr": "(u - (t0 - atan(t0) / 1 rad)) * (rs^2 - rf^2) + 2 * rb^2 * (iA - i0)"}),
        ),
        // The hub, wider than the teeth, stands on the rim's top face: the
        // union is the rim and the hub's cylinder, whose part over the
        // rim's face is the face of the hub's bottom outside the rim.
        (
            "assert.add",
            json!({"name": "volume", "measure": "volume", "of": "Gear",
                   "eq": "(pi * rf^2 + teeth * toothArea) * width + pi * (hubD / 2)^2 * hubL - pi * (bore / 2)^2 * (width + hubL)"}),
        ),
        (
            "assert.add",
            json!({"name": "one body", "measure": "count", "of": {"kind": "body"}, "eq": "1"}),
        ),
    ]
}

#[test]
fn a_hub_over_the_teeth_of_a_formula_gear_holds_small_trees() {
    let mut s = Session::new(Workspace::none());
    let before = craft3d_kernel::work::work().patch_trees;
    for (id, params) in gear("30 mm") {
        run(&mut s, id, params);
    }
    let check = run(&mut s, "document.check", json!({}));
    assert_eq!(check["ok"], true, "{check}");
    assert_eq!(check["assertions"]["passed"], 2, "{check}");
    let built = craft3d_kernel::work::work().patch_trees - before;
    let (trees, bytes) = craft3d_kernel::shelf_size();
    // Before: 160 trees of 6 to 22 MB built, 2.2 GB held by the union,
    // the shelf full (57 trees, 804 MB). Now 80 trees, 68 MB on the shelf
    // (2026-10-23, the Strix).
    assert!(
        bytes <= 128 << 20,
        "{built} trees built, {trees} on the shelf take {bytes} bytes"
    );
}
