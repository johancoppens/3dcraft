//! A seeded hunt of what `measure.thickness` says (the thinnest place by
//! the ray method, and `atLeast`, proven: no place is thinner), against
//! closed forms of the test's own: plates (thickness t, their faces
//! parallel only to rounding once turned off the axes), shells of balls
//! and tubes, concentric and off-centre (R − r − e along the line of the
//! centres), a block with a blind hole near a wall (the wall: the distance
//! from the axis less the radius), a tapered plate measured from its flat
//! face (the thin end), and a strip between a spline wall and the same
//! wall moved up, measured from the lower wall (rays cast by the test's
//! own root finding); each turned about a random axis and moved far from
//! the origin, at scales 1e-3 to 1e6.
//!
//! The oracles (docs/design/verification.md › *Wall thickness*, *The
//! proven thickness*): `atLeast` is never above the true least thickness
//! (beyond 1e-13 of the distance from the origin); the thickness found is
//! a ray at a real place, so never below the truth by more than the
//! tolerance; where the walls are parallel or concentric (plates, shells,
//! tubes, the wall by a hole) the command says it is exact, and it is held
//! to twice the tolerance there; elsewhere (the tapered plate, the spline
//! strip) only the lower side is held, the found value being a searched
//! least.
//!
//! The gate runs a fixed set per kind; `-- --ignored` the wide sweeps
//! (`HUNT_CASES` per kind, default 40: a thin shell's proof takes seconds).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod hunt_checks_common;

use craft3d_math::Portable;
use hunt_checks_common::*;
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Plate,
    BallShell,
    Tube,
    HoleByWall,
    Tapered,
    SplineStrip,
    /// A cone less the same cone moved down its axis: two parallel cone
    /// faces, measured from the inner one.
    ConeShell,
    /// A ring less a thinner ring, its tube's circle the same or wider:
    /// r1 − r2 − e thick.
    TorusShell,
}

const SCALES: [f64; 5] = [1e-3, 1.0, 1.0, 1e3, 1e6];

struct Case {
    id: String,
    what: String,
    k: f64,
    steps: Vec<Step>,
    /// The true least thickness.
    truth: f64,
    /// The command says it is exact here (parallel or concentric walls).
    exact: bool,
    /// Measured from the face of the body `A` whose outward normal, in
    /// the case's own axes, is this (before the motion), or (a zero
    /// vector) the face the last spline sketched sweeps; else the body.
    from_face: Option<V>,
    /// Or measured from the faces this query finds.
    query: Option<Value>,
    motion: Motion,
}

fn block(name: &str, size: V, at: V) -> Step {
    (
        "feature.block",
        json!({"name": name, "size": mm3(size), "position": mm3(at)}),
    )
}

fn sphere(name: &str, r: f64, c: V) -> Step {
    (
        "feature.sphere",
        json!({"name": name, "radius": mm(r), "position": mm3(c)}),
    )
}

fn cylinder(name: &str, r: f64, h: f64, at: V) -> Step {
    (
        "feature.cylinder",
        json!({"name": name, "radius": mm(r), "height": mm(h), "position": mm3(at)}),
    )
}

fn subtract(name: &str, target: &str, tool: &str) -> Step {
    (
        "feature.boolean",
        json!({"op": "subtract", "target": target, "tool": tool, "name": name}),
    )
}

