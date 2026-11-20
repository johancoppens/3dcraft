//! What a plane cut costs, counted in work, not time (docs/testing.md ›
//! *Performance tests*): the case of the public repository's issue 3, a
//! prism of 70 outline vertices trimmed by 24 planes, built with one
//! `feature.planeCut` and built the way it had to be built before, a
//! datum plane, a sketch, a rectangle, an extrusion and a boolean per
//! plane. The counts are the kernel's and regeneration's own
//! (`craft3d_kernel::work`, `craft3d_regen::work`), process-wide, so this
//! file holds one test and runs its commands without the comparison with
//! a full regeneration that `common::checked` adds after each.
//!
//! Held: both ways give the same body; one feature asks no more booleans
//! than planes; the work per plane does not grow with the number of
//! planes beyond what the body's own growth explains; and what one
//! changed plane rebuilds (the whole feature: every plane again). The
//! counts are printed for the design document
//! (docs/design/plane-cut.md › *Cost and bounds*).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::plain::run;
use common::session;
use craft3d_engine::Session;
use serde_json::{Value, json};

/// SplitMix64: seeded, the same on every platform.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `[-1, 1)`.
    fn noise(&mut self) -> f64 {
        2.0 * ((self.next() >> 11) as f64 / (1u64 << 53) as f64) - 1.0
    }
}

/// `count` directions evenly round a circle, from the cosine and sine of
/// one step (no trigonometry: a rotation applied again and again).
fn round(count: usize, (c, s): (f64, f64)) -> Vec<[f64; 2]> {
    let mut out = Vec::with_capacity(count);
    let mut d = [1.0, 0.0];
    for _ in 0..count {
        out.push(d);
        d = [c * d[0] - s * d[1], s * d[0] + c * d[1]];
    }
    out
}

const STEP_70: (f64, f64) = (0.995_974_293_995_239_1, 0.089_639_308_903_433_5);
const STEP_24: (f64, f64) = (0.965_925_826_289_068_3, 0.258_819_045_102_520_74);

/// A plane of the roof: a point of it and its normal, up and outward.
#[derive(Clone, Copy)]
struct Slope {
    point: [f64; 3],
    normal: [f64; 3],
    /// A direction in the plane, level.
    along: [f64; 3],
}

/// `count` planes round a cone of about 30 degrees, each through a point
/// at radius 10 and height 18, scattered as measured planes are: a
/// third of a millimetre in height, a few hundredths in slope.
fn slopes(count: usize) -> Vec<Slope> {
    let mut rng = Rng(20_261_120);
    // The 24 directions; fewer planes take every third, more none.
    let directions = round(24, STEP_24);
    (0..count)
        .map(|j| {
            let [dx, dy] = directions[(j * 24 / count) % 24];
            // tan 30 degrees, scattered.
            let t = 0.577 + 0.02 * rng.noise();
            let n = (1.0 + t * t).sqrt();
            Slope {
                point: [10.0 * dx, 10.0 * dy, 18.0 + 0.3 * rng.noise()],
                normal: [t * dx / n, t * dy / n, 1.0 / n],
                along: [-dy, dx, 0.0],
            }
        })
        .collect()
}

fn mm(v: f64) -> String {
    format!("{v} mm")
}

/// The prism: an outline of 70 vertices, a radius of 20 with a wave of 3
/// and a scatter of 1, extruded 30 (`Prism`).
fn prism(s: &mut Session) {
    let mut rng = Rng(70);
    let points: Vec<Value> = round(70, STEP_70)
        .into_iter()
        .enumerate()
        .map(|(i, [dx, dy])| {
            // A wave of five lobes, as a polygon's corners give it.
            let wave = if i % 14 < 7 { 1.0 } else { -1.0 };
            let r = 20.0 + 3.0 * wave * (((i % 7) as f64) / 7.0) + rng.noise();
            json!([mm(r * dx), mm(r * dy)])
        })
        .collect();
    run(s, "file.new", json!({"units": "mm"}));
    run(
        s,
        "sketch.create",
        json!({"plane": "xy", "name": "Outline"}),
    );
    run(
        s,
        "sketch.polyline",
        json!({"sketch": "Outline", "points": points, "closed": true}),
    );
    let made = run(
        s,
        "feature.extrude",
        json!({"sketch": "Outline", "distance": "30 mm", "name": "Prism"}),
    );
    assert_eq!(made["status"], "ok", "{made}");
}

