//! Fillets whose first tool's boolean builds an invalid body (random
//! designs 103578, 110541, 121649, 124545 and 128676: a tool's tangent
//! edge, on a face it is coincident with, crosses that face's spline
//! border; `docs/design/blends.md` › *A tool boolean's invalid body*). The
//! fillet passed the boolean's *"boolean produced an invalid body"* on as
//! its own error, and its search for what fits never ran. Now the search
//! runs; a largest radius that fits is said only where the faces give it
//! (the border reaches something of its face there) and every size tried
//! between it and the radius asked fails too; otherwise the fillet's
//! refusal names the boolean's reason.
//!
//! Each case is built by commands at scales 1e-3, 1, 1e3 and 1e6 (the
//! document's tolerance 1e-6 times the scale) and turned (X to Y, Y to Z,
//! Z to X; and 30 degrees about Z). What fits is held to the faces' own
//! limit worked out by hand; a radius just under it builds and passes the
//! deep check. Where the boolean's faults lie below that limit (124545), a
//! chamfer of the same edge, whose tool has no tangent edge, finds the
//! limit, and fillets below it build: there the answer is the boolean's
//! refusal, never a "too large" that is not so.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::print_stdout
)]

use craft3d_engine::{Session, Workspace};
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug)]
enum Turn {
    None,
    /// (x, y, z) to (z, x, y): X to Y, Y to Z, Z to X.
    Cycle,
    /// 30 degrees about Z.
    Z30,
}

const TURNS: [Turn; 3] = [Turn::None, Turn::Cycle, Turn::Z30];
const SCALES: [f64; 4] = [1e-3, 1.0, 1e3, 1e6];

/// The sine of 30 degrees; its cosine is [`c30`].
const S30: f64 = 0.5;

fn c30() -> f64 {
    0.75_f64.sqrt()
}

/// The words of the fillet's refusal that names the boolean's reason.
const BOOLEAN_REFUSED: &str =
    "the boolean that cuts this fillet refused it: boolean produced an invalid body";

fn v(t: Turn, k: f64, p: [f64; 3]) -> Value {
    let [x, y, z] = [p[0] * k, p[1] * k, p[2] * k];
    match t {
        Turn::None => json!([x, y, z]),
        Turn::Cycle => json!([z, x, y]),
        Turn::Z30 => json!([x * c30() - y * S30, x * S30 + y * c30(), z]),
    }
}

fn axis(t: Turn, p: [f64; 3]) -> Value {
    v(t, 1.0, p)
}

/// The world plane that the XY plane turns to, its sketch axes the
/// turned X and Y.
fn plane(t: Turn) -> &'static str {
    match t {
        Turn::None | Turn::Z30 => "xy",
        Turn::Cycle => "yz",
    }
}

/// A point of a sketch on [`plane`].
fn uv(t: Turn, k: f64, p: [f64; 2]) -> Value {
    let [x, y] = [p[0] * k, p[1] * k];
    match t {
        Turn::None | Turn::Cycle => json!([x, y]),
        Turn::Z30 => json!([x * c30() - y * S30, x * S30 + y * c30()]),
    }
}