fn case(rng: &mut Rng, kind: Kind, index: usize) -> Option<Case> {
    let k = rng.pick(&SCALES);
    let motion = Motion::random(rng, k);
    let mut steps = Vec::new();
    let mut exact = true;
    let mut from_face = None;
    let mut query = None;
    let (truth, what) = match kind {
        Kind::Plate => {
            let (a, b) = (rng.between(4.0, 20.0) * k, rng.between(4.0, 20.0) * k);
            let t = rng.pick(&[1e-4, 1e-3, 0.05, 0.3, 0.9]) * a.min(b);
            steps.push(block("A0", V::new(a, b, t), V::new(0.0, 0.0, 0.0)));
            (t, format!("plate {a:.6e} x {b:.6e} x {t:.6e}"))
        }
        Kind::BallShell => {
            let big = rng.between(2.0, 10.0) * k;
            let t0 = rng.pick(&[1e-3, 0.02, 0.1, 0.3]) * big;
            let small = big - t0;
            let e = if rng.below(3) == 0 {
                0.0
            } else {
                rng.between(0.0, 0.9) * t0
            };
            let c = rng.direction() * (rng.between(-2.0, 2.0) * k);
            steps.push(sphere("Outer", big, c));
            steps.push(sphere("Inner", small, c + rng.direction() * e));
            steps.push(subtract("A0", "Outer", "Inner"));
            (
                big - small - e,
                format!("shell of a ball {big:.6e}, the hollow {small:.6e} {e:.6e} off"),
            )
        }
        Kind::Tube => {
            let big = rng.between(2.0, 10.0) * k;
            let t0 = rng.pick(&[1e-3, 0.02, 0.1, 0.3]) * big;
            let small = big - t0;
            let e = if rng.below(3) == 0 {
                0.0
            } else {
                rng.between(0.0, 0.9) * t0
            };
            let h = rng.between(3.0, 6.0) * big;
            let phi = rng.between(0.0, 360.0).to_radians();
            steps.push(cylinder("Outer", big, h, V::new(0.0, 0.0, 0.0)));
            steps.push(cylinder(
                "Bore",
                small,
                3.0 * h,
                V::new(e * phi.pcos(), e * phi.psin(), -h),
            ));
            steps.push(subtract("A0", "Outer", "Bore"));
            (
                big - small - e,
                format!("tube {big:.6e}, its bore {small:.6e} {e:.6e} off, {h:.6e} long"),
            )
        }
        Kind::HoleByWall => {
            // A flat wall against a round one: the least is searched.
            exact = false;
            let r = rng.between(0.5, 3.0) * k;
            let w = rng.pick(&[1e-3, 0.02, 0.1, 0.4]) * r;
            // Every other wall at least four times as thick, the floor too.
            let other = 4.0 * w + rng.between(0.2, 1.0) * r;
            let size = V::new(
                w + 2.0 * r + other,
                2.0 * (r + other),
                rng.between(2.0, 4.0) * r + other,
            );
            let depth = size.z - other;
            steps.push(block("Blank", size, V::new(0.0, 0.0, 0.0)));
            steps.push(cylinder(
                "Drill",
                r,
                depth + r,
                V::new(w + r, 0.5 * size.y, size.z - depth),
            ));
            steps.push(subtract("A0", "Blank", "Drill"));
            (
                w,
                format!(
                    "block {:.4e} x {:.4e} x {:.4e}, a blind hole {r:.6e} {w:.6e} from a wall",
                    size.x, size.y, size.z
                ),
            )
        }
        Kind::Tapered => {
            exact = false;
            let l = rng.between(5.0, 20.0) * k;
            let t0 = rng.pick(&[1e-3, 0.02, 0.1]) * l;
            let t1 = t0 + rng.between(0.02, 0.5) * l;
            let w = rng.between(2.0, 10.0) * k;
            steps.push(("sketch.create", json!({"name": "P", "plane": "xy"})));
            steps.push((
                "sketch.polyline",
                json!({"sketch": "P", "closed": true, "points": [[0.0, 0.0], [l, 0.0], [l, t1], [0.0, t0]]}),
            ));
            steps.push((
                "feature.extrude",
                json!({"sketch": "P", "distance": mm(w), "name": "A0"}),
            ));
            // Measured from the flat face (y = 0, its normal −y): the rays
            // go straight across, t0 + slope · x long.
            from_face = Some(V::new(0.0, -1.0, 0.0));
            (
                t0,
                format!("tapered plate {l:.6e} long from {t0:.6e} to {t1:.6e}, {w:.6e} wide"),
            )
        }
        Kind::ConeShell => {
            let r = rng.between(2.0, 8.0) * k;
            let h = rng.between(2.0, 12.0) * k;
            let sl = r.phypot(h);
            // The inner cone the outer one moved down by d: their sides
            // are parallel, d r / sl apart (the sine of the half-angle).
            let t = rng.pick(&[1e-3, 0.02, 0.1, 0.3]) * r.min(h);
            let d = t * sl / r;
            steps.push((
                "feature.cone",
                json!({"name": "Outer", "radius": mm(r), "height": mm(h)}),
            ));
            steps.push((
                "feature.cone",
                json!({"name": "Inner", "radius": mm(r), "height": mm(h), "position": mm3(V::new(0.0, 0.0, -d))}),
            ));
            steps.push((
                "feature.boolean",
                json!({"op": "subtract", "target": "Outer", "tool": "Inner", "name": "A0"}),
            ));
            // From the hollow's side: its rays go out and up, square to
            // both sides (from the outer side they would run out through
            // the base near its acute rim).
            query = Some(
                json!({"kind": "face", "where": {"madeBy": "Inner", "surface": "cone", "in": "A"}}),
            );
            (
                t,
                format!("cone shell {r:.6e}, {h:.6e}, the hollow {d:.6e} lower: {t:.6e} thick"),
            )
        }
        Kind::TorusShell => {
            let big = rng.between(3.0, 10.0) * k;
            let r1 = rng.between(0.2, 0.6) * big;
            let t0 = rng.pick(&[1e-3, 0.02, 0.1, 0.3]) * r1;
            let r2 = r1 - t0;
            let e = if rng.below(3) == 0 {
                0.0
            } else {
                rng.between(0.0, 0.9) * t0
            };
            steps.push((
                "feature.torus",
                json!({"name": "Outer", "major": mm(big), "minor": mm(r1)}),
            ));
            steps.push((
                "feature.torus",
                json!({"name": "Inner", "major": mm(big + e), "minor": mm(r2)}),
            ));
            steps.push(subtract("A0", "Outer", "Inner"));
            // The tube's wall is thinnest on the outside of the ring,
            // where the hollow's circle has moved to: r1 − r2 − e.
            (
                r1 - r2 - e,
                format!(
                    "ring shell {big:.6e}, tube {r1:.6e}, hollow {r2:.6e} on a circle {e:.6e} wider"
                ),
            )
        }
        Kind::SplineStrip => {
            exact = false;
            let l = rng.between(5.0, 20.0) * k;
            // Flat at both ends (so no ray runs out through an end), a hump
            // or a dip between; the strip t high above it.
            let h = rng.between(-0.25, 0.25) * l;
            let t = rng.pick(&[1e-3, 0.02, 0.05]) * l;
            let lower = Bezier([[0.0, 0.0], [l / 3.0, 0.0], [2.0 * l / 3.0, h], [l, h]]);
            let upper = Bezier(lower.0.map(|[x, y]| [x, y + t]));
            let w = rng.between(2.0, 10.0) * k;
            let truth = strip_thickness(&lower, &upper, l)?;
            let p = lower.0;
            let q = upper.0;
            steps.push(("sketch.create", json!({"name": "P", "plane": "xy"})));
            steps.push((
                "sketch.polyline",
                json!({"sketch": "P", "points": [[p[3][0], p[3][1]], [q[3][0], q[3][1]]]}),
            ));
            steps.push((
                "sketch.spline",
                json!({"sketch": "P", "control": ["$last", [q[2][0], q[2][1]], [q[1][0], q[1][1]], [q[0][0], q[0][1]]]}),
            ));
            steps.push((
                "sketch.polyline",
                json!({"sketch": "P", "points": ["$end", [p[0][0], p[0][1]]]}),
            ));
            steps.push((
                "sketch.spline",
                json!({"sketch": "P", "control": ["$last", [p[1][0], p[1][1]], [p[2][0], p[2][1]], "$first"]}),
            ));
            steps.push((
                "feature.extrude",
                json!({"sketch": "P", "distance": mm(w), "name": "A0"}),
            ));
            // Measured from the lower wall: the face the last spline
            // sweeps.
            from_face = Some(V::new(0.0, 0.0, 0.0));
            (
                truth,
                format!("spline strip {l:.6e} long, {t:.6e} high, its wall rising {h:.6e}"),
            )
        }
    };
    steps.push(motion.step("A0", "A"));
    Some(Case {
        id: format!("{kind:?} {index}"),
        what: format!("{what} at scale {k:e}, {}", motion.describe()),
        k,
        steps,
        truth,
        exact,
        from_face,
        query,
        motion,
    })
}