/// The kernel's and regeneration's counts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Work {
    booleans: u64,
    face_pairs: u64,
    face_candidates: u64,
    edge_candidates: u64,
    features_rebuilt: u64,
    regenerations: u64,
}

fn work() -> Work {
    let kernel = craft3d_kernel::work::work();
    let regen = craft3d_regen::work::work();
    Work {
        booleans: kernel.booleans,
        face_pairs: kernel.face_pairs,
        face_candidates: kernel.face_candidates,
        edge_candidates: kernel.edge_candidates,
        features_rebuilt: regen.features_rebuilt,
        regenerations: regen.regenerations,
    }
}

/// What `command` cost.
fn counted<T>(command: impl FnOnce() -> T) -> (T, Work) {
    let before = work();
    let out = command();
    let after = work();
    (
        out,
        Work {
            booleans: after.booleans - before.booleans,
            face_pairs: after.face_pairs - before.face_pairs,
            face_candidates: after.face_candidates - before.face_candidates,
            edge_candidates: after.edge_candidates - before.edge_candidates,
            features_rebuilt: after.features_rebuilt - before.features_rebuilt,
            regenerations: after.regenerations - before.regenerations,
        },
    )
}

fn body(s: &mut Session, name: &str) -> Value {
    let document = run(s, "document.inspect", json!({"inertia": false}));
    document["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["name"] == name)
        .unwrap_or_else(|| panic!("no body {name} in {document}"))
        .clone()
}

fn features(s: &mut Session) -> usize {
    run(s, "document.inspect", json!({"measures": false}))["features"]
        .as_array()
        .unwrap()
        .len()
}

/// The planes as one feature takes them; plane `lifted` through a point
/// raised by the variable `lift`.
fn listed(slopes: &[Slope], lifted: usize) -> Vec<Value> {
    slopes
        .iter()
        .enumerate()
        .map(|(j, slope)| {
            let [x, y, z] = slope.point;
            let height = if j == lifted {
                format!("{z} mm + lift")
            } else {
                mm(z)
            };
            json!({"point": [mm(x), mm(y), height], "normal": slope.normal})
        })
        .collect()
}

/// One feature for all the planes (`Roof`), and what it cost.
fn one_feature(s: &mut Session, slopes: &[Slope]) -> Work {
    run(s, "variable.set", json!({"name": "lift", "expr": "0 mm"}));
    let (made, cost) = counted(|| {
        run(
            s,
            "feature.planeCut",
            json!({"target": "Prism", "planes": listed(slopes, slopes.len() / 2), "name": "Roof"}),
        )
    });
    assert_eq!(made["status"], "ok", "{made}");
    cost
}

/// The three points a datum plane of a slope goes through, the slope
/// raised by `lift`: counter-clockwise about its normal.
fn three_points(slope: &Slope, lift: f64) -> Value {
    let (p, u, n) = (slope.point, slope.along, slope.normal);
    let v = [
        n[1] * u[2] - n[2] * u[1],
        n[2] * u[0] - n[0] * u[2],
        n[0] * u[1] - n[1] * u[0],
    ];
    let at = |d: [f64; 3]| {
        json!([
            mm(p[0] + 10.0 * d[0]),
            mm(p[1] + 10.0 * d[1]),
            mm(p[2] + lift + 10.0 * d[2])
        ])
    };
    json!([at([0.0; 3]), at(u), at(v)])
}

/// Five features a plane, and what the planes cost together; the last
/// body is `C<n - 1>`.
fn five_features(s: &mut Session, slopes: &[Slope]) -> Work {
    let mut target = "Prism".to_string();
    let ((), cost) = counted(|| {
        for (j, slope) in slopes.iter().enumerate() {
            run(
                s,
                "datum.plane",
                json!({"through": three_points(slope, 0.0), "name": format!("P{j}")}),
            );
            run(
                s,
                "sketch.create",
                json!({"plane": format!("P{j}"), "name": format!("S{j}")}),
            );
            run(
                s,
                "sketch.rect",
                json!({"sketch": format!("S{j}"), "from": ["-100 mm", "-100 mm"], "to": ["100 mm", "100 mm"]}),
            );
            let name = format!("C{j}");
            let made = run(
                s,
                "feature.extrude",
                json!({"sketch": format!("S{j}"), "distance": "100 mm", "cut": target, "name": name}),
            );
            assert_eq!(made["status"], "ok", "plane {j}: {made}");
            target = name;
        }
    });
    cost
}

#[test]
fn one_feature_costs_no_more_than_five_features_a_plane() {
    let roof = slopes(24);
    let middle = roof.len() / 2;

    // One feature.
    let mut s = session();
    prism(&mut s);
    let one = one_feature(&mut s, &roof);
    let one_body = body(&mut s, "Roof");
    let one_history = features(&mut s);
    let (_, one_edit) = counted(|| {
        run(
            &mut s,
            "variable.set",
            json!({"name": "lift", "expr": "0.25 mm"}),
        )
    });
    let one_lifted = body(&mut s, "Roof");

    // Five features a plane.
    let mut t = session();
    prism(&mut t);
    let five = five_features(&mut t, &roof);
    let last = format!("C{}", roof.len() - 1);
    let five_body = body(&mut t, &last);
    let five_history = features(&mut t);
    let (_, five_edit) = counted(|| {
        run(
            &mut t,
            "datum.plane",
            json!({"replace": format!("P{middle}"), "through": three_points(&roof[middle], 0.25)}),
        )
    });
    let five_lifted = body(&mut t, &last);

    println!("plane_cut_cost: 70 outline vertices, 24 planes");
    println!("plane_cut_cost: one feature        history {one_history:3}  build {one:?}");
    println!("plane_cut_cost: five a plane       history {five_history:3}  build {five:?}");
    println!("plane_cut_cost: one feature, the middle plane changed  {one_edit:?}");
    println!("plane_cut_cost: five a plane, the middle plane changed {five_edit:?}");

    // The same body, both ways, before the edit and after it.
    for (a, b) in [(&one_body, &five_body), (&one_lifted, &five_lifted)] {
        for key in ["faces", "edges", "vertices", "shells"] {
            assert_eq!(a[key], b[key], "{key}");
        }
        for key in ["volume", "area"] {
            let (x, y) = (a[key].as_f64().unwrap(), b[key].as_f64().unwrap());
            assert!((x - y).abs() <= 1e-9 * y.abs(), "{key}: {x} {y}");
        }
    }
    assert!(one_lifted["volume"].as_f64().unwrap() > one_body["volume"].as_f64().unwrap());
    let check = run(&mut s, "document.check", json!({"deep": true}));
    assert_eq!(check["ok"], true, "{check}");

    // The history: the outline, the prism and the cut, against four
    // entries a plane more than the first two.
    assert_eq!(one_history, 3);
    assert_eq!(five_history, 2 + 4 * roof.len());
    // One boolean a plane at the most (a plane clear of the body's box
    // needs none), and no more of the boolean's work than the same cuts
    // cost one by one: the body is the same at every step, the tool a
    // block either way.
    assert!(one.booleans <= roof.len() as u64, "{one:?}");
    assert!(one.booleans <= five.booleans, "{one:?} {five:?}");
    assert!(
        one.face_pairs <= five.face_pairs + five.face_pairs / 10,
        "{one:?} {five:?}"
    );
    assert!(
        one.face_candidates <= five.face_candidates + five.face_candidates / 10,
        "{one:?} {five:?}"
    );
    assert!(
        one.features_rebuilt < five.features_rebuilt,
        "{one:?} {five:?}"
    );
    // One changed plane: the feature is rebuilt whole, every plane again,
    // where the five-feature way rebuilds the planes from the changed one
    // on. That is the price of one history entry (slice 3 of the design).
    assert_eq!(one_edit.features_rebuilt, 1, "{one_edit:?}");
    assert_eq!(one_edit.booleans, one.booleans, "{one_edit:?}");
    assert!(five_edit.booleans < one_edit.booleans, "{five_edit:?}");

    // Twice the planes cost about twice the work, not four times: every
    // third direction, and all of them.
    let mut costs = Vec::new();
    for count in [8, 24] {
        let mut s = session();
        prism(&mut s);
        let cost = one_feature(&mut s, &slopes(count));
        println!("plane_cut_cost: {count:2} planes in one feature {cost:?}");
        costs.push((count as u64, cost));
    }
    let (few, small) = costs[0];
    let (many, large) = costs[1];
    // Per plane, at most half as much again: the body gains a face or two
    // a plane, which every later plane meets.
    assert!(
        large.face_candidates * few * 2 <= small.face_candidates * many * 3,
        "{small:?} {large:?}"
    );
}
