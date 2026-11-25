//! A fillet or a chamfer says it is too large below a size X only where it
//! is shown (the developer, 2026-11-24; `docs/design/blends.md` › *Too
//! large only where it is shown*), and says what is shown: *"the R mm
//! asked does not fit …, nor the radii (distances) tried from X mm up to
//! it; the largest found to fit is just below X mm"* (since 2026-11-25,
//! the reviews' F1 and F4: nothing larger than the size asked is tried,
//! and seven sizes between, not every one). It is shown where an event of the faces (the border reaching something of
//! its face) gives X, a build just before it succeeds and the seven sizes
//! evenly between X and the size asked fail too; or, where no event
//! explains the failure, where the result itself shows no room (the blend
//! takes a face away, reaches a face it should not, comes out in pieces)
//! at the size asked, at the X a bisection on the builds ends on, and at
//! the seven sizes between. Otherwise the refusal at the size asked is
//! said as it is: the boolean's, in the blend's words, or the blend's own.
//!
//! Until 2026-11-25 the search also said the X a bisection ended on
//! whatever the failures were (the boolean's at sizes that fit, too), and
//! "below 0" where nothing it tried built.
//!
//! The oracles are builds and closed forms: every X said here is held to
//! the faces' own limit worked out by hand and against builds of sizes
//! between X and the size asked and past it (none builds), and a size
//! just under the limit builds and passes the deep check; an answer that
//! was false on `main` (22f0bb13) is held against a larger size that
//! builds and passes the deep check. Each case at scales 1e-3 to 1e6 (the
//! document's tolerance 1e-6 times the scale) and turned.

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

type Steps = Vec<(&'static str, Value)>;

/// A body made at scale `k`, turned by `t`, and the two faces of the edge
/// blended.
type Made = (Steps, [&'static str; 2]);
type Make = fn(f64, Turn) -> Made;

fn new(k: f64) -> (&'static str, Value) {
    ("file.new", json!({"units": "mm", "tolerance": 1e-6 * k}))
}

/// Random design 124545 as reduced: a torus about Z through (0, 8.75), R
/// 7.5, r 2.5, at z = -7.5, cut to the cylinder of radius 7.5 about X from
/// x = 0 to 10; the rim at x = 0, between the cap (a lens) and the wall.
/// The cap leaves room up to 7.5 - (sqrt(1.25² + 7.5²) - 2.5) = 2.396547,
/// past which a blend of the rim would take the whole cap.
fn s124545(k: f64, t: Turn) -> Made {
    (
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
        ],
        ["F3:copy(0)/F2:face(0)", "F3:copy(0)/F2:face(1)"],
    )
}

fn cap() -> f64 {
    7.5 - ((1.25_f64 * 1.25 + 7.5 * 7.5).sqrt() - 2.5)
}

/// Random design 110541 as reduced, with the cone's base about (cx, cy)
/// at z = 3.75 and its height `h`: a cone less a hole of radius 2.5 about
/// Z; the hole's rim in the base.
fn cone_hole(k: f64, t: Turn, [cx, cy, h]: [f64; 3]) -> Made {
    (
        vec![
            new(k),
            (
                "feature.cylinder",
                json!({"name": "Hole", "position": v(t, k, [0.0, 0.0, 0.0]), "axis": axis(t, [0.0, 0.0, 1.0]), "radius": 2.5 * k, "height": 20.0 * k}),
            ),
            (
                "feature.cone",
                json!({"name": "Cone", "position": v(t, k, [cx, cy, 3.75]), "axis": axis(t, [0.0, 0.0, 1.0]), "radius": 7.5 * k, "height": h * k, "angle": "0 deg"}),
            ),
            (
                "feature.boolean",
                json!({"name": "N3", "op": "subtract", "target": "Cone", "tool": "Hole"}),
            ),
        ],
        ["F3:copy(0)/F2:face(1)", "F3:copy(1)/F1:face(0)"],
    )
}

/// The cone 12.5 high about (-2.5, 0.02): the base's outer circle comes
/// nearest the hole's axis at 7.5 - sqrt(2.5² + 0.02²), so what fits is
/// below that less 2.5.
fn s110541(k: f64, t: Turn) -> Made {
    cone_hole(k, t, [-2.5, 0.02, 12.5])
}

/// The cone only 6 high: what fits the hole's rim is where the cone's
/// meeting with the hole is lowest, h (1 - (|c| + 2.5) / 7.5) = 1.999936.
fn short_cone(k: f64, t: Turn) -> Made {
    cone_hole(k, t, [-2.5, 0.02, 6.0])
}

fn short_cone_limit() -> f64 {
    let c = (2.5_f64 * 2.5 + 0.02 * 0.02).sqrt();
    6.0 * (1.0 - (c + 2.5) / 7.5)
}

/// Random design 103578 as reduced: a cylinder of radius 5 about Z from z
/// = 10 to 15 cut to a torus about Y (R 12.5, r 10); the wall's lower
/// border is highest at z = 22.5 - sqrt(75): what fits the top rim is
/// below 5 sqrt(3) - 7.5.
fn s103578(k: f64, t: Turn) -> Made {
    (
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
        ],
        ["F3:copy(0)/F2:face(0)", "F3:copy(0)/F2:face(2)"],
    )
}

/// A cylinder of radius 5 about Z from z = 10 to 15 less a ball about
/// (0, 8, 8) of radius sqrt(45): the wall's lower border is highest at y =
/// 5, z = 14, so what fits the top rim is below 1.
fn ball_rim(k: f64, t: Turn) -> Made {
    (
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
        ],
        ["F3:copy(0)/F1:face(0)", "F3:copy(0)/F1:face(2)"],
    )
}

