//! What a user gets from the booleans of a plate far from the origin, at
//! a size in the millions (a document in micrometres, or in map
//! coordinates): the plate of `crates/kernel/tests/hunt_sheet_plates.rs`
//! case 123, where the point classifier was found wrong (2026-11-14), by
//! the commands a user calls. A rounded rectangle 54.667 by 20.261 with
//! corners of radius 10.1262 (its short sides 0.0086 long), 6.286 thick
//! about its sketch, drawn on a datum plane turned and some 2.6e8 from
//! the origin at scale 1e6 (`datum.plane` through three points,
//! `sketch.arc`, `sketch.line`, `feature.fill`, `feature.plate`), and a
//! ball of radius 3 about the middle of a short side united with it,
//! taken out of it and intersected with it (`feature.sphere`,
//! `feature.boolean`).
//!
//! In that frame the plate's short sides are ruled splines 6.286e6
//! across, where the search for a nearest point ended hundreds of
//! tolerances short (`crates/kernel/src/boolean/freeform.rs` ›
//! `descend`). The three booleans are held to the two volume identities
//! and to the same part at scale 1; so is a plate in the xy plane whose
//! outline has a spline, 8 thick, and a block through that rim.
//!
//! What triggers the fault is the ratio of a face's two tangents (from
//! about 2e6), not the size: a part of ordinary size meets it when its
//! wall is a B-spline whose knots are stretched by millions, as a file
//! from another modeller can bring. The last test imports such a pin
//! (`file.import` of a STEP file it writes, r 11.3412 and h 32.2413 mm,
//! its wall the exact rational B-spline of the cylinder on knots of u
//! times 3e7) and puts a block through its side (`feature.block`,
//! `feature.boolean`): before the repair the subtraction gave a valid
//! body of the pin's own volume, 13028.08 mm³ for 12060.03, and the union
//! and the intersection were refused (the review of 2026-11-15).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::plain::{body, new_document, run, session_in, workdir};
use craft3d_engine::Session;
use serde_json::{Value, json};

/// A command that is to build: its result, or its error.
fn step(s: &mut Session, id: &str, params: Value) -> Result<Value, String> {
    let result = s.execute(id, &params).map_err(|e| format!("{id}: {e}"))?;
    if result["status"] == "ok" {
        Ok(result)
    } else {
        Err(format!("{id}: {}", result["error"]))
    }
}