/// The least length, over the points of the lower curve, of the ray along
/// its normal into the strip (up and back, `(−y', x')`) until it meets the
/// upper curve, or the strip's end walls x = 0 and x = l; by bisection on
/// the ray for each point, dense points refined by golden sections. `None`
/// where a ray leaves through an end wall (the oracle would then need
/// those faces: the case is drawn again).
fn strip_thickness(lower: &Bezier, upper: &Bezier, l: f64) -> Option<f64> {
    let ray = |s: f64| -> f64 {
        let p = lower.at(s);
        let d = lower.d(s);
        let n = d[0].phypot(d[1]);
        let dir = [-d[1] / n, d[0] / n];
        // The upper curve is a graph over x (its control points are evenly
        // spaced along x... at 0, l/3, 2l/3, l): above it is out.
        let above = |q: [f64; 2]| {
            let u = (q[0] / l).clamp(0.0, 1.0);
            q[1] - upper.at(u)[1]
        };
        // Walk out in steps of a thousandth of the length to the first
        // place above the upper curve or past an end, then bisect.
        let step = 1e-3 * l;
        let mut t = 0.0;
        let (mut lo, mut hi) = loop {
            let q = [p[0] + dir[0] * (t + step), p[1] + dir[1] * (t + step)];
            if above(q) > 0.0 || q[0] < 0.0 || q[0] > l {
                break (t, t + step);
            }
            t += step;
            if t > 10.0 * l {
                return f64::NAN;
            }
        };
        for _ in 0..200 {
            let mid = 0.5 * (lo + hi);
            let q = [p[0] + dir[0] * mid, p[1] + dir[1] * mid];
            if above(q) > 0.0 || q[0] < 0.0 || q[0] > l {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        let q = [p[0] + dir[0] * hi, p[1] + dir[1] * hi];
        if q[0] < 0.0 || q[0] > l {
            f64::INFINITY
        } else {
            hi
        }
    };
    let n = 4000;
    let samples: Vec<f64> = (0..=n).map(|i| ray(i as f64 / n as f64)).collect();
    if samples.iter().any(|v| !v.is_finite()) {
        return None;
    }
    let mut best = f64::INFINITY;
    for i in 0..=n {
        let left = i == 0 || samples[i] <= samples[i - 1];
        let right = i == n || samples[i] <= samples[i + 1];
        if left && right {
            let lo = (i as f64 - 1.0).max(0.0) / n as f64;
            let hi = (i as f64 + 1.0).min(n as f64) / n as f64;
            best = best.min(golden(lo, hi, ray).1);
        }
    }
    Some(best)
}

/// Runs the steps of a case, putting the last polyline's points in for
/// `$last`, `$first` (of the first polyline) and `$end` (the last spline's
/// end).
fn build_case(s: &mut craft3d_engine::Session, steps: &[Step]) -> Result<String, Ran> {
    let mut first: Vec<Value> = Vec::new();
    let mut spline = String::new();
    let mut last: Vec<Value> = Vec::new();
    let mut end = Value::Null;
    for (id, params) in steps {
        let mut params = params.clone();
        for key in ["control", "points"] {
            if let Some(list) = params.get_mut(key).and_then(Value::as_array_mut) {
                for c in list.iter_mut() {
                    if c == "$last" {
                        *c = last.last().cloned().unwrap_or(Value::Null);
                    } else if c == "$first" {
                        *c = first.first().cloned().unwrap_or(Value::Null);
                    } else if c == "$end" {
                        *c = end.clone();
                    }
                }
            }
        }
        match run(s, id, &params) {
            Ran::Ok(v) => {
                if *id == "sketch.polyline" {
                    last = v["points"].as_array().cloned().unwrap_or_default();
                    if first.is_empty() {
                        first = last.clone();
                    }
                }
                if *id == "sketch.spline" {
                    end = v["end"].clone();
                    spline = v["entity"].as_str().unwrap_or("").to_string();
                }
            }
            other => return Err(other),
        }
    }
    Ok(spline)
}

/// The name of the face of `A` whose outward normal (at its middle, for a
/// flat face; any face reported with a normal) is nearest `n`.
fn face_along(s: &mut craft3d_engine::Session, n: V) -> Option<String> {
    let Ran::Ok(out) = run(
        s,
        "select.query",
        &json!({"kind": "face", "where": {"in": "A"}, "context": true}),
    ) else {
        return None;
    };
    let items = out["items"].as_array()?;
    items
        .iter()
        .filter_map(|it| {
            let m = it["surface"]["normal"].as_array()?;
            let m = V::new(m[0].as_f64()?, m[1].as_f64()?, m[2].as_f64()?);
            Some((it["name"].as_str()?.to_string(), m.dot(n)))
        })
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .filter(|(_, d)| *d > 0.999)
        .map(|(name, _)| name)
}

fn judge(case: &Case) -> Option<(Verdict, String)> {
    let mut s = document(case.k);
    let spline = match build_case(&mut s, &case.steps) {
        Ok(spline) => spline,
        Err(Ran::Panic(p)) => return Some((Verdict::Panic, format!("building: {p}"))),
        Err(e) => {
            if std::env::var("HUNT_VERBOSE").is_ok() {
                println!("not built: {}: {e:?}", case.what);
            }
            return None;
        }
    };
    let tol = 1e-6 * case.k;
    let eps = 1e-13 * (case.motion.reach() + 50.0 * case.k) + 1e-12 * case.truth;
    let of = match case.from_face {
        None if case.query.is_some() => case.query.clone().unwrap(),
        None => json!("A"),
        Some(n) if n.len() == 0.0 => {
            json!({"kind": "face", "where": {"name": format!("*side({spline})"), "in": "A"}})
        }
        Some(n) => {
            let name = face_along(&mut s, case.motion.turn.apply(n))?;
            json!({"kind": "face", "where": {"name": name, "in": "A"}})
        }
    };
    let out = match run(&mut s, "measure.thickness", &json!({"of": of})) {
        Ran::Panic(p) => return Some((Verdict::Panic, format!("thickness: {p}"))),
        Ran::Err(e) => return Some((Verdict::Declined, format!("thickness: {e}"))),
        Ran::Ok(v) => v,
    };
    let (Some(found), Some(at_least)) = (out["thickness"].as_f64(), out["atLeast"].as_f64()) else {
        return Some((Verdict::Declined, format!("no thickness in {out}")));
    };
    let t = case.truth;
    let mut wrong = Vec::new();
    let mut notes = Vec::new();
    if at_least > t + eps {
        wrong.push(format!(
            "atLeast {at_least:.17e} above the true thickness {t:.17e} (by {:.3e})",
            at_least - t
        ));
    }
    if found < t - 2.0 * tol - eps {
        wrong.push(format!(
            "thickness {found:.17e} below the true {t:.17e} (by {:.3e})",
            t - found
        ));
    }
    let acc = 2.0 * tol + 1e-9 * t + eps;
    if found > t + acc {
        let line = format!(
            "thickness {found:.17e} above the true {t:.17e} by {:.3e} ({:.1} tolerances)",
            found - t,
            (found - t) / tol
        );
        if case.exact && found <= t + acc + 8.0 * tol {
            notes.push(format!("inexact: {line}"));
        } else if case.exact {
            wrong.push(format!("{line}, where it says it is exact"));
        } else {
            notes.push(format!("{line} (a searched least)"));
        }
    }
    if at_least > found {
        wrong.push(format!(
            "atLeast {at_least:e} above the thickness {found:e}"
        ));
    }
    let verdict = if wrong.is_empty() {
        Verdict::Right
    } else {
        Verdict::Wrong
    };
    let mut lines = wrong;
    lines.extend(notes);
    lines.push(format!("found {found:.6e}, atLeast {at_least:.6e}"));
    Some((verdict, lines.join("; ")))
}

fn sweep(kind: Kind, seed: u64, count: usize) -> Tally {
    let mut rng = Rng(seed ^ (kind as u64 + 7).wrapping_mul(0x9E37_79B9_7F4A_7C15));
    let mut tally = Tally::default();
    let mut index = 0;
    let mut draws = 0;
    while index < count && draws < 8 * count {
        draws += 1;
        let Some(case) = case(&mut rng, kind, index) else {
            continue;
        };
        let started = std::time::Instant::now();
        let Some((verdict, why)) = judge(&case) else {
            continue;
        };
        let ms = started.elapsed().as_millis();
        if why.contains("inexact") {
            tally.inexact += 1;
        }
        let line = format!("{}: {why} [{ms} ms]", case.what);
        if verdict != Verdict::Right
            || why.contains("inexact")
            || std::env::var("HUNT_VERBOSE").is_ok()
        {
            println!("{} {} {line}", verdict.letter(), case.id);
        }
        tally.add(&case.id, verdict, line);
        index += 1;
    }
    tally
}

/// Wrong cases of the gate's sweep, by kind and index, with what is wrong.
const KNOWN: [(Kind, usize, &str); 0] = [];

const GATE_SEED: u64 = 0x5EED_7417_0001;

fn hold(kind: Kind, count: usize) {
    let tally = sweep(kind, GATE_SEED, count);
    tally.print(&format!("{kind:?}"));
    for (id, letter, what) in &tally.notes {
        let index: usize = id.rsplit(' ').next().unwrap().parse().unwrap();
        let known = KNOWN.iter().any(|(k, i, _)| *k == kind && *i == index);
        assert!(
            *letter == 'D' || (known && *letter == 'W'),
            "{id} {letter}: {what}"
        );
    }
    for (k, i, _) in KNOWN.iter().filter(|(k, ..)| *k == kind) {
        let id = format!("{k:?} {i}");
        assert!(
            tally.notes.iter().any(|(n, l, _)| *n == id && *l == 'W'),
            "{id} is listed as known wrong but is not: take it off KNOWN"
        );
    }
}

const GATE_CASES: usize = 10;

#[test]
fn plates() {
    hold(Kind::Plate, GATE_CASES);
}

#[test]
fn shells_and_tubes() {
    // Thin shells cost a proof of tens of thousands of pairs each.
    hold(Kind::BallShell, GATE_CASES / 2);
    hold(Kind::Tube, GATE_CASES / 2);
}

#[test]
fn a_hole_by_a_wall() {
    hold(Kind::HoleByWall, GATE_CASES);
}

#[test]
fn tapered_and_spline_walls() {
    hold(Kind::Tapered, GATE_CASES);
    hold(Kind::SplineStrip, GATE_CASES);
}

#[test]
fn cone_shells() {
    hold(Kind::ConeShell, GATE_CASES);
}

#[test]
fn torus_shells() {
    // A ring's proof is the slowest of these: a few seconds a case.
    hold(Kind::TorusShell, 3);
}

fn wide(kinds: &[Kind]) {
    let count: usize = std::env::var("HUNT_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(40);
    let seed: u64 = std::env::var("HUNT_SEED")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0x5EED_7417_1000);
    let mut all = Tally::default();
    for &kind in kinds {
        let tally = sweep(kind, seed, count);
        println!("{}", tally.summary(&format!("{kind:?}")));
        all.merge(tally);
    }
    println!("{}", all.summary("all"));
}

#[test]
#[ignore]
fn wide_plates_and_holes() {
    wide(&[Kind::Plate, Kind::HoleByWall]);
}

#[test]
#[ignore]
fn wide_ball_shells() {
    wide(&[Kind::BallShell]);
}

#[test]
#[ignore]
fn wide_tubes() {
    wide(&[Kind::Tube]);
}

#[test]
#[ignore]
fn wide_slanted() {
    wide(&[Kind::Tapered, Kind::SplineStrip]);
}

#[test]
#[ignore]
fn wide_cone_shells() {
    wide(&[Kind::ConeShell]);
}

#[test]
#[ignore]
fn wide_torus_shells() {
    wide(&[Kind::TorusShell]);
}

/// Found by this hunt (2026-11-14) and repaired the same day: a blind
/// hole with a 118° drill point (a cylinder of radius 2 and a cone at its
/// end, apex down) in a block, 0.5 of floor left under the apex and every
/// other wall thicker. The thinnest place is under the apex: the kernel's
/// ray straight up through it meets the bottom face and the apex 0.5
/// apart. `measure.thickness` answered 0.50000004 with `atLeast` the
/// same, 3.5e-8 (0.035 tolerances) above the truth: the proof missed the
/// point's face near its apex (its parameter box was taken from the
/// heights of its edges, and the apex is on none; as for the clearance of
/// a ball sunk into a cone, `hunt_checks_distance.rs`), so `atLeast` was
/// the place found, which the search along the face leaves a little
/// short of the apex. Now what is proven is no more than the floor, and
/// within a billionth of it (`proofs_at_poles.rs` holds the same at three
/// scales and far from the origin, and under a pit that is a cone or a
/// ball nose).
#[test]
fn the_floor_under_a_drill_point_is_proven_no_thicker_than_it_is() {
    let (r, w, depth) = (2.0_f64, 0.5_f64, 6.0_f64);
    let tip = r / (59.0_f64.to_radians()).ptan();
    let size = V::new(4.0 * r + 6.0, 4.0 * r + 6.0, depth + tip + w);
    let mid = (0.5 * size.x, 0.5 * size.y);
    let top = size.z;
    let mut s = document(1.0);
    build(
        &mut s,
        &[
            (
                "feature.block",
                json!({"name": "Blank", "size": mm3(size)}),
            ),
            (
                "feature.cylinder",
                json!({"name": "Bore", "radius": mm(r), "height": mm(depth + 1.0), "position": mm3(V::new(mid.0, mid.1, top - depth))}),
            ),
            (
                "feature.cone",
                json!({"name": "Point", "radius": mm(r), "height": mm(tip), "position": mm3(V::new(mid.0, mid.1, top - depth)), "axis": [0, 0, -1]}),
            ),
            (
                "feature.boolean",
                json!({"op": "union", "target": "Bore", "tool": "Point", "name": "Drill"}),
            ),
            (
                "feature.boolean",
                json!({"op": "subtract", "target": "Blank", "tool": "Drill", "name": "A"}),
            ),
        ],
    )
    .unwrap();
    let Ran::Ok(ray) = run(
        &mut s,
        "measure.ray",
        &json!({"from": mm3(V::new(mid.0, mid.1, -1.0)), "direction": [0, 0, 1], "of": "A"}),
    ) else {
        panic!("ray");
    };
    let floor =
        ray["hits"][1]["distance"].as_f64().unwrap() - ray["hits"][0]["distance"].as_f64().unwrap();
    assert!((floor - w).abs() < 1e-12, "{ray}");
    let Ran::Ok(out) = run(&mut s, "measure.thickness", &json!({"of": "A"})) else {
        panic!("thickness");
    };
    let at_least = out["atLeast"].as_f64().unwrap();
    assert!(
        at_least <= floor && at_least >= floor * (1.0 - 1e-9),
        "the floor under the point is proven {at_least}, against {floor}"
    );
}