/// The same cylinder cut by the plane z = 11.5 + y / 2: the wall's lower
/// border is an ellipse, highest at y = 5, z = 14; what fits is below 1.
fn plane_rim(k: f64, t: Turn) -> Made {
    (
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
        ],
        ["F2:copy(0)/F1:face(0)", "F2:copy(0)/F1:face(2)"],
    )
}

/// What the blend answered.
#[derive(Debug)]
enum Answer {
    /// Built, and whether the deep check passed (when it was run).
    Built {
        deep: Option<bool>,
    },
    Refused(String),
}

impl Answer {
    fn built(&self) -> bool {
        matches!(self, Answer::Built { .. })
    }

    /// The size a too-large refusal says is the least that does not fit
    /// (*"…; the largest found to fit is just below X mm"*).
    fn below(&self) -> Option<f64> {
        let Answer::Refused(words) = self else {
            return None;
        };
        if !words.contains(": too large for edges") {
            return None;
        }
        words
            .rsplit("below ")
            .next()?
            .strip_suffix(" mm")?
            .parse()
            .ok()
    }

    fn words(&self) -> &str {
        match self {
            Answer::Built { .. } => "",
            Answer::Refused(words) => words,
        }
    }
}

/// The fillet (radius `r`) or the chamfer (distance `r`) of the edge of
/// `make` at scale 1, scaled by `k`; the deep check of what it built when
/// `deep`.
fn blend(make: Make, k: f64, t: Turn, r: f64, chamfer: bool, deep: bool) -> Answer {
    let (setup, faces) = make(k, t);
    let mut s = Session::new(Workspace::unrestricted());
    for (id, params) in &setup {
        let done = s.execute(id, params).unwrap();
        assert_ne!(done["status"], "failed", "{id} {params}: {done}");
    }
    let (id, params) = if chamfer {
        (
            "feature.chamfer",
            json!({"edges": [faces], "name": "Blend", "distance": r * k}),
        )
    } else {
        (
            "feature.fillet",
            json!({"chain": false, "edges": [faces], "name": "Blend", "radius": r * k}),
        )
    };
    let done = s.execute(id, &params).unwrap();
    if done["status"] == "failed" {
        return Answer::Refused(done["error"].as_str().unwrap().to_string());
    }
    let deep =
        deep.then(|| s.execute("document.check", &json!({"deep": true})).unwrap()["ok"] == true);
    Answer::Built { deep }
}