type Steps = Vec<(&'static str, Value)>;
type Make = fn(f64, Turn, f64) -> Steps;

fn new(k: f64) -> (&'static str, Value) {
    ("file.new", json!({"units": "mm", "tolerance": 1e-6 * k}))
}

fn fillet(name: &str, faces: [&str; 2], r: f64) -> (&'static str, Value) {
    (
        "feature.fillet",
        json!({"chain": false, "edges": [faces], "name": name, "radius": r}),
    )
}

/// Random design 124545 as reduced: a torus about Z through (0, 8.75), R
/// 7.5, r 2.5, at z = -7.5, cut to the cylinder of radius 7.5 about X from
/// x = 0 to 10; the rim at x = 0, between the cap (a lens) and the wall
/// (whose other border is a spline that ends where the rim ends),
/// filleted. The cap leaves room up to 7.5 - (sqrt(1.25² + 7.5²) - 2.5).
fn s124545(k: f64, t: Turn, r: f64) -> Steps {
    vec![
        new(k),
        (
            "feature.torus",
            json!({"angle": "90 deg", "axis": axis(t, [0.0, 0.0, 1.0]), "major": 7.5 * k, "minor": 2.5 * k, "name": "N0", "position": v(t, k, [0.0, 8.75, -7.5])}),
        ),
        (
            "feature.cylinder",
            json!({"name": "C", "position": v(t, k, [0.0, 0.0, 0.0]), "axis": axis(t, [1.0, 0.0, 0.0]), "radius": 7.5 * k, "height": 10.0 * k}),
        ),
        (
            "feature.boolean",
            json!({"name": "N1", "op": "intersect", "target": "C", "tool": "N0"}),
        ),
        fillet(
            "N4",
            ["F3:copy(0)/F2:face(0)", "F3:copy(0)/F2:face(1)"],
            r * k,
        ),
    ]
}

/// [`s124545`] with the rim chamfered by `d` instead.
fn c124545(k: f64, t: Turn, d: f64) -> Steps {
    let mut steps = s124545(k, t, 1.0);
    steps.pop();
    steps.push((
        "feature.chamfer",
        json!({"edges": [["F3:copy(0)/F2:face(0)", "F3:copy(0)/F2:face(1)"]], "name": "N4", "distance": d * k}),
    ));
    steps
}

/// Random design 121649 as reduced: a cylinder of radius 2.5 about X from
/// x = -10 to -7.5, less a cone whose base circle passes through the
/// rim's one vertex; the rim at x = -10 filleted.
fn s121649(k: f64, t: Turn, r: f64) -> Steps {
    vec![
        new(k),
        (
            "feature.cylinder",
            json!({"axis": axis(t, [1.0, 0.0, 0.0]), "height": 2.5 * k, "name": "N0", "position": v(t, k, [-10.0, 0.0, 0.0]), "radius": 2.5 * k, "angle": "0 deg"}),
        ),
        (
            "feature.cone",
            json!({"axis": axis(t, [0.0, 0.0, 1.0]), "height": 10.0 * k, "name": "N1", "position": v(t, k, [-7.5, 2.5, 0.0]), "radius": 2.5 * k}),
        ),
        (
            "feature.boolean",
            json!({"name": "N3", "op": "subtract", "target": "N0", "tool": "N1"}),
        ),
        fillet(
            "N6",
            ["F3:copy(0)/F1:face(0)", "F3:copy(0)/F1:face(1)"],
            r * k,
        ),
    ]
}

/// Random design 128676 as reduced: a hole of radius 5 cut down through a
/// torus's tube, its rim filleted by the hole's own radius.
fn s128676(k: f64, t: Turn, r: f64) -> Steps {
    vec![
        new(k),
        (
            "feature.torus",
            json!({"angle": "0 deg", "axis": axis(t, [1.0, 1.0, 0.0]), "major": 15.0 * k, "minor": 2.5 * k, "name": "N0", "position": v(t, k, [5.0, 0.0, 10.0])}),
        ),
        (
            "sketch.create",
            json!({"name": "S2", "offset": 0.0, "plane": plane(t)}),
        ),
        (
            "sketch.circle",
            json!({"center": uv(t, k, [-5.0, 5.0]), "radius": 5.0 * k, "sketch": "S2"}),
        ),
        (
            "feature.extrude",
            json!({"cut": "N0", "name": "N2", "reversed": true, "sketch": "S2", "throughAll": true}),
        ),
        fillet(
            "N4",
            ["F4:copy(1)/F3:side(e2)", "F4:copy(1)/F3:startCap(e2)"],
            r * k,
        ),
    ]
}

/// Random design 103578 as reduced: a cylinder of radius 5 about Z from
/// z = 10 to 15 cut to a torus about Y; the wall's lower border (a spline)
/// is highest, at z = 10 + 12.5 - sqrt(75), where y = ±5. What fits the
/// top rim is below 15 - that, 5 sqrt(3) - 7.5.
fn s103578(k: f64, t: Turn, r: f64) -> Steps {
    vec![
        new(k),
        (
            "feature.torus",
            json!({"name": "Tor", "position": v(t, k, [0.0, 0.0, 10.0]), "axis": axis(t, [0.0, 1.0, 0.0]), "major": 12.5 * k, "minor": 10.0 * k}),
        ),
        (
            "feature.cylinder",
            json!({"name": "Cyl", "position": v(t, k, [0.0, 0.0, 10.0]), "axis": axis(t, [0.0, 0.0, 1.0]), "radius": 5.0 * k, "height": 5.0 * k}),
        ),
        (
            "feature.boolean",
            json!({"name": "N5", "op": "intersect", "target": "Cyl", "tool": "Tor"}),
        ),
        fillet(
            "F",
            ["F3:copy(0)/F2:face(0)", "F3:copy(0)/F2:face(2)"],
            r * k,
        ),
    ]
}

/// Random design 110541 as reduced: a cone (base radius 7.5 at z = 3.75
/// about (-2.5, 0.02)) less a hole of radius 2.5 about Z; the hole's rim
/// in the base filleted. The base's outer circle comes nearest the hole's
/// axis at 7.5 - sqrt(2.5² + 0.02²): what fits is below that less 2.5.
fn s110541(k: f64, t: Turn, r: f64) -> Steps {
    vec![
        new(k),
        (
            "feature.cylinder",
            json!({"name": "Hole", "position": v(t, k, [0.0, 0.0, 0.0]), "axis": axis(t, [0.0, 0.0, 1.0]), "radius": 2.5 * k, "height": 20.0 * k}),
        ),
        (
            "feature.cone",
            json!({"name": "Cone", "position": v(t, k, [-2.5, 0.02, 3.75]), "axis": axis(t, [0.0, 0.0, 1.0]), "radius": 7.5 * k, "height": 12.5 * k, "angle": "0 deg"}),
        ),
        (
            "feature.boolean",
            json!({"name": "N3", "op": "subtract", "target": "Cone", "tool": "Hole"}),
        ),
        fillet(
            "F",
            ["F3:copy(0)/F2:face(1)", "F3:copy(1)/F1:face(0)"],
            r * k,
        ),
    ]
}

/// A cylinder of radius 5 about Z from z = 10 to 15, less a ball about
/// (0, 8, 8) of radius sqrt(45): the wall's lower border is the spline in
/// which they meet, highest at y = 5, z = 14. What fits the top rim is
/// below 1.
fn ball_rim(k: f64, t: Turn, r: f64) -> Steps {
    vec![
        new(k),
        (
            "feature.cylinder",
            json!({"name": "Cyl", "position": v(t, k, [0.0, 0.0, 10.0]), "axis": axis(t, [0.0, 0.0, 1.0]), "radius": 5.0 * k, "height": 5.0 * k}),
        ),
        (
            "feature.sphere",
            json!({"name": "Ball", "position": v(t, k, [0.0, 8.0, 8.0]), "radius": 45.0_f64.sqrt() * k}),
        ),
        (
            "feature.boolean",
            json!({"name": "N", "op": "subtract", "target": "Cyl", "tool": "Ball"}),
        ),
        fillet(
            "F",
            ["F3:copy(0)/F1:face(0)", "F3:copy(0)/F1:face(2)"],
            r * k,
        ),
    ]
}

/// The same cylinder cut by the plane z = 11.5 + y / 2: the wall's lower
/// border is an ellipse, highest at y = 5, z = 14. What fits the top rim
/// is below 1.
fn plane_rim(k: f64, t: Turn, r: f64) -> Steps {
    vec![
        new(k),
        (
            "feature.cylinder",
            json!({"name": "Cyl", "position": v(t, k, [0.0, 0.0, 10.0]), "axis": axis(t, [0.0, 0.0, 1.0]), "radius": 5.0 * k, "height": 5.0 * k}),
        ),
        (
            "feature.planeCut",
            json!({"name": "P", "target": "Cyl", "planes": [{"normal": axis(t, [0.0, 0.5, -1.0]), "point": v(t, k, [0.0, 5.0, 14.0]), "side": "below"}]}),
        ),
        fillet(
            "F",
            ["F2:copy(0)/F1:face(0)", "F2:copy(0)/F1:face(2)"],
            r * k,
        ),
    ]
}

/// What the last step answered.
#[derive(Debug)]
enum Answer {
    /// Built, and whether the deep check passed.
    Built {
        deep: bool,
    },
    Refused(String),
}

thread_local! {
    /// What the last step of the last [`answer`] on this thread cost: the
    /// kernel's booleans (counted for the whole process: exact with one
    /// test thread) and the work units of this thread.
    static COST: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
}

fn answer(steps: &Steps) -> Answer {
    let mut s = Session::new(Workspace::unrestricted());
    let (last, before) = steps.split_last().unwrap();
    for (id, params) in before {
        let made = s.execute(id, params).unwrap();
        assert_ne!(made["status"], "failed", "{id} {params}: {made}");
    }
    let booleans = craft3d_kernel::work::work().booleans;
    let (made, work) = craft3d_geom::allowance::measured(|| s.execute(last.0, &last.1));
    let booleans = craft3d_kernel::work::work().booleans - booleans;
    COST.with(|cost| *cost.borrow_mut() = format!("[{booleans} booleans, {work} units]"));
    let made = made.unwrap();
    if made["status"] == "failed" {
        return Answer::Refused(made["error"].as_str().unwrap().to_string());
    }
    let check = s.execute("document.check", &json!({"deep": true})).unwrap();
    Answer::Built {
        deep: check["ok"] == true,
    }
}

/// The largest size a too-large refusal says fits.
fn below(text: &str) -> Option<f64> {
    let at = text.find("that fits there is below ")?;
    let rest = text.get(at + "that fits there is below ".len()..)?;
    rest.split(' ').next()?.parse().ok()
}

/// Every case is answered before any is held, so that one run shows them
/// all; `wrong` collects what is not right.
fn said(label: &str, answer: &Answer) {
    let text = match answer {
        Answer::Built { deep } => format!("built, deep check ok: {deep}"),
        Answer::Refused(words) => words.chars().take(400).collect(),
    };
    let cost = COST.with(|cost| cost.borrow().clone());
    println!("{label}: {text} {cost}");
}

/// The radius `r` (at scale 1) is too large, with what fits said at
/// `limit`, the faces' own limit (at scale 1): never below it but for the
/// sixth digit a message cuts it to, and above it by no more than a
/// ten-thousandth (where the search finds the limit at a vertex beside it).
fn too_large(
    wrong: &mut Vec<String>,
    label: &str,
    make: Make,
    k: f64,
    t: Turn,
    r: f64,
    limit: f64,
) {
    let got = answer(&make(k, t, r));
    said(label, &got);
    let fits = match &got {
        Answer::Refused(words) if words.starts_with("radius: too large for edges.0") => {
            below(words)
        }
        _ => None,
    };
    match fits {
        Some(x) if x >= limit * k * (1.0 - 1e-5) && x <= limit * k * (1.0 + 1e-4) => {}
        _ => wrong.push(format!("{label}: not too large below {limit}: {got:?}")),
    }
}

/// The radius `r` (at scale 1) builds, and the body passes the deep check.
fn builds(wrong: &mut Vec<String>, label: &str, make: Make, k: f64, t: Turn, r: f64) {
    let got = answer(&make(k, t, r));
    said(label, &got);
    if !matches!(got, Answer::Built { deep: true }) {
        wrong.push(format!("{label}: not built and checked: {got:?}"));
    }
}

/// The radius `r` (at scale 1) is refused by the fillet in the words that
/// name the boolean's invalid body as its reason.
fn boolean_refused(wrong: &mut Vec<String>, label: &str, make: Make, k: f64, t: Turn, r: f64) {
    let got = answer(&make(k, t, r));
    said(label, &got);
    match &got {
        Answer::Refused(words)
            if words.starts_with("edges.0 (between ") && words.contains(BOOLEAN_REFUSED) => {}
        _ => wrong.push(format!("{label}: not the boolean's refusal: {got:?}")),
    }
}

fn held(wrong: &[String]) {
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// A rim whose wall ends below on the spline in which it meets a ball
/// (made for it) is refused as too large, with what fits where the wall is
/// narrowest, at the radii at which the first tool's boolean built an
/// invalid body (1.3, 1.45, 1.49, 1.5; on `main` before 2026-11-23 *"boolean
/// produced an invalid body"*); a radius just under that builds. The same
/// rim whose wall ends on an ellipse (a plane cut, exact) answers the same.
#[test]
fn a_rim_whose_wall_ends_on_a_spline_is_too_large_past_the_narrowest_of_the_wall() {
    let mut wrong = Vec::new();
    for (name, make) in [("ball", ball_rim as Make), ("plane", plane_rim as Make)] {
        for k in SCALES {
            for t in TURNS {
                for r in [1.3, 1.45, 1.49, 1.5] {
                    let label = format!("{name} rim k={k:e} {t:?} r={r}");
                    too_large(&mut wrong, &label, make, k, t, r, 1.0);
                }
                let label = format!("{name} rim k={k:e} {t:?} r=0.9999");
                builds(&mut wrong, &label, make, k, t, 0.9999);
            }
        }
    }
    held(&wrong);
}

/// Random designs 103578 and 110541 (group B, where it was first
/// described), refused as *"boolean produced an invalid body"* on `main`
/// before 2026-11-23, are too large, with what fits where the faces give
/// it: the narrowest of 103578's wall, 5 sqrt(3) - 7.5, and where the
/// base of 110541's cone comes nearest the hole; just under that builds.
#[test]
fn seeds_103578_and_110541_are_too_large_with_what_their_faces_leave() {
    let narrowest = 5.0 * 3.0_f64.sqrt() - 7.5;
    let nearest = 7.5 - (2.5_f64 * 2.5 + 0.02 * 0.02).sqrt() - 2.5;
    let mut wrong = Vec::new();
    for k in SCALES {
        for t in TURNS {
            for r in [1.17, 1.2, 1.25, 1.3] {
                let label = format!("103578 k={k:e} {t:?} r={r}");
                too_large(&mut wrong, &label, s103578, k, t, r, narrowest);
            }
            let label = format!("103578 k={k:e} {t:?} just under");
            builds(&mut wrong, &label, s103578, k, t, narrowest * 0.9999);
            for r in [4.5, 4.99, 5.0] {
                let label = format!("110541 k={k:e} {t:?} r={r}");
                too_large(&mut wrong, &label, s110541, k, t, r, nearest);
            }
            let label = format!("110541 k={k:e} {t:?} just under");
            builds(&mut wrong, &label, s110541, k, t, nearest * 0.9999);
        }
    }
    held(&wrong);
}

/// Random design 124545: at the radii whose first tool's boolean built an
/// invalid body the fillet is refused by the boolean, in words that name
/// it, and not as too large: the boolean also fails at radii that fit, so
/// the search's builds say nothing of what fits. What fits is up to
/// 2.39655, where the fillet would take the whole cap: a chamfer of the
/// rim, whose tool has no tangent edge, finds it, and fillets of 0.9 and
/// 1.3 build (on `main`, a search from 1 said *"below 0.6528"*).
#[test]
fn seed_124545_is_refused_by_the_boolean_whose_faults_lie_below_what_fits() {
    let cap = 7.5 - ((1.25_f64 * 1.25 + 7.5 * 7.5).sqrt() - 2.5);
    let mut wrong = Vec::new();
    for r in [1.25, 2.39, 2.45, 2.5, 3.0] {
        let label = format!("124545 k=1 r={r}");
        boolean_refused(&mut wrong, &label, s124545, 1.0, Turn::None, r);
    }
    for (k, t) in [
        (1e-3, Turn::None),
        (1e3, Turn::None),
        (1e6, Turn::None),
        (1.0, Turn::Cycle),
    ] {
        let label = format!("124545 k={k:e} {t:?} r=2.5");
        boolean_refused(&mut wrong, &label, s124545, k, t, 2.5);
    }
    for r in [0.9, 1.3] {
        let label = format!("124545 k=1 r={r}");
        builds(&mut wrong, &label, s124545, 1.0, Turn::None, r);
    }
    let chamfer = answer(&c124545(1.0, Turn::None, 2.5));
    said("124545 chamfer 2.5", &chamfer);
    match &chamfer {
        Answer::Refused(words)
            if below(words).is_some_and(|x| x >= cap * (1.0 - 1e-5) && x <= cap * (1.0 + 1e-4)) => {
        }
        _ => wrong.push(format!(
            "124545 chamfer: not too large below {cap}: {chamfer:?}"
        )),
    }
    held(&wrong);
}

/// Random designs 121649 and 128676: no smaller radius builds as the
/// search tries them (the cone reaches the rim of 121649 at its vertex; the
/// fillet of 128676 by the hole's own radius is a piece of a ball), and the
/// fillet is refused by the boolean in words that name it, where it was
/// *"boolean produced an invalid body"* on `main` before 2026-11-23.
#[test]
fn seeds_121649_and_128676_are_refused_by_the_boolean_in_the_fillets_words() {
    let mut wrong = Vec::new();
    for r in [0.62, 0.625, 0.63, 0.7, 0.8] {
        let label = format!("121649 k=1 r={r}");
        boolean_refused(&mut wrong, &label, s121649, 1.0, Turn::None, r);
    }
    for k in [1e-3, 1e3, 1e6] {
        let label = format!("121649 k={k:e} r=0.625");
        boolean_refused(&mut wrong, &label, s121649, k, Turn::None, 0.625);
    }
    for k in SCALES {
        for t in TURNS {
            let label = format!("128676 k={k:e} {t:?} r=5");
            boolean_refused(&mut wrong, &label, s128676, k, t, 5.0);
        }
    }
    held(&wrong);
}

/// A cylinder of radius 5 about Z from z = 10 to 15 less a ball about
/// (0, a, b) of radius `rho` (the ball clear of the top): the wall's lower
/// border is highest at y = 5, z = b + sqrt(rho² - (a - 5)²).
fn ball_rim_at(k: f64, t: Turn, r: f64, [a, b, rho]: [f64; 3]) -> Steps {
    let mut steps = ball_rim(k, t, r);
    steps[2] = (
        "feature.sphere",
        json!({"name": "Ball", "position": v(t, k, [0.0, a, b]), "radius": rho * k}),
    );
    steps
}

/// [`s103578`] with the torus's minor radius `minor`: the wall's lower
/// border is highest at z = 22.5 - sqrt(minor² - 25).
fn s103578_minor(k: f64, t: Turn, r: f64, minor: f64) -> Steps {
    let mut steps = s103578(k, t, r);
    steps[1] = (
        "feature.torus",
        json!({"name": "Tor", "position": v(t, k, [0.0, 0.0, 10.0]), "axis": axis(t, [0.0, 1.0, 0.0]), "major": 12.5 * k, "minor": minor * k}),
    );
    steps
}

/// [`s110541`] with the cone's base about (cx, cy) and its height `h`.
fn s110541_at(k: f64, t: Turn, r: f64, [cx, cy, h]: [f64; 3]) -> Steps {
    let mut steps = s110541(k, t, r);
    steps[2] = (
        "feature.cone",
        json!({"name": "Cone", "position": v(t, k, [cx, cy, 3.75]), "axis": axis(t, [0.0, 0.0, 1.0]), "radius": 7.5 * k, "height": h * k, "angle": "0 deg"}),
    );
    steps
}

fn ball_rim_6_8_6(k: f64, t: Turn, r: f64) -> Steps {
    ball_rim_at(k, t, r, [6.0, 8.0, 6.0])
}

fn torus_rim_9_5(k: f64, t: Turn, r: f64) -> Steps {
    s103578_minor(k, t, r, 9.5)
}

fn cone_hole_off(k: f64, t: Turn, r: f64) -> Steps {
    s110541_at(k, t, r, [-2.5, 0.5, 12.5])
}

fn cone_hole_short(k: f64, t: Turn, r: f64) -> Steps {
    s110541_at(k, t, r, [-2.5, 0.02, 6.0])
}

/// What fits the hole's rim of [`cone_hole_short`]: the lowest point of
/// the cone's meeting with the hole, h (1 - (|c| + 2.5) / 7.5), 1.999936
/// (the base's circle leaves 2.49992).
fn short_cone_limit() -> f64 {
    let c = (2.5_f64 * 2.5 + 0.02 * 0.02).sqrt();
    6.0 * (1.0 - (c + 2.5) / 7.5)
}

/// The configuration of group B made anew (the review of 2026-11-23):
/// a rim whose wall ends below on a spline (a ball of another size and
/// place, a torus of another minor radius), and a hole's rim in a cone's
/// base where the cone is moved off the hole or so short that the wall's
/// upper border (cone ∩ hole) is nearer than the base's circle. At these
/// radii (multiples of the limit) the first tool's boolean built an invalid
/// body at scale 1, unturned; each is too large, with what fits worked
/// out by hand: the ball's 15 - b - sqrt(rho² - (a - 5)²), the torus's
/// 15 - (22.5 - sqrt(minor² - 25)), the cone's least of 7.5 - |c| - 2.5
/// (its base's circle) and h (1 - (|c| + 2.5) / 7.5) (the lowest point of
/// its meeting with the hole); just under it (0.999 of it, the short cone
/// 0.98: [`owed_a_short_cone_is_never_said_too_large_below_a_radius_that_builds`])
/// builds and passes the deep check. At every scale and turn, whether or
/// not the boolean's body is invalid there.
#[test]
fn rims_made_anew_whose_wall_ends_on_a_spline_are_too_large_past_what_the_faces_leave() {
    let ball = 15.0 - 8.0 - (36.0_f64 - 1.0).sqrt();
    let torus = 15.0 - (22.5 - (9.5_f64 * 9.5 - 25.0).sqrt());
    let off = 7.5 - (2.5_f64 * 2.5 + 0.5 * 0.5).sqrt() - 2.5;
    let short = short_cone_limit();
    let cases: [(&str, Make, f64, &[u32], f64); 4] = [
        (
            "ball (0, 6, 8) r 6",
            ball_rim_6_8_6,
            ball,
            &[51, 55, 65],
            0.999,
        ),
        (
            "torus minor 9.5",
            torus_rim_9_5,
            torus,
            &[54, 57, 63],
            0.999,
        ),
        (
            "cone off (-2.5, 0.5)",
            cone_hole_off,
            off,
            &[89, 90, 93],
            0.999,
        ),
        ("cone of height 6", cone_hole_short, short, &[54, 61], 0.98),
    ];
    let mut wrong = Vec::new();
    for (name, make, limit, factors, under) in cases {
        for k in SCALES {
            for t in TURNS {
                for &i in factors {
                    // As the review's sweep made them: the limit times i / 50.
                    let r = limit * f64::from(i) * 0.02;
                    let label = format!("{name} k={k:e} {t:?} r={r}");
                    too_large(&mut wrong, &label, make, k, t, r, limit);
                }
                let label = format!("{name} k={k:e} {t:?} r={} (just under)", under * limit);
                builds(&mut wrong, &label, make, k, t, under * limit);
            }
        }
    }
    // The release note's example: 103578's rim at 1.16, under 5 sqrt(3) -
    // 7.5 = 1.160254, builds.
    builds(
        &mut wrong,
        "103578 k=1 r=1.16",
        s103578,
        1.0,
        Turn::None,
        1.16,
    );
    held(&wrong);
}

/// Owed, found by the review of 2026-11-23: the short cone's rim
/// ([`cone_hole_short`]) at 0.999 of what fits, unturned and turned X to Y
/// to Z, at every scale, is said too large, *"below 1.99793"* (just under
/// the radius asked), by the search on builds alone from the boolean's
/// other refusal near the tangency, on `main` (its program, 1f196da4) as on
/// the branch; turned 30 degrees about Z it builds, and so does the limit
/// itself, 1.999936, which passes the deep check. The right answer is a
/// body.
#[test]
#[ignore = "owed: the search's answer from the boolean's other refusals, where they lie below what fits"]
fn owed_a_short_cone_is_never_said_too_large_below_a_radius_that_builds() {
    let limit = short_cone_limit();
    let mut wrong = Vec::new();
    builds(
        &mut wrong,
        "short cone k=1 at the limit",
        cone_hole_short,
        1.0,
        Turn::None,
        limit,
    );
    for k in SCALES {
        for t in [Turn::None, Turn::Cycle] {
            let label = format!("short cone k={k:e} {t:?} r={}", 0.999 * limit);
            builds(&mut wrong, &label, cone_hole_short, k, t, 0.999 * limit);
        }
    }
    held(&wrong);
}

/// Owed: 124545 turned 30 degrees about Z, and at radii where the
/// boolean refuses in other words (at the edge of tolerance), is said to be
/// too large with a largest radius that fits found by builds alone, below
/// what fits (2.39655): on `main`, *"below 2.06238"* at 2.5 turned, *"below
/// 0.6528"* at 1, *"below 0.5109"* at 1.5. The right answer there is a body,
/// too large with what fits from 2.39655, or the boolean's refusal.
#[test]
#[ignore = "owed: the search's answer from the boolean's other refusals, where they lie below what fits"]
fn owed_seed_124545_is_never_said_too_large_below_what_fits() {
    let cap = 7.5 - ((1.25_f64 * 1.25 + 7.5 * 7.5).sqrt() - 2.5);
    let mut wrong = Vec::new();
    for (t, r) in [
        (Turn::Z30, 2.5),
        (Turn::None, 1.0),
        (Turn::None, 1.5),
        (Turn::None, 2.0),
    ] {
        let got = answer(&s124545(1.0, t, r));
        let label = format!("124545 {t:?} r={r}");
        said(&label, &got);
        match &got {
            Answer::Refused(words) if below(words).is_some_and(|x| x < cap * (1.0 - 1e-5)) => {
                wrong.push(format!("{label}: too large below what fits: {words}"));
            }
            _ => {}
        }
    }
    held(&wrong);
}