/// The sketch `Outline` filled and plated `thickness` thick on `side`,
/// then `tool` (a command and its parameters, the body named `Tool`)
/// united with the plate, intersected with it and taken out of it, each
/// in a document of its own made by `draw`: the volumes of the plate, the
/// tool and the three results, or the error of the first command that
/// failed.
fn volumes(
    k: f64,
    name: &str,
    draw: &dyn Fn(&mut Session),
    (thickness, side): (f64, &str),
    tool: (&str, Value),
) -> Result<[f64; 5], String> {
    let mut found = [0.0; 5];
    for (index, op) in ["union", "intersect", "subtract"].into_iter().enumerate() {
        let dir = workdir("classify-far", &format!("{name}-{k}-{op}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        draw(&mut s);
        step(
            &mut s,
            "feature.fill",
            json!({"sketch": "Outline", "name": "Sheet"}),
        )?;
        let face = body(&mut s, "Sheet")["faceNames"][0]
            .as_str()
            .unwrap()
            .to_string();
        step(
            &mut s,
            "feature.plate",
            json!({"faces": [face], "thickness": format!("{thickness} mm"), "side": side, "name": "Plate"}),
        )?;
        found[0] = body(&mut s, "Plate")["volume"].as_f64().unwrap();
        step(&mut s, tool.0, tool.1.clone())?;
        found[1] = body(&mut s, "Tool")["volume"].as_f64().unwrap();
        step(
            &mut s,
            "feature.boolean",
            json!({"op": op, "target": "Plate", "tool": "Tool", "name": "Result"}),
        )?;
        let result = body(&mut s, "Result");
        if result["valid"] != true {
            return Err(format!("{op}: the result is not valid: {result}"));
        }
        found[2 + index] = result["volume"].as_f64().unwrap();
    }
    Ok(found)
}

/// The tool passes through the plate's rim, and the three booleans meet
/// the two identities within a billionth of the plate's volume.
fn hold(what: &str, v: &[f64; 5]) {
    let [plate, tool, union, intersection, subtraction] = *v;
    let slack = 1e-9 * plate;
    assert!(
        intersection > 1e-3 * tool && intersection < 0.999 * tool,
        "{what}: the tool is to pass through the rim: {v:?}"
    );
    assert!(
        (union + intersection - plate - tool).abs() <= slack,
        "{what}: union {union:e} + intersection {intersection:e} is not plate {plate:e} + tool {tool:e}"
    );
    assert!(
        (subtraction + intersection - plate).abs() <= slack,
        "{what}: subtraction {subtraction:e} + intersection {intersection:e} is not the plate {plate:e}"
    );
}

/// The volumes at scale 1e6 are those at scale 1 times 1e18.
fn same_at_any_size(small: &[f64; 5], large: &[f64; 5]) {
    for (a, b) in small.iter().zip(large) {
        assert!(
            (b / 1e18 - a).abs() <= 1e-9 * small[0],
            "at scale 1e6 {large:?} against {small:?} at scale 1"
        );
    }
}

/// The plate of the finding and a ball about the middle of a short side,
/// at scale `k`.
fn far_plate(k: f64) -> Result<[f64; 5], String> {
    // The frame of the finding at scale 1e6: the middle of the plate, its
    // long axis, and the direction of its short sides.
    let long = [-0.83477, -0.48978, -0.25157];
    let short = [-0.50006, 0.48315, 0.71868];
    let middle = [-23.899e6, -156.750e6, -162.736e6].map(|c: f64| c * k / 1e6);
    let along = |d: [f64; 3], reach: f64| [0, 1, 2].map(|i| middle[i] + d[i] * reach * k);
    let (w, h, r) = (54.667, 20.261, 10.1262);
    let draw = |s: &mut Session| {
        run(
            s,
            "datum.plane",
            json!({"through": [middle, along(long, 100.0), along(short, 100.0)], "name": "Far"}),
        );
        run(
            s,
            "sketch.create",
            json!({"plane": "Far", "name": "Outline"}),
        );
        let at = |x: f64, y: f64| [x * k, y * k];
        let (x, y) = (0.5 * w, 0.5 * h);
        // Counter-clockwise from the lower right corner: an arc, a line.
        let centres = [
            (x - r, -y + r),
            (x - r, y - r),
            (-x + r, y - r),
            (-x + r, -y + r),
        ];
        let from = [(x - r, -y), (x, y - r), (-x + r, y), (-x, -y + r)];
        let to = [(x, -y + r), (x - r, y), (-x, y - r), (-x + r, -y)];
        for i in 0..4 {
            run(
                s,
                "sketch.arc",
                json!({"sketch": "Outline", "center": at(centres[i].0, centres[i].1),
                    "start": at(from[i].0, from[i].1), "end": at(to[i].0, to[i].1)}),
            );
            let next = from[(i + 1) % 4];
            run(
                s,
                "sketch.line",
                json!({"sketch": "Outline", "from": at(to[i].0, to[i].1), "to": at(next.0, next.1)}),
            );
        }
    };
    // About the middle of the short side at the end of the long axis,
    // within the plate's thickness.
    let ball = json!({"radius": 3.0 * k, "position": along(long, 0.5 * w), "name": "Tool"});
    volumes(
        k,
        "far",
        &draw,
        (6.286 * k, "mid"),
        ("feature.sphere", ball),
    )
}

#[test]
fn the_plate_of_the_finding_and_a_ball_through_its_short_side() {
    let small = far_plate(1.0).unwrap();
    hold("at scale 1", &small);
    let large = far_plate(1e6).unwrap_or_else(|e| panic!("at scale 1e6: {e}"));
    hold("at scale 1e6", &large);
    same_at_any_size(&small, &large);
}

/// A plate in the xy plane with a spline in its outline, 8 thick, and a
/// block through that rim, at scale `k`.
fn spline_plate(k: f64) -> Result<[f64; 5], String> {
    let draw = |s: &mut Session| {
        run(
            s,
            "sketch.create",
            json!({"plane": "xy", "name": "Outline"}),
        );
        let at = |x: f64, y: f64| [x * k, y * k];
        for (from, to) in [
            (at(0.0, 0.0), at(60.0, 0.0)),
            (at(60.0, 0.0), at(60.0, 30.0)),
            (at(0.0, 30.0), at(0.0, 0.0)),
        ] {
            run(
                s,
                "sketch.line",
                json!({"sketch": "Outline", "from": from, "to": to}),
            );
        }
        run(
            s,
            "sketch.spline",
            json!({"sketch": "Outline", "through": [
                at(60.0, 30.0), at(45.0, 36.0), at(30.0, 31.0), at(15.0, 37.0), at(0.0, 30.0)
            ]}),
        );
    };
    let block = json!({
        "size": [10.0 * k, 20.0 * k, 4.0 * k],
        "position": [25.0 * k, 25.0 * k, 2.0 * k],
        "name": "Tool"
    });
    volumes(
        k,
        "spline",
        &draw,
        (8.0 * k, "out"),
        ("feature.block", block),
    )
}

#[test]
fn a_plate_with_a_spline_rim_and_a_block_through_it() {
    let small = spline_plate(1.0).unwrap();
    hold("at scale 1", &small);
    let large = spline_plate(1e6).unwrap_or_else(|e| panic!("at scale 1e6: {e}"));
    hold("at scale 1e6", &large);
    same_at_any_size(&small, &large);
}

// ---------------------------------------------------------------------
// A STEP pin whose wall is a B-spline on stretched knots
// ---------------------------------------------------------------------

/// The circle of radius 1 as a rational quadratic B-spline: nine control
/// points `(x, y, weight)` on [`RING_KNOTS`].
const RING: [(f64, f64, f64); 9] = [
    (1.0, 0.0, 1.0),
    (1.0, 1.0, std::f64::consts::FRAC_1_SQRT_2),
    (0.0, 1.0, 1.0),
    (-1.0, 1.0, std::f64::consts::FRAC_1_SQRT_2),
    (-1.0, 0.0, 1.0),
    (-1.0, -1.0, std::f64::consts::FRAC_1_SQRT_2),
    (0.0, -1.0, 1.0),
    (1.0, -1.0, std::f64::consts::FRAC_1_SQRT_2),
    (1.0, 0.0, 1.0),
];
const RING_KNOTS: [f64; 12] = [
    0.0, 0.0, 0.0, 0.25, 0.25, 0.5, 0.5, 0.75, 0.75, 1.0, 1.0, 1.0,
];

/// The pin of the review, in millimetres: radius and height.
const PIN: (f64, f64) = (11.3412, 32.2413);

/// The wall of a pin of radius `r` and height `h` standing on the xy
/// plane about z: the exact rational B-spline of the cylinder, u around
/// (its knots times `pu`) and v up (its knots times `pv`).
fn wall(r: f64, h: f64, (pu, pv): (f64, f64)) -> craft3d_geom::Surface {
    let mut ctrl = Vec::new();
    let mut weights = Vec::new();
    for (x, y, weight) in RING {
        for z in [0.0, h] {
            ctrl.push(craft3d_math::Vec3::new(r * x, r * y, z));
            weights.push(weight);
        }
    }
    craft3d_geom::Surface::Nurbs(
        craft3d_geom::NurbsSurface::new(
            2,
            1,
            RING_KNOTS.iter().map(|t| t * pu).collect(),
            vec![0.0, 0.0, pv, pv],
            9,
            2,
            ctrl,
            weights,
        )
        .unwrap(),
    )
}

/// The pin with that wall written to `dir/pin.step`, as a modeller that
/// keeps its walls as B-splines writes such a part.
fn write_pin(dir: &std::path::Path, (r, h): (f64, f64), knots: (f64, f64)) {
    let mut pin = craft3d_kernel::primitives::cylinder(&craft3d_math::Frame::WORLD, r, h).unwrap();
    let surface = wall(r, h, knots);
    for face in &mut pin.faces {
        if face.surface.kind() == "cylinder" {
            face.surface = surface.clone();
        }
    }
    let (bytes, _) = craft3d_io::export_step(
        &[craft3d_io::StepBody::new("Pin", &pin)],
        &craft3d_io::StepOptions::default(),
    )
    .unwrap();
    std::fs::write(dir.join("pin.step"), bytes).unwrap();
}

/// The block through the pin's side: `2r` along x from the axis out,
/// `0.8r` across, from three tenths of the height to six tenths.
fn block_through(r: f64, h: f64) -> Value {
    let w = 0.4 * r;
    json!({"size": [2.0 * r, 2.0 * w, 0.3 * h], "position": [0.0, -w, 0.3 * h], "name": "Tool"})
}

/// The volumes of the pin, the block and what they share, in closed form.
fn pin_volumes(r: f64, h: f64) -> (f64, f64, f64) {
    use craft3d_math::Portable;
    let (w, dz) = (0.4 * r, 0.3 * h);
    (
        std::f64::consts::PI * r * r * h,
        2.0 * r * 2.0 * w * dz,
        dz * (w * (r * r - w * w).sqrt() + r * r * (w / r).pasin()),
    )
}

/// The pin on knots stretched by `knots`, imported into a new document
/// in millimetres at the default tolerance, and `op` of it and the block
/// through its side: the pin's volume, the block's and the result.
fn imported_pin(knots: (f64, f64), op: &str) -> Result<(f64, f64, Value), String> {
    let dir = workdir(
        "classify-far-pin",
        &format!("{:e}-{:e}-{op}", knots.0, knots.1),
    );
    write_pin(&dir, PIN, knots);
    let mut s = session_in(&dir);
    run(&mut s, "file.new", json!({"units": "mm"}));
    s.execute("file.import", &json!({"path": "pin.step"}))
        .map_err(|e| format!("file.import: {e}"))?;
    let document = run(&mut s, "document.inspect", json!({}));
    let pin = document["bodies"][0]["name"]
        .as_str()
        .ok_or_else(|| format!("no body imported: {document}"))?
        .to_string();
    let pin_volume = body(&mut s, &pin)["volume"].as_f64().unwrap();
    step(&mut s, "feature.block", block_through(PIN.0, PIN.1))?;
    let block_volume = body(&mut s, "Tool")["volume"].as_f64().unwrap();
    step(
        &mut s,
        "feature.boolean",
        json!({"op": op, "target": pin, "tool": "Tool", "name": "Result"}),
    )?;
    Ok((pin_volume, block_volume, body(&mut s, "Result")))
}

/// A pin imported from a STEP file whose wall is the exact rational
/// B-spline of its cylinder, and a block through its side: union,
/// intersection and subtraction are their closed forms whether the
/// wall's knots are even or stretched (those of u times 3e7 or 1e9, those
/// of v times 1e-7), and the stretched files give the even file's bodies.
/// On the stretched knots one tangent of the wall is millions of times
/// the other, where the search for a nearest point ended short (2026-11-14,
/// `crates/kernel/src/boolean/freeform.rs` › `descend`): before the repair
/// the subtraction was a valid body of 13028.08 mm³ for 12060.03, the pin
/// untouched, and the union ("the result passes through itself") and the
/// intersection ("the result is empty") were refused, on each of the three
/// stretches (the review's Check 43669015955 on `release/0.1`, with the
/// same file); the even file was right.
#[test]
fn a_block_through_an_imported_pin_whose_wall_is_on_stretched_knots() {
    let (pin, block, shared) = pin_volumes(PIN.0, PIN.1);
    let closed = [pin + block - shared, shared, pin - shared];
    let ops = ["union", "intersect", "subtract"];
    let mut even: Vec<Value> = Vec::new();
    for knots in [(1.0, 1.0), (3e7, 1.0), (1e9, 1.0), (1.0, 1e-7)] {
        for (index, op) in ops.into_iter().enumerate() {
            let what = format!(
                "{op}, the knots of u times {:e} and of v {:e}",
                knots.0, knots.1
            );
            let (a, b, result) = imported_pin(knots, op).unwrap_or_else(|e| panic!("{what}: {e}"));
            assert!(
                (a - pin).abs() <= 1e-9 * pin,
                "{what}: the pin {a} for {pin}"
            );
            assert!(
                (b - block).abs() <= 1e-9 * block,
                "{what}: the block {b} for {block}"
            );
            assert_eq!(result["valid"], true, "{what}: {result}");
            let volume = result["volume"].as_f64().unwrap();
            assert!(
                (volume - closed[index]).abs() <= 1e-9 * pin,
                "{what}: {volume} for {}",
                closed[index]
            );
            match even.get(index) {
                None => even.push(result),
                Some(twin) => {
                    for key in ["faces", "edges", "vertices", "shells", "faceNames"] {
                        assert_eq!(result[key], twin[key], "{what}: {key}");
                    }
                    for key in ["volume", "area"] {
                        let (here, there) =
                            (result[key].as_f64().unwrap(), twin[key].as_f64().unwrap());
                        assert!(
                            (here - there).abs() <= 1e-12 * there,
                            "{what}: {key} {here} where the even file's is {there}"
                        );
                    }
                }
            }
        }
    }
}
