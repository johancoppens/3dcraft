//! Motion of assemblies (docs/design/motion.md › *Assemblies*): the
//! timeline `t` drives a joint variable, each moment is a solve carried
//! along the branch from the assembly's rest state, and its parts are
//! placed by transforms. Held to closed forms: a crank-rocker four-bar's
//! coupler point against the intersection of two circles, seek equal to
//! play, and the dead point of a four-bar whose crank cannot go round.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::Portable;
mod common;

use common::plain::run;
use common::{error, session_in, workdir};
use craft3d_engine::Session;
use serde_json::{Value, json};
use std::path::Path;

/// A bar part with a published length, its body `Bar` 4 wide and 2
/// thick, and datum axes `End1` and `End2` 2 in from its ends.
fn bar_part(s: &mut Session) {
    bar_part_of(s, 2.0);
}

/// [`bar_part`] `thickness` thick.
fn bar_part_of(s: &mut Session, thickness: f64) {
    run(s, "file.new", json!({}));
    run(s, "variable.set", json!({ "name": "len", "expr": "24 mm" }));
    run(
        s,
        "variable.publish",
        json!({ "name": "len", "min": "10 mm", "max": "500 mm" }),
    );
    run(
        s,
        "feature.block",
        json!({ "name": "Bar", "size": ["len", 4, thickness] }),
    );
    run(
        s,
        "datum.axis",
        json!({ "name": "End1", "through": [[2, 2, 0], [2, 2, 1]] }),
    );
    run(
        s,
        "datum.axis",
        json!({ "name": "End2", "through": [["len - 2 mm", 2, 0], ["len - 2 mm", 2, 1]] }),
    );
    run(s, "file.save", json!({ "path": "bar.3dcraft" }));
    run(s, "file.close", json!({}));
}

/// The two points where circles about `a` (radius `ra`) and `b` (radius
/// `rb`) meet in the plane.
fn circles(a: [f64; 2], ra: f64, b: [f64; 2], rb: f64) -> Option<[[f64; 2]; 2]> {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let d = (dx * dx + dy * dy).sqrt();
    if d > ra + rb || d < (ra - rb).abs() || d == 0.0 {
        return None;
    }
    let along = (ra * ra - rb * rb + d * d) / (2.0 * d);
    let h = (ra * ra - along * along).max(0.0).sqrt();
    let (ux, uy) = (dx / d, dy / d);
    let m = [a[0] + along * ux, a[1] + along * uy];
    Some([
        [m[0] - h * uy, m[1] + h * ux],
        [m[0] + h * uy, m[1] - h * ux],
    ])
}

/// A four-bar of one bar part: ground pivots at the origin and at
/// (`d`, 0), crank `a`, coupler `b`, rocker `c` (distances between the
/// axes); placed near its elbow-up pose at a crank angle of 0, the crank
/// `Drive` driven by `360 deg * t`.
fn four_bar(s: &mut Session, [a, b, c, d]: [f64; 4]) {
    run(s, "assembly.new", json!({}));
    let crank_tip = [a, 0.0];
    let elbow = circles(crank_tip, b, [d, 0.0], c).expect("the four-bar closes at t = 0")[0];
    // Where a bar of axis distance `len` lies with End1 at `p` and End2 at
    // `q` (its axes are 2 in from its corner at (0, 0)).
    let bar = |name: &str, len: f64, p: [f64; 2], q: [f64; 2]| {
        let phi = (q[1] - p[1]).patan2(q[0] - p[0]);
        let (cs, sn) = (phi.pcos(), phi.psin());
        let position = [
            p[0] - (2.0 * cs - 2.0 * sn),
            p[1] - (2.0 * sn + 2.0 * cs),
            0.0,
        ];
        json!({ "source": "bar.3dcraft", "name": name, "body": "Bar",
                "params": { "len": format!("{} mm", len + 4.0) },
                "position": position, "rotation": [0, 0, phi.to_degrees()] })
    };
    run(s, "instance.add", bar("Ground", d, [0.0, 0.0], [d, 0.0]));
    run(s, "instance.add", bar("Crank", a, [0.0, 0.0], crank_tip));
    run(s, "instance.add", bar("Coupler", b, crank_tip, elbow));
    run(s, "instance.add", bar("Rocker", c, [d, 0.0], elbow));
    for (name, x, y) in [
        ("Base", "world/z", "Ground/End1"),
        ("Drive", "Ground/End1", "Crank/End1"),
        ("Elbow", "Crank/End2", "Coupler/End1"),
        ("Knee", "Coupler/End2", "Rocker/End2"),
        ("Hip", "Ground/End2", "Rocker/End1"),
    ] {
        let kind = if name == "Base" { "fixed" } else { "revolute" };
        run(
            s,
            "mate.add",
            json!({ "kind": kind, "a": x, "b": y, "name": name, "flip": true }),
        );
    }
    run(s, "variable.set", json!({ "name": "t", "expr": "0" }));
    run(
        s,
        "variable.set",
        json!({ "name": "Drive_angle", "expr": "360 deg * t" }),
    );
}