/// Every case is answered before any is held, so that one run shows them
/// all; `wrong` collects what is not right.
fn said(label: &str, answer: &Answer) {
    let text = match answer {
        Answer::Built { deep } => format!("built, deep check {deep:?}"),
        Answer::Refused(words) => words.chars().take(300).collect(),
    };
    println!("{label}: {text}");
}

fn held(wrong: &[String]) {
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// A rim and its blend, with the faces' own limit worked out by hand.
#[derive(Clone, Copy)]
struct Rim {
    name: &'static str,
    make: Make,
    chamfer: bool,
    limit: f64,
    /// The size, as a multiple of the limit, that builds just under it.
    under: f64,
}

/// A rim whose largest size is said: at `factors` times its limit, at
/// `scales` and `turns`, the blend is too large with X said within the
/// sixth digit below the limit and a ten-thousandth above it (an event
/// found at a vertex beside the nearest point), and no size tried at or
/// above X builds: five evenly between X and the size asked, and half as
/// much again as the size asked. Its `under` builds and passes the deep
/// check.
fn the_size_said_is_true(rim: &Rim, factors: &[f64], scales: &[f64], turns: &[Turn]) {
    let Rim {
        name,
        make,
        chamfer,
        limit,
        under,
    } = *rim;
    let mut wrong = Vec::new();
    for &k in scales {
        for &t in turns {
            for &f in factors {
                let r = f * limit;
                let label = format!("{name} k={k:e} {t:?} at {r}");
                let got = blend(make, k, t, r, chamfer, false);
                said(&label, &got);
                let Some(x) = got.below().map(|x| x / k) else {
                    wrong.push(format!("{label}: not too large with a size: {got:?}"));
                    continue;
                };
                // Said as shown: the size asked and those tried from X.
                if !got.words().contains(" asked does not fit") {
                    wrong.push(format!("{label}: not said up to the size asked: {got:?}"));
                }
                if !(limit * (1.0 - 1e-5)..=limit * (1.0 + 1e-4)).contains(&x) {
                    wrong.push(format!("{label}: below {x}, not the faces' {limit}"));
                }
                let from = x * (1.0 + 2e-5);
                let tried: Vec<f64> = (1..=5)
                    .map(|i| from + (r - from) * f64::from(i) / 6.0)
                    .chain([1.5 * r])
                    .collect();
                for s in tried {
                    let past = blend(make, k, t, s, chamfer, false);
                    if past.built() {
                        wrong.push(format!("{label}: below {x}, but {s} builds"));
                    }
                }
            }
            let label = format!("{name} k={k:e} {t:?} at {} (under)", under * limit);
            let got = blend(make, k, t, under * limit, chamfer, true);
            said(&label, &got);
            if !matches!(got, Answer::Built { deep: Some(true) }) {
                wrong.push(format!("{label}: not built and checked: {got:?}"));
            }
        }
    }
    held(&wrong);
}

/// Events: where the wall of a rim ends below on a spline (a torus's cut,
/// 103578), and on a plane's ellipse: the border reaches the wall's
/// highest point. Said, as on `main`, with the sizes between failing.
#[test]
fn a_rim_cut_by_a_torus_is_too_large_past_the_narrowest_of_its_wall() {
    let rim = |chamfer| Rim {
        name: if chamfer { "103578 chamfer" } else { "103578" },
        make: s103578,
        chamfer,
        limit: 5.0 * 3.0_f64.sqrt() - 7.5,
        under: 0.999,
    };
    the_size_said_is_true(&rim(false), &[1.05, 1.6], &SCALES, &TURNS);
    the_size_said_is_true(&rim(true), &[1.3], &[1.0], &[Turn::None, Turn::Z30]);
}

#[test]
fn a_rim_cut_by_a_ball_or_a_plane_is_too_large_past_the_highest_of_its_wall_border() {
    let rim = |name, make, chamfer| Rim {
        name,
        make,
        chamfer,
        limit: 1.0,
        under: 0.999,
    };
    let ball = rim("ball rim", ball_rim as Make, false);
    the_size_said_is_true(&ball, &[1.05, 1.6], &SCALES, &TURNS);
    let plane = rim("plane rim", plane_rim as Make, false);
    the_size_said_is_true(&plane, &[1.3], &SCALES, &TURNS);
    let chamfer = rim("ball rim chamfer", ball_rim as Make, true);
    the_size_said_is_true(&chamfer, &[1.3], &[1.0], &[Turn::None, Turn::Z30]);
}

/// The base's circle of a cone, nearest the hole beside its vertex
/// (110541), and the lowest point where a short cone meets the hole: the
/// border reaches them. For the short cone the size just under is 0.98 of
/// the limit: at 0.999 the boolean refuses (below).
#[test]
fn a_holes_rim_in_a_cone_is_too_large_past_what_the_cone_leaves() {
    let cone = Rim {
        name: "110541",
        make: s110541,
        chamfer: false,
        limit: 7.5 - (2.5_f64 * 2.5 + 0.02 * 0.02).sqrt() - 2.5,
        under: 0.999,
    };
    the_size_said_is_true(&cone, &[1.05, 1.6], &SCALES, &TURNS);
    let short = Rim {
        name: "short cone",
        make: short_cone,
        chamfer: false,
        limit: short_cone_limit(),
        under: 0.98,
    };
    the_size_said_is_true(&short, &[1.08, 1.22], &SCALES, &TURNS);
}

/// No event: a chamfer of 124545's rim takes the whole cap past 2.396547
/// (the cap is a lens whose far point is a maximum of the measure across
/// the rim, no event), and the result shows it, *"the chamfer takes away
/// the whole of a face next to the edge"*, at the size asked, where the
/// bisection ends and between: said, as on `main`, to the closed form.
#[test]
fn a_chamfer_that_takes_a_whole_lens_is_too_large_where_the_result_shows_it() {
    let lens = Rim {
        name: "124545 chamfer",
        make: s124545,
        chamfer: true,
        limit: cap(),
        under: 0.999,
    };
    the_size_said_is_true(&lens, &[1.0432, 1.25], &[1.0], &[Turn::None, Turn::Z30]);
}

/// What `main` (22f0bb13) said below a radius that builds: the short
/// cone's rim at 0.999 of what fits (1.99794), unturned and turned X to Y
/// to Z, at every scale, was *"too large … below 1.99793"* (a bisection on
/// the boolean's refusals, *"a curve that runs along a surface within the
/// tolerance"*), while 0.9995 of it (1.99894) builds and passes the deep
/// check. It is now the boolean's refusal in the fillet's words. (Turned 30
/// degrees about Z it builds; the right answer is a body, owed:
/// `blend_invalid_tool.rs` › `owed_a_short_cone_is_never_said_too_large_below_a_radius_that_builds`.)
#[test]
fn a_size_found_where_the_builds_fail_at_the_boolean_is_not_said() {
    let limit = short_cone_limit();
    let mut wrong = Vec::new();
    for k in SCALES {
        for t in [Turn::None, Turn::Cycle] {
            let label = format!("short cone k={k:e} {t:?} at 0.999");
            let got = blend(short_cone, k, t, 0.999 * limit, false, false);
            said(&label, &got);
            let words = got.words();
            if got.below().is_some()
                || !words.starts_with("edges.0 (between ")
                || !words.contains("the boolean that cuts this fillet refused it")
            {
                wrong.push(format!("{label}: not the boolean's refusal: {got:?}"));
            }
            let label = format!("short cone k={k:e} {t:?} at 0.9995");
            let got = blend(short_cone, k, t, 0.9995 * limit, false, true);
            said(&label, &got);
            if !matches!(got, Answer::Built { deep: Some(true) }) {
                wrong.push(format!("{label}: not built and checked: {got:?}"));
            }
        }
    }
    held(&wrong);
}