fn xy(v: &Value) -> [f64; 2] {
    [v[0].as_f64().unwrap(), v[1].as_f64().unwrap()]
}

fn session(dir: &Path) -> Session {
    let mut s = session_in(dir);
    bar_part(&mut s);
    s
}

/// The coupler's far axis (Coupler/End2) traced over a turn of the crank
/// is the intersection of the circle about the crank's tip and the circle
/// about the rocker's pivot, on the branch the assembly rests on, at every
/// sample (crank-rocker: 20 + 60 <= 50 + 40, so the crank goes round).
#[test]
fn a_crank_rocker_coupler_point_follows_the_circle_intersections() {
    let dir = workdir("motion-asm", "crank-rocker");
    let mut s = session(&dir);
    let [a, b, c, d] = [20.0, 50.0, 40.0, 60.0];
    four_bar(&mut s, [a, b, c, d]);
    let crank = run(
        &mut s,
        "view.trace",
        json!({ "point": "Crank/End2", "samples": 33, "name": "Tip" }),
    );
    let coupler = run(
        &mut s,
        "view.trace",
        json!({ "point": "Coupler/End2", "samples": 33, "name": "Elbow" }),
    );
    let tips = crank["points"].as_array().unwrap();
    let elbows = coupler["points"].as_array().unwrap();
    assert_eq!(tips.len(), 33);
    // The side of the line from the tip to the rocker's pivot the elbow
    // is on: the branch.
    let side = |tip: [f64; 2], elbow: [f64; 2]| {
        ((d - tip[0]) * (elbow[1] - tip[1]) - (0.0 - tip[1]) * (elbow[0] - tip[0])).signum()
    };
    let mut branch = None;
    for (k, (tip, elbow)) in tips.iter().zip(elbows).enumerate() {
        let t = k as f64 / 32.0;
        let theta = 2.0 * std::f64::consts::PI * t;
        let (tip, elbow) = (xy(tip), xy(elbow));
        let want_tip = [a * theta.pcos(), a * theta.psin()];
        assert!(
            (tip[0] - want_tip[0]).abs() < 1e-6 && (tip[1] - want_tip[1]).abs() < 1e-6,
            "t = {t}: the crank's tip is at {tip:?}, not {want_tip:?}"
        );
        let both = circles(want_tip, b, [d, 0.0], c).unwrap();
        let near = both
            .iter()
            .min_by(|p, q| {
                let e = |r: &[f64; 2]| (r[0] - elbow[0]).phypot(r[1] - elbow[1]);
                e(p).total_cmp(&e(q))
            })
            .unwrap();
        let off = (near[0] - elbow[0]).phypot(near[1] - elbow[1]);
        assert!(
            off < 1e-6,
            "t = {t}: the elbow is {off} off the circles: {elbow:?}"
        );
        let here = side(want_tip, elbow);
        assert_eq!(
            *branch.get_or_insert(here),
            here,
            "t = {t}: the branch changed"
        );
    }
    // The document itself did not move.
    let i = run(&mut s, "assembly.inspect", json!({}));
    assert_eq!(i["solve"]["ok"], true, "{i}");
}

/// A moment is a function of `t` alone: sought directly, or reached by
/// playing toward it, it is the same state, bit for bit.
#[test]
fn seeking_and_playing_show_the_same_state() {
    let dir = workdir("motion-asm", "seek");
    let mut boxes = Vec::new();
    for play in [false, true] {
        let mut s = session(&dir);
        four_bar(&mut s, [20.0, 50.0, 40.0, 60.0]);
        if play {
            for k in 0..=14 {
                run(&mut s, "view.motion", json!({ "t": f64::from(k) * 0.05 }));
                s.scene(None).unwrap();
            }
        }
        let shown = run(&mut s, "view.motion", json!({ "t": 0.7 }));
        assert_eq!(shown["t"], 0.7, "{shown}");
        assert_eq!(shown["rebuilt"].as_array().unwrap().len(), 0, "{shown}");
        assert!(shown["deadPoint"].is_null(), "{shown}");
        let scene = s.scene(None).unwrap().unwrap();
        assert_eq!(scene.bodies.len(), 4);
        let per_body: Vec<_> = scene
            .bodies
            .iter()
            .map(|body| {
                let (lo, hi) = body.mesh.bbox().corners().unwrap();
                (
                    body.name.clone(),
                    lo.x.to_bits(),
                    lo.y.to_bits(),
                    hi.x.to_bits(),
                    hi.y.to_bits(),
                )
            })
            .collect();
        boxes.push(per_body);
        // The rest state comes back with t: null.
        run(&mut s, "view.motion", json!({ "t": null }));
    }
    assert_eq!(boxes[0], boxes[1]);
}

/// The thickness of the shown body `name` (its box along z).
fn shown_thickness(s: &mut Session, name: &str) -> f64 {
    let scene = s.scene(None).unwrap().unwrap();
    let body = scene
        .bodies
        .iter()
        .find(|b| b.name == name)
        .unwrap_or_else(|| panic!("no shown body {name}"));
    let (lo, hi) = body.mesh.bbox().corners().unwrap();
    hi.z - lo.z
}

/// The moment the view shows is made again when the sources are read
/// again (`assembly.reload` changes what is placed without a new document
/// state): a thicker bar shows thicker at the same t.
#[test]
fn the_motion_shown_follows_a_reloaded_source() {
    let dir = workdir("motion-asm", "reload");
    let mut s = session(&dir);
    four_bar(&mut s, [20.0, 50.0, 40.0, 60.0]);
    run(&mut s, "view.motion", json!({ "t": 0.1 }));
    assert!((shown_thickness(&mut s, "Crank") - 2.0).abs() < 1e-6);
    // The source is changed by another session, as an editor would.
    let mut other = session_in(&dir);
    bar_part_of(&mut other, 5.0);
    let reloaded = run(&mut s, "assembly.reload", json!({}));
    assert!(
        !reloaded["changed"].as_array().is_some_and(Vec::is_empty),
        "{reloaded}"
    );
    let shown = run(&mut s, "view.motion", json!({}));
    assert_eq!(shown["t"], 0.1, "{shown}");
    let thick = shown_thickness(&mut s, "Crank");
    assert!(
        (thick - 5.0).abs() < 1e-6,
        "the moment shown is stale: {thick}"
    );
}

/// A four-bar whose crank cannot go round (40 + 60 > 30 + 40) stops at
/// its dead point, where coupler and rocker lie in one line:
/// |B − D| = b + c, cos θ = (a² + d² − (b + c)²) / (2ad).
#[test]
fn a_dead_point_is_reported_with_its_t() {
    let dir = workdir("motion-asm", "dead");
    let mut s = session(&dir);
    let [a, b, c, d] = [40.0_f64, 30.0, 40.0, 60.0];
    four_bar(&mut s, [a, b, c, d]);
    let theta = ((a * a + d * d - (b + c) * (b + c)) / (2.0 * a * d)).pacos();
    let t_dead = theta / (2.0 * std::f64::consts::PI);
    let checked = run(
        &mut s,
        "motion.check",
        json!({ "measure": { "measure": "volume", "of": "Crank", "min": "1 mm^3" }, "samples": 65 }),
    );
    let dead = checked["deadPoints"].as_array().unwrap();
    assert!(!dead.is_empty(), "{checked}");
    let at = dead[0]["t"].as_f64().unwrap();
    assert!(
        at <= t_dead + 1e-9 && at > t_dead - 1.0 / 128.0,
        "the dead point is at t = {t_dead}, reported at {at}: {checked}"
    );
    // view.motion past it shows where the motion stopped, and says so.
    let shown = run(&mut s, "view.motion", json!({ "t": 0.5 }));
    assert_eq!(shown["deadPoint"]["t"].as_f64().unwrap(), at, "{shown}");
    // Before it, nothing stops.
    let shown = run(&mut s, "view.motion", json!({ "t": 0.1 }));
    assert!(shown["deadPoint"].is_null(), "{shown}");
}

/// Collisions, the video and the refusals of an assembly's motion.
#[test]
fn collisions_replay_and_refusals_of_an_assembly_motion() {
    let dir = workdir("motion-asm", "collide");
    let mut s = session(&dir);
    four_bar(&mut s, [20.0, 50.0, 40.0, 60.0]);
    // The bars lie in one plane and overlap at their joints: each joint's
    // two bars collide, named by their instances.
    let shown = run(
        &mut s,
        "view.motion",
        json!({ "t": 0.25, "collisions": true }),
    );
    let pairs = shown["collisions"].as_array().unwrap();
    assert!(!pairs.is_empty(), "{shown}");
    let names: Vec<String> = pairs
        .iter()
        .flat_map(|p| {
            [
                p["aName"].as_str().unwrap().to_string(),
                p["bName"].as_str().unwrap().to_string(),
            ]
        })
        .collect();
    assert!(names.iter().any(|n| n == "Crank"), "{names:?}");
    let motion = s.motion().unwrap();
    assert!(!motion.collisions.is_empty());
    run(
        &mut s,
        "view.motion",
        json!({ "t": null, "collisions": false }),
    );
    let out = run(
        &mut s,
        "view.replay",
        json!({ "dir": "frames", "width": 64, "height": 48, "motion": { "frames": 3 } }),
    );
    assert_eq!(out["frames"], 3, "{out}");
    // A point not named through an instance, a feature given, a sketch
    // point: refused by name.
    let e = error(&mut s, "view.trace", json!({ "point": "Nobody/End1" }));
    assert!(e.contains("Nobody"), "{e}");
    let e = error(
        &mut s,
        "view.trace",
        json!({ "point": "Crank/End2", "feature": "Crank" }),
    );
    assert!(e.contains("through its instance"), "{e}");
    // Without a timeline: the way out.
    run(&mut s, "variable.delete", json!({ "name": "Drive_angle" }));
    run(&mut s, "variable.delete", json!({ "name": "t" }));
    let e = error(&mut s, "view.motion", json!({ "t": 0.5 }));
    assert!(e.contains("timeline"), "{e}");
}

/// A-042 in an assembly: a ring part placed wholly inside a plate part
/// overlaps it, so a clearance with a minimum fails over the motion (the
/// assembly's measures are the part's, `measure.clearance`'s code).
#[test]
fn a_ring_enclosed_in_a_plate_fails_a_clearance_over_the_motion() {
    let dir = workdir("motion-asm", "enclosed-ring");
    let mut s = session_in(&dir);
    let base = json!({ "name": "Base", "through": [[0, 0, 0], [1, 0, 0], [0, 1, 0]] });
    run(&mut s, "file.new", json!({ "units": "mm" }));
    run(
        &mut s,
        "feature.block",
        json!({ "name": "Plate", "size": [40, 40, 10], "position": [-20, -20, 0] }),
    );
    run(&mut s, "datum.plane", base.clone());
    run(&mut s, "file.save", json!({ "path": "plate.3dcraft" }));
    run(&mut s, "file.close", json!({}));
    // A ring about z at z = 5 in its part, of radii 14.4 and 1.75 mm.
    run(&mut s, "file.new", json!({ "units": "mm" }));
    run(
        &mut s,
        "feature.torus",
        json!({ "name": "Ring", "major": 14.4, "minor": 1.75, "position": [0, 0, 5] }),
    );
    run(&mut s, "datum.plane", base);
    run(&mut s, "file.save", json!({ "path": "ring.3dcraft" }));
    run(&mut s, "file.close", json!({}));
    run(&mut s, "assembly.new", json!({ "units": "mm" }));
    run(&mut s, "variable.set", json!({ "name": "t", "expr": "0" }));
    run(
        &mut s,
        "instance.add",
        json!({ "source": "plate.3dcraft", "name": "Plate" }),
    );
    run(
        &mut s,
        "instance.add",
        json!({ "source": "ring.3dcraft", "name": "Ring" }),
    );
    run(
        &mut s,
        "mate.add",
        json!({ "kind": "fixed", "name": "Down", "a": "world/xy", "b": "Plate/Base", "flip": true }),
    );
    // The ring's base on the plate's: the ring at z from 3.25 to 6.75
    // and radii 12.65 to 16.15, all inside the plate's 40 × 40 × 10.
    run(
        &mut s,
        "mate.add",
        json!({ "kind": "fixed", "name": "In", "a": "Plate/Base", "b": "Ring/Base", "flip": true }),
    );
    let inspected = run(&mut s, "assembly.inspect", json!({ "measures": true }));
    let ring = inspected["instances"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["name"] == "Ring")
        .unwrap()
        .clone();
    let lo = ring["measured"]["box"][0][2].as_f64().unwrap();
    let hi = ring["measured"]["box"][1][2].as_f64().unwrap();
    assert!(
        lo > 0.0 && hi < 10.0,
        "the ring lies within the plate's height: {ring}"
    );
    let out = run(
        &mut s,
        "motion.check",
        json!({ "measure": { "measure": "clearance", "from": "Plate", "to": "Ring", "min": "0 mm" }, "samples": 2 }),
    );
    assert_eq!(out["ok"], false, "{out}");
    assert_eq!(out["assertions"][0]["status"], "failed", "{out}");
}
