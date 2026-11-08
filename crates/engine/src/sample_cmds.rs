//! Proof over the range (Jigs, phase 4; docs/design/verification.md ›
//! *Proof over the range*): `document.sample` sets the public parameters
//! of a copy of the active document to the defaults, to the corners of
//! their box and to seeded random points, and checks every point as
//! `document.check` does, so a Jig's author knows that every value a form
//! lets through gives a valid model. The open document and its undo
//! history are never touched.
//!
//! A point is a position per parameter: its default, or a fraction `u` of
//! its range (0 and 1 at the corners). The values are worked out from the
//! fractions against the ranges *at that point*: a range that uses other
//! parameters (a bore less than the root diameter) is evaluated after
//! them, again until nothing moves. The fractions come from a seeded
//! generator of our own (SplitMix64), one stream for the corners and one
//! for the random points, so a seed gives the same points and the same
//! verdict everywhere; the times are reported, and only a `budgetMs`
//! makes them part of the verdict. Points are made one at a time, as the
//! run reaches them, so a run stopped by its time limit made no more than
//! it checked.

use crate::commands::{Run, fail, spec};
use crate::param_cmds::{Range, nearest_in_range, outside, prepare_set, ranges, value_text};
use crate::params::Params;
use crate::{CommandError, CommandSpec, OpenDocument, Session, Workspace};
use craft3d_doc::{Document, FeatureId, ParamKind};
use craft3d_topo::BodyKind;
use serde_json::{Map, Value, json};
use std::collections::BTreeSet;

pub(crate) static SPECS: &[CommandSpec] = &[spec!(
    "document.sample",
    "Check Over the Range",
    r#"{"samples": integer = 32, "seed": integer = 1, "corners": boolean = true, "maxCorners": integer = 64, "choices": boolean = true, "deep": boolean = false, "strict": boolean = false, "budgetMs": number = none, "timeLimitMs": number or null = 60000, "maxFailures": integer = 100, "values": boolean = false}"#,
    "Proves a Jig over its parameter range: on a copy of the active document (the document and its undo history do not change), sets the public parameters (param.list) to a series of points and checks each one: the defaults; the corners of the box of the ranges (every parameter at its min or max, a boolean off or on, a choice its first or last; all 2^n of them while that is at most maxCorners (at most 4096), else maxCorners of them chosen by the seed, the two all-low and all-high corners always among them; samples does not cut the corners: maxCorners or corners: false does); with choices every other choice of each choice parameter, the rest at the defaults (a choice outside that parameter's own range at the defaults is skipped: counts.skipped); then samples (at most 10000) random points chosen by the seed (a number uniform over its range and on its step, an integer, a choice or a boolean uniform over its values within its min and max, so a boolean whose max is another parameter is off while that is; the same points whatever corners are asked for). A range that uses other parameters is evaluated at each point after them; ranges that keep moving each other fail the point (did not settle). Away from the defaults, a parameter the point leaves at its default that is outside its range there (a choice that narrows another range) is moved to the nearest value in it, as a form and param.set with clamp move it, and listed under the point's clamped ({parameter, from, to}). A side of a range that is not given is the default. Each point is set as param.set sets it (a point it refuses, such as a range that is empty there, fails with the reason), regenerated without the open document's cache (timed: ms; the kernel's trees of freeform surfaces are shared within the process, so a surface the open document has met may be quicker), and checked as document.check (deep with deep) checks: no failed variables or features, valid bodies, no invalid parameter, no broken assertion (an assertion neither passed nor failed, notProven, does not fail the point but does not pass it either: the point counts under unproven; with strict it fails the point) (the bodies measured, mass properties from the exact faces, at the defaults and the corners, and with deep at every point: measuring costs several times a regeneration); and every feature whose body is a solid at the defaults has a solid there too. With budgetMs a point whose regeneration takes longer fails as well; without it times are reported and never part of the verdict. timeLimitMs bounds the whole run (checked between points; null: no limit, as craft3d-cli check-range runs it); in a browser, which has no clock, a run stops after 128 points instead. A run that stops early has complete false, stoppedBy (timeLimit, pointCap, or caller when the host asked it to stop) and ok false when a point failed, else null: never true. The same seed gives the same points and the same verdict. What the kernel kept for the points (intersections, projections, meshes) is let go of at the end of the run, or after each point where the host asks for that (craft3d-cli check-range), so a long run holds the memory of one point. Returns ok (true when no point failed: unproven points do not make it false unless strict), strict, complete, the number of points by kind (cornersInBox: 2^n, of which corner were tried), passed (no problem and every assertion proven), unproven (no problem, some assertion not proven), failed (how many failed), failures (the first maxFailures: index, kind default|corner|choice|random, values: every parameter as param.set takes it, to reproduce the point, clamped, problems, ms), unprovenPoints (the first maxFailures: index, kind, values, assertions [{assertion, message}], ms), points ({index, kind, ok, ms, unproven: how many assertions, when any, clamped when any} each; with values also its values), the parameters, unbounded (numbers without min and max: only their default is tried) and timing (medianMs, maxMs, totalMs, slowest point; null where there is no clock, as in a browser).",
    doc: true,
    document_sample
)];

/// Most random points.
const MAX_SAMPLES: u64 = 10_000;
/// Most corners.
const MAX_CORNERS: u64 = 4096;
/// Most points of the choices.
const MAX_CHOICE_POINTS: usize = 4096;
/// Most failures listed.
const MAX_FAILURES: u64 = 10_000;
/// How many tries a seeded subset of corners gets per corner wanted.
const CORNER_TRIES: usize = 8;
/// The time limit of a run when none is given: a window or an agent
/// waits no longer than this for an answer.
const DEFAULT_TIME_LIMIT_MS: f64 = 60_000.0;
/// Most points of a run: in a browser, which has no clock to hold a time
/// limit to and one thread the page waits on, few.
#[cfg(target_arch = "wasm32")]
const MAX_POINTS: Option<usize> = Some(128);
#[cfg(not(target_arch = "wasm32"))]
const MAX_POINTS: Option<usize> = None;

/// SplitMix64 (Steele, Lea and Flood, 2014): small, seeded, the same on
/// every platform.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in [0, 1), from the top 53 bits.
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1_u64 << 53) as f64
    }
}

/// Where one parameter is at a point.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Pos {
    Default,
    /// A fraction of its range, from 0 (min) to 1 (max).
    At(f64),
    /// A choice by its number.
    Choice(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Default,
    Corner,
    Choice,
    Random,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Default => "default",
            Kind::Corner => "corner",
            Kind::Choice => "choice",
            Kind::Random => "random",
        }
    }

    fn index(self) -> usize {
        match self {
            Kind::Default => 0,
            Kind::Corner => 1,
            Kind::Choice => 2,
            Kind::Random => 3,
        }
    }
}

struct Point {
    kind: Kind,
    /// One per parameter, in the order of [`ranges`].
    pos: Vec<Pos>,
}

/// Whether a parameter has a range to move in: a boolean, a choice, or a
/// number with a min or a max.
fn moves(range: &Range) -> bool {
    match range.kind {
        ParamKind::Boolean => true,
        ParamKind::Choice => range.choices > 1,
        ParamKind::Number | ParamKind::Integer => range.min.is_some() || range.max.is_some(),
    }
}

/// The value `u` picks of those on the grid `base + k * step` within
/// `[lo, hi]` (`param_cmds::step_grid`, so a point is a value `param.set`
/// takes), held to `[lo, hi]` (rounding must not step past an end: a
/// range that ends at 0 ends at 0, not at 5.6e-17). A range with no value
/// of the grid in it has only its ends, which are always taken.
fn pick(lo: f64, hi: f64, (base, step): (f64, f64), u: f64) -> Option<f64> {
    if !(hi >= lo && step > 0.0 && base.is_finite()) {
        return None;
    }
    // Within rounding of a grid value counts as on it.
    const SLACK: f64 = 1e-9;
    let first = ((lo - base) / step - SLACK).ceil();
    let last = ((hi - base) / step + SLACK).floor();
    if !(first.is_finite() && last.is_finite()) || last - first >= 9.0e15 {
        return None;
    }
    if first > last {
        return Some(if u < 0.5 { lo } else { hi });
    }
    let k = first + (u * (last - first + 1.0)).floor().min(last - first);
    Some((base + k * step).clamp(lo, hi))
}

/// The whole numbers from `lo` to `hi` (a boolean's 0 and 1, a choice's 1
/// to n) that also lie in `range`'s min and max: a boolean whose max is
/// another parameter (`opt` only when `big`) is off while that is 0, as
/// the form offers it.
fn whole_values(range: &Range, lo: f64, hi: f64) -> (f64, f64) {
    let lo = range.min.map_or(lo, |min| (min - 1e-9).ceil().max(lo));
    let hi = range.max.map_or(hi, |max| (max + 1e-9).floor().min(hi));
    (lo, hi)
}

/// The value of `range` at fraction `u`, in its unit; none when its range
/// is empty or does not evaluate (the point then keeps the default, and
/// `param.set` says what is wrong).
fn value_at(range: &Range, u: f64) -> Option<f64> {
    let u = u.clamp(0.0, 1.0);
    match range.kind {
        ParamKind::Boolean => {
            let (lo, hi) = whole_values(range, 0.0, 1.0);
            pick(lo, hi, (0.0, 1.0), u)
        }
        ParamKind::Choice => {
            let (lo, hi) = whole_values(range, 1.0, range.choices as f64);
            pick(lo, hi, (0.0, 1.0), u)
        }
        ParamKind::Integer => {
            let lo = range.min.or(range.default)?.ceil();
            let hi = range.max.or(range.default)?.floor();
            // Its grid, else every whole number.
            let grid = range.grid().unwrap_or((0.0, 1.0));
            pick(lo, hi, grid, u)
        }
        ParamKind::Number => {
            let lo = range.min.or(range.default)?;
            let hi = range.max.or(range.default)?;
            if hi < lo {
                return None;
            }
            match range.grid() {
                Some(grid) => pick(lo, hi, grid, u),
                None if u >= 1.0 => Some(hi),
                None => Some((lo + u * (hi - lo)).clamp(lo, hi)),
            }
        }
    }
}

/// The values of `point`, each as `param.set` takes it, in the order of
/// the parameters: worked out against the ranges at the point itself,
/// again until no value moves (a range that uses another parameter after
/// that one), at most once per parameter and once more. A parameter the
/// point leaves at its default (a choice point's other parameters) that
/// is outside its range there is moved to the nearest value of it, as a
/// form and `param.set {clamp: true}` move it (a choice that narrows
/// another range); not at the defaults themselves, which are checked as
/// they are. Each move is listed (`clamped`: {parameter, from, to}). A
/// choice point whose choice lies outside that parameter's own range at
/// the defaults is not made at all (`skip`): the form does not offer that
/// choice there, and moving it would only repeat another choice. With the
/// values as far as they got comes why they cannot be set, if they
/// cannot: a value the document refuses, or ranges that do not settle
/// (two ranges that push each other on).
fn resolve(base: &Document, point: &Point) -> Resolved {
    let mut clamped: Vec<(String, String, String)> = Vec::new();
    let (values, unset, skip) = resolve_into(base, point, &mut clamped);
    Resolved {
        values,
        unset,
        skip,
        clamped: clamped
            .into_iter()
            .map(|(name, from, to)| json!({"parameter": name, "from": from, "to": to}))
            .collect(),
    }
}

/// What [`resolve`] made of a point.
struct Resolved {
    values: Map<String, Value>,
    unset: Option<String>,
    skip: bool,
    clamped: Vec<Value>,
}

fn resolve_into(
    base: &Document,
    point: &Point,
    moves: &mut Vec<(String, String, String)>,
) -> (Map<String, Value>, Option<String>, bool) {
    let mut doc = base.clone();
    let first = ranges(&doc);
    let values_of = |doc: &Document| -> Map<String, Value> {
        first
            .iter()
            .map(|range| {
                let text = doc
                    .variable(&range.name)
                    .map(|v| v.expr.source().to_string())
                    .unwrap_or_else(|| range.default_source.clone());
                (range.name.clone(), Value::String(text))
            })
            .collect()
    };
    // Every parameter at its default first.
    for range in &first {
        if let Err(e) = doc.set_variable(&range.name, &range.default_source) {
            let why = format!("`{}`: its default: {e}", range.name);
            return (values_of(&doc), Some(why), false);
        }
    }
    let clamp = point.kind != Kind::Default;
    // Into the range, as a form clamps.
    let clamped = |range: &Range, value: f64| -> Option<f64> {
        (clamp && outside(range, value))
            .then(|| nearest_in_range(range, value))
            .flatten()
    };
    let rounds = first.len().saturating_add(1);
    for round in 0..rounds {
        let now = ranges(&doc);
        let mut moved = false;
        for (range, pos) in now.iter().zip(&point.pos) {
            let mut from = None;
            let number = match pos {
                Pos::Default => range.value.and_then(|v| {
                    let to = clamped(range, v)?;
                    from = Some(value_text(range, v));
                    Some(to)
                }),
                Pos::At(u) => value_at(range, *u),
                Pos::Choice(k) => {
                    let k = *k as f64;
                    // Against its range at the defaults: a choice the
                    // form does not offer there is no point.
                    if round == 0 && outside(range, k) {
                        return (values_of(&doc), None, true);
                    }
                    Some(k)
                }
            };
            let text = number.map(|v| value_text(range, v));
            let Some(text) = text else { continue };
            if let Some(from) = from {
                match moves.iter_mut().find(|(name, _, _)| *name == range.name) {
                    Some(entry) => entry.2.clone_from(&text),
                    None => moves.push((range.name.clone(), from, text.clone())),
                }
            }
            let same = doc
                .variable(&range.name)
                .is_some_and(|v| v.expr.source() == text);
            if !same {
                if let Err(e) = doc.set_variable(&range.name, &text) {
                    let why = format!("`{}` = `{text}`: {e}", range.name);
                    return (values_of(&doc), Some(why), false);
                }
                moved = true;
            }
        }
        if !moved {
            return (values_of(&doc), None, false);
        }
    }
    let why = format!(
        "the ranges did not settle after {rounds} rounds (ranges that use each other keep moving the values); the values are the last ones"
    );
    (values_of(&doc), Some(why), false)
}

/// Streams of the generator, so the random points stay the same whatever
/// corners are asked for.
const CORNER_STREAM: u64 = 0xC0FF_EE00_C0A1_E215;
const RANDOM_STREAM: u64 = 0x5A3D_0E5E_ED00_1234;

/// The points of a run, made one at a time in a fixed order: the
/// defaults, the corners, the choices, the random points.
struct Points<'a> {
    list: &'a [Range],
    /// Which parameters move.
    mask: Vec<bool>,
    /// How many move.
    movers: usize,
    stage: Kind,
    done: bool,
    // Corners.
    corners: bool,
    max_corners: usize,
    /// All 2^n corners, when they fit in `max_corners`.
    all: Option<u64>,
    corners_made: usize,
    corner_seen: BTreeSet<Vec<bool>>,
    corner_tries: usize,
    corner_rng: Rng,
    // Choices.
    choices: bool,
    choice_at: (usize, usize),
    choices_made: usize,
    // Random points.
    samples: usize,
    random_made: usize,
    random_rng: Rng,
}

impl<'a> Points<'a> {
    fn new(
        list: &'a [Range],
        seed: u64,
        corners: bool,
        max_corners: usize,
        choices: bool,
        samples: usize,
    ) -> Self {
        let mask: Vec<bool> = list.iter().map(moves).collect();
        let movers = mask.iter().filter(|m| **m).count();
        let all = (movers > 0 && movers < 64)
            .then(|| 1_u64 << movers)
            .filter(|&all| all <= max_corners as u64);
        Points {
            list,
            mask,
            movers,
            stage: Kind::Default,
            done: false,
            corners: corners && movers > 0 && max_corners > 0,
            max_corners,
            all,
            corners_made: 0,
            corner_seen: BTreeSet::new(),
            corner_tries: 0,
            corner_rng: Rng(seed ^ CORNER_STREAM),
            choices,
            choice_at: (0, 1),
            choices_made: 0,
            samples,
            random_made: 0,
            random_rng: Rng(seed ^ RANDOM_STREAM),
        }
    }

    /// Whether every corner is tried.
    fn all_corners(&self) -> bool {
        self.corners && self.all.is_some()
    }

    /// The corner whose movers are at their high end where `bits` says.
    fn corner(&self, bits: &dyn Fn(usize) -> bool) -> Point {
        let mut i = 0_usize;
        let pos = self
            .mask
            .iter()
            .map(|&moving| {
                if !moving {
                    return Pos::Default;
                }
                let high = bits(i);
                i += 1;
                Pos::At(if high { 1.0 } else { 0.0 })
            })
            .collect();
        Point {
            kind: Kind::Corner,
            pos,
        }
    }

    fn next_corner(&mut self) -> Option<Point> {
        if !self.corners || self.corners_made >= self.max_corners {
            return None;
        }
        if let Some(all) = self.all {
            let mask = self.corners_made as u64;
            if mask >= all {
                return None;
            }
            self.corners_made += 1;
            return Some(self.corner(&|i| mask >> i & 1 == 1));
        }
        let n = self.movers;
        let bits = match self.corners_made {
            0 => vec![false; n],
            1 => vec![true; n],
            _ => loop {
                if self.corner_tries >= self.max_corners.saturating_mul(CORNER_TRIES) {
                    return None;
                }
                self.corner_tries += 1;
                let bits: Vec<bool> = (0..n).map(|_| self.corner_rng.next() >> 63 == 1).collect();
                if !self.corner_seen.contains(&bits) {
                    break bits;
                }
            },
        };
        self.corner_seen.insert(bits.clone());
        self.corners_made += 1;
        Some(self.corner(&|i| bits.get(i).copied().unwrap_or(false)))
    }

    fn next_choice(&mut self) -> Option<Point> {
        if !self.choices {
            return None;
        }
        while self.choices_made < MAX_CHOICE_POINTS {
            let (i, k) = self.choice_at;
            let range = self.list.get(i)?;
            if range.kind != ParamKind::Choice || k > range.choices {
                self.choice_at = (i + 1, 1);
                continue;
            }
            self.choice_at = (i, k + 1);
            if range.default.map(|d| d.round() as i64) == Some(k as i64) {
                continue;
            }
            self.choices_made += 1;
            let pos = (0..self.list.len())
                .map(|j| if j == i { Pos::Choice(k) } else { Pos::Default })
                .collect();
            return Some(Point {
                kind: Kind::Choice,
                pos,
            });
        }
        None
    }

    fn next_random(&mut self) -> Option<Point> {
        if self.random_made >= self.samples {
            return None;
        }
        self.random_made += 1;
        let pos = self
            .mask
            .iter()
            .map(|&moving| {
                // One draw per parameter, moving or not, so a parameter's
                // draws do not depend on the others'.
                let u = self.random_rng.unit();
                if moving { Pos::At(u) } else { Pos::Default }
            })
            .collect();
        Some(Point {
            kind: Kind::Random,
            pos,
        })
    }
}

impl Iterator for Points<'_> {
    type Item = Point;

    fn next(&mut self) -> Option<Point> {
        loop {
            if self.done {
                return None;
            }
            let made = match self.stage {
                Kind::Default => {
                    self.stage = Kind::Corner;
                    return Some(Point {
                        kind: Kind::Default,
                        pos: vec![Pos::Default; self.list.len()],
                    });
                }
                Kind::Corner => self.next_corner(),
                Kind::Choice => self.next_choice(),
                Kind::Random => self.next_random(),
            };
            if made.is_some() {
                return made;
            }
            match self.stage {
                Kind::Default | Kind::Corner => self.stage = Kind::Choice,
                Kind::Choice => self.stage = Kind::Random,
                Kind::Random => self.done = true,
            }
        }
    }
}

/// A clock where there is one: a browser has none to ask.
#[cfg(not(target_arch = "wasm32"))]
struct Clock(std::time::Instant);

#[cfg(not(target_arch = "wasm32"))]
impl Clock {
    fn start() -> Option<Self> {
        // native clock: `Clock` is cfg(not(target_arch = "wasm32")).
        Some(Clock(std::time::Instant::now()))
    }

    fn ms(&self) -> f64 {
        self.0.elapsed().as_secs_f64() * 1000.0
    }
}

#[cfg(target_arch = "wasm32")]
struct Clock;

#[cfg(target_arch = "wasm32")]
impl Clock {
    fn start() -> Option<Self> {
        None
    }

    fn ms(&self) -> f64 {
        0.0
    }
}

/// What one point gave.
struct Checked {
    problems: Vec<Value>,
    /// The assertions neither passed nor failed there (`notProven`), each
    /// `{assertion, message}`: not a problem of the design, and not passed
    /// either (docs/design/verification.md › *Proven, not found*).
    unproven: Vec<Value>,
    ms: Option<f64>,
}

impl Checked {
    fn refused(problems: Vec<Value>) -> Self {
        Checked {
            problems,
            unproven: Vec::new(),
            ms: None,
        }
    }
}

/// The kinds of the bodies of the features that have one of their own.
fn body_kinds(regen: &craft3d_regen::Regen) -> Vec<(FeatureId, BodyKind)> {
    regen
        .features
        .iter()
        .filter(|f| f.is_built())
        .filter_map(|f| f.own_body().map(|b| (f.id, b.kind)))
        .collect()
}

/// Sets `values` on a copy of `base` as `param.set` would, regenerates it
/// without any cache of the open document's and checks it; the kinds of
/// its bodies go to `kinds`. The bodies are measured (mass properties from
/// the exact faces) with `measure` or `deep` only, which the caller gives
/// at the defaults and the corners: on a part with many curved faces
/// measuring costs several times the regeneration (the welding table Jig,
/// 2026-10-23: 90 % of a check-range run with every point measured).
fn check_point(
    base: &Document,
    values: &Map<String, Value>,
    deep: bool,
    measure: bool,
    body_budget: Option<usize>,
    solids: Option<&[(FeatureId, BodyKind)]>,
    kinds: &mut Vec<(FeatureId, BodyKind)>,
) -> Result<Checked, CommandError> {
    let mut problems = Vec::new();
    let sources = match prepare_set("document.sample", base, values, false) {
        Ok((sources, _)) => sources,
        Err(error) => {
            problems.push(json!({
                "problem": format!("param.set refuses this point: {}", error.message()),
            }));
            return Ok(Checked::refused(problems));
        }
    };
    let mut doc = base.clone();
    for (name, source) in &sources {
        if let Err(error) = doc.set_variable(name, source) {
            problems.push(json!({ "parameter": name, "problem": error.to_string() }));
            return Ok(Checked::refused(problems));
        }
    }
    // A session of its own, with a regeneration cache of its own (and the
    // host's budget of bodies), so nothing of the open document's cache
    // is reused and nothing of it changes. `Session::execute` is not used:
    // recovery does not see this session. The kernel's trees of freeform
    // surfaces are shared within the process; the caller lets go of those
    // no open document holds after each point.
    let mut scratch = Session::new(Workspace::none());
    if let Some(bytes) = body_budget {
        scratch.set_body_budget(bytes);
    }
    scratch.add_document(OpenDocument::new(doc, None))?;
    let clock = Clock::start();
    let regen = scratch.document_mut()?.regen_needing(if deep {
        craft3d_regen::Need::All
    } else {
        craft3d_regen::Need::Bodies
    });
    let ms = clock.as_ref().map(Clock::ms);
    *kinds = body_kinds(&regen);
    if let Some(solids) = solids {
        for (id, kind) in solids.iter().filter(|(_, k)| *k == BodyKind::Solid) {
            if let Some((_, now)) = kinds.iter().find(|(f, _)| f == id)
                && *now != *kind
            {
                let name = scratch
                    .document()?
                    .history
                    .current()
                    .feature(*id)
                    .map_or_else(|| id.to_string(), |f| f.name.clone());
                problems.push(json!({
                    "feature": id.to_string(), "name": name,
                    "problem": format!("its body is a solid at the defaults and a {} here", match now {
                        BodyKind::Solid => "solid",
                        BodyKind::Sheet => "sheet",
                    }),
                }));
            }
        }
    }
    let result = crate::commands::check_document(&mut scratch, deep, None, measure)?;
    if let Some(found) = result.get("problems").and_then(Value::as_array) {
        problems.extend(found.iter().cloned());
    }
    let unproven = result
        .get("assertionSummary")
        .and_then(|a| a.get("unproven"))
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .map(|a| json!({"assertion": a.get("name"), "message": a.get("message")}))
                .collect()
        })
        .unwrap_or_default();
    Ok(Checked {
        problems,
        unproven,
        ms,
    })
}

fn median(sorted: &[f64]) -> Option<f64> {
    let n = sorted.len();
    if n == 0 {
        return None;
    }
    let mid = n / 2;
    if n % 2 == 1 {
        sorted.get(mid).copied()
    } else {
        Some((sorted.get(mid - 1)? + sorted.get(mid)?) / 2.0)
    }
}

/// What receives each point of a run as it is checked ([`with_point_sink`]).
type PointSink = Box<dyn FnMut(&Value) -> bool>;

thread_local! {
    static POINT_SINK: std::cell::RefCell<Option<PointSink>> =
        const { std::cell::RefCell::new(None) };
}

/// Runs `work` with `sink` receiving every point a `document.sample` run on
/// this thread checks, as soon as it is checked: `{index, kind, ok, ms,
/// values, problems, unproven, clamped}`, so that a run cut short (a
/// timeout, a kill) has said what it found: `craft3d-cli check-range`
/// prints a line per point. The sink answers whether the run goes on:
/// false stops it before the next point, as its time limit does
/// (`stoppedBy: "caller"`; check-range stops when the process that started
/// it is gone). Nothing else it does changes the run. Puts back whatever
/// sink was there before, also when `work` panics. The sink is the calling
/// thread's: the points are checked one after the other on that thread
/// (each spreading its own work over others); were they ever checked on
/// worker threads, the sink would hear nothing from those.
pub fn with_point_sink<R>(
    sink: impl FnMut(&Value) -> bool + 'static,
    work: impl FnOnce() -> R,
) -> R {
    struct Restore(Option<PointSink>);
    impl Drop for Restore {
        fn drop(&mut self) {
            let earlier = self.0.take();
            let _ = POINT_SINK.try_with(|slot| {
                if let Ok(mut slot) = slot.try_borrow_mut() {
                    *slot = earlier;
                }
            });
        }
    }
    let earlier = POINT_SINK
        .try_with(|slot| {
            slot.try_borrow_mut()
                .ok()
                .and_then(|mut slot| slot.replace(Box::new(sink)))
        })
        .ok()
        .flatten();
    let _restore = Restore(earlier);
    work()
}

/// Gives `point` to the sink of this thread, if there is one, and says
/// whether the run goes on (without a sink it does).
fn tell(point: &Value) -> bool {
    POINT_SINK
        .try_with(|slot| match slot.try_borrow_mut() {
            Ok(mut slot) => slot.as_mut().is_none_or(|sink| sink(point)),
            Err(_) => true,
        })
        .unwrap_or(true)
}

/// Lets go of what the kernel keeps per process between operations: the
/// intersections of surfaces, the projections onto them, the meshes of
/// faces, the bodies that passed their check, and what is remembered of
/// bodies that are gone. Each is worked out again, the same, when asked
/// for: only the time differs. A point's surfaces are mostly its own, so
/// what one point kept is of little use to the next, and kept it filled
/// the caches to their bounds and the memory with them (a gear pair's
/// check over 13 points: 1.2 GB, 2026-10-24).
fn forget_kept() {
    craft3d_geom::intersect::forget_kept();
    craft3d_geom::forget_projections();
    craft3d_tess::forget_kept_meshes();
    craft3d_topo::forget_passed();
    crate::mass::forget_gone();
}

/// How many corners the box of `movers` parameters has (2^n), while that
/// fits in a u64.
fn corners_in_box(movers: usize) -> Option<u64> {
    u32::try_from(movers)
        .ok()
        .filter(|n| *n < 64)
        .map(|n| 1_u64 << n)
}

fn document_sample(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "document.sample";
    let p = Params::new(
        ID,
        params,
        &[
            "samples",
            "seed",
            "corners",
            "maxCorners",
            "choices",
            "deep",
            "strict",
            "budgetMs",
            "timeLimitMs",
            "maxFailures",
            "values",
        ],
    )?;
    let samples = p.integer("samples", 0, MAX_SAMPLES)?.unwrap_or(32) as usize;
    let seed = p.integer("seed", 0, u64::MAX)?.unwrap_or(1);
    let with_corners = p.bool("corners")?.unwrap_or(true);
    let max_corners = p.integer("maxCorners", 0, MAX_CORNERS)?.unwrap_or(64) as usize;
    let with_choices = p.bool("choices")?.unwrap_or(true);
    let deep = p.bool("deep")?.unwrap_or(false);
    let strict = p.bool("strict")?.unwrap_or(false);
    let positive = |key: &str| -> Result<Option<f64>, CommandError> {
        match p.number(key)? {
            None => Ok(None),
            Some(ms) if ms.is_finite() && ms > 0.0 => Ok(Some(ms)),
            Some(ms) => fail(format!("{ID}: `{key}` must be positive, got {ms}")),
        }
    };
    let budget = positive("budgetMs")?;
    if budget.is_some() && Clock::start().is_none() {
        return fail(format!(
            "{ID}: `budgetMs` needs a clock, and there is none here (a browser); check the times with craft3d-cli check-range"
        ));
    }
    // Not given: the default; null: no limit.
    let time_limit = match p.get("timeLimitMs") {
        None if p.has("timeLimitMs") => None,
        None => Some(DEFAULT_TIME_LIMIT_MS),
        Some(_) => positive("timeLimitMs")?,
    };
    let max_failures = p.integer("maxFailures", 0, MAX_FAILURES)?.unwrap_or(100) as usize;
    let with_values = p.bool("values")?.unwrap_or(false);

    let base: Document = (**session.document()?.history.current()).clone();
    let body_budget = session.body_budget;
    let list = ranges(&base);
    if list.is_empty() {
        return fail(format!(
            "{ID}: the document has no public parameters to check over; publish a variable with variable.publish first"
        ));
    }
    let unbounded: Vec<&str> = list
        .iter()
        .filter(|r| !moves(r) && matches!(r.kind, ParamKind::Number | ParamKind::Integer))
        .map(|r| r.name.as_str())
        .collect();
    let mut points = Points::new(
        &list,
        seed,
        with_corners,
        max_corners,
        with_choices,
        samples,
    );
    let all_corners = points.all_corners();
    let in_box = corners_in_box(points.movers);

    // Each point made and checked in turn; the defaults first, whose
    // bodies' kinds the others are held to.
    let clock = Clock::start();
    let mut solids: Option<Vec<(FeatureId, BodyKind)>> = None;
    let mut failures = Vec::new();
    let mut failed = 0_usize;
    let mut unproven_points = Vec::new();
    let mut unproven = 0_usize;
    let mut listed = Vec::new();
    let mut times: Vec<(f64, usize)> = Vec::new();
    let mut by_kind = [0_usize; 4];
    let mut stopped: Option<&str> = None;
    let mut index = 0_usize;
    let mut skipped = 0_usize;
    // Whether the caller (the sink) asked to stop.
    let mut halted = false;
    // What the kernel keeps is let go of after every point where the
    // caller asked for that (`Session::set_lean_sampling`: check-range,
    // whose run may take hours), else once at the end of the run (a
    // window's, at most a minute by default, whose open document keeps its
    // own answers warm meanwhile).
    let per_point = session.lean_sampling;
    for point in &mut points {
        if halted {
            stopped = Some("caller");
            break;
        }
        if MAX_POINTS.is_some_and(|cap| index >= cap) {
            stopped = Some("pointCap");
            break;
        }
        if index > 0
            && let (Some(limit), Some(clock)) = (time_limit, clock.as_ref())
            && clock.ms() > limit
        {
            stopped = Some("timeLimit");
            break;
        }
        let Resolved {
            values,
            unset,
            skip,
            clamped,
        } = resolve(&base, &point);
        if skip {
            skipped += 1;
            continue;
        }
        if let Some(n) = by_kind.get_mut(point.kind.index()) {
            *n += 1;
        }
        let checked = match unset {
            None => {
                let mut kinds = Vec::new();
                let checked = check_point(
                    &base,
                    &values,
                    deep,
                    // Measured at the defaults and the corners, where
                    // the extremes are; the random points only with deep.
                    matches!(point.kind, Kind::Default | Kind::Corner),
                    body_budget,
                    solids.as_deref(),
                    &mut kinds,
                );
                // What the point's bodies alone held in the kernel goes,
                // and what the kernel kept while it worked on them.
                session.release_trees_now();
                if per_point {
                    forget_kept();
                }
                let checked = match checked {
                    Ok(checked) => checked,
                    Err(error) => {
                        forget_kept();
                        crate::verify::forget_kept_lines();
                        return Err(error);
                    }
                };
                if point.kind == Kind::Default {
                    solids = Some(kinds);
                }
                checked
            }
            Some(error) => Checked::refused(vec![
                json!({ "problem": format!("the point cannot be set: {error}") }),
            ]),
        };
        let mut problems = checked.problems;
        if let (Some(ms), Some(budget)) = (checked.ms, budget)
            && ms > budget
        {
            problems.push(json!({
                "problem": format!("regeneration took {ms:.1} ms, over the budget of {budget} ms"),
            }));
        }
        let mut not_proven = checked.unproven;
        if strict {
            // An assertion not proven fails the point.
            for a in not_proven.drain(..) {
                problems.push(json!({
                    "assertion": a.get("assertion"),
                    "problem": format!(
                        "not proven (strict): {}",
                        a.get("message").and_then(Value::as_str).unwrap_or("")
                    ),
                }));
            }
        }
        if let Some(ms) = checked.ms {
            times.push((ms, index));
        }
        let ok = problems.is_empty();
        let mut entry = json!({
            "index": index,
            "kind": point.kind.name(),
            "ok": ok,
            "ms": checked.ms,
        });
        if let Some(map) = entry.as_object_mut() {
            if !not_proven.is_empty() {
                map.insert("unproven".into(), not_proven.len().into());
            }
            if with_values {
                map.insert("values".into(), Value::Object(values.clone()));
            }
            if !clamped.is_empty() {
                map.insert("clamped".into(), clamped.clone().into());
            }
        }
        listed.push(entry);
        halted = !tell(&json!({
            "index": index,
            "kind": point.kind.name(),
            "ok": ok,
            "ms": checked.ms,
            "values": values,
            "problems": problems,
            "unproven": not_proven,
            "clamped": clamped,
        }));
        if !ok {
            failed += 1;
            if failures.len() < max_failures {
                failures.push(json!({
                    "index": index,
                    "kind": point.kind.name(),
                    "values": values,
                    "clamped": clamped,
                    "problems": problems,
                    "ms": checked.ms,
                }));
            }
        } else if !not_proven.is_empty() {
            unproven += 1;
            if unproven_points.len() < max_failures {
                unproven_points.push(json!({
                    "index": index,
                    "kind": point.kind.name(),
                    "values": values,
                    "assertions": not_proven,
                    "ms": checked.ms,
                }));
            }
        }
        index += 1;
    }
    // What the run's points left in the kernel goes (after each point
    // already where the caller asked for that), and the lines of
    // thicknesses, which are kept across the points (capped, some 66 MB at
    // most: `verify::KEPT_LINES`), so that points over a body the sampled
    // values do not reach cast none again, also with lean sampling.
    forget_kept();
    crate::verify::forget_kept_lines();

    let timing = if times.is_empty() {
        Value::Null
    } else {
        let mut sorted: Vec<f64> = times.iter().map(|(ms, _)| *ms).collect();
        sorted.sort_by(f64::total_cmp);
        let slowest = times
            .iter()
            .copied()
            .max_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, index)| index);
        json!({
            "medianMs": median(&sorted),
            "maxMs": sorted.last(),
            "totalMs": sorted.iter().sum::<f64>(),
            "slowest": slowest,
        })
    };
    let complete = stopped.is_none();
    // A run that stopped early proves nothing about the points it did not
    // reach: never ok.
    let ok = if failed > 0 {
        Value::Bool(false)
    } else if complete {
        Value::Bool(true)
    } else {
        Value::Null
    };
    let [defaults, corner_count, choice_count, random] = by_kind;
    Ok(json!({
        "ok": ok,
        "complete": complete,
        "stoppedBy": stopped,
        "seed": seed,
        "deep": deep,
        "strict": strict,
        "budgetMs": budget,
        "timeLimitMs": time_limit,
        "parameters": list.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
        "unbounded": unbounded,
        "counts": {
            "points": index,
            "default": defaults,
            "corner": corner_count,
            "cornersInBox": in_box,
            "allCorners": all_corners,
            "choice": choice_count,
            "random": random,
            "skipped": skipped,
        },
        "passed": index.saturating_sub(failed).saturating_sub(unproven),
        "unproven": unproven,
        "failed": failed,
        "failures": failures,
        "unprovenPoints": unproven_points,
        "unprovenTruncated": unproven > unproven_points.len(),
        "truncated": failed > failures.len(),
        "points": listed,
        "timing": timing,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Measuring is the costly part of a point's check: without `deep`
    /// the bodies of the defaults and the corners are measured, with it
    /// every point's.
    #[test]
    fn random_points_are_measured_only_when_deep() {
        let mut s = Session::new(Workspace::none());
        for (id, params) in [
            ("file.new", json!({"units": "mm"})),
            ("variable.set", json!({"name": "w", "expr": "40 mm"})),
            (
                "feature.block",
                json!({"name": "Plate", "size": ["w", 20, 5]}),
            ),
            (
                "variable.publish",
                json!({"name": "w", "min": "10 mm", "max": "80 mm"}),
            ),
        ] {
            s.execute(id, &params).unwrap();
        }
        let measured = |s: &mut Session, deep: bool| {
            let before = crate::mass::fresh_measures();
            let result = s
                .execute(
                    "document.sample",
                    &json!({"samples": 3, "seed": 1, "deep": deep}),
                )
                .unwrap();
            assert_eq!(result["ok"], true, "{result}");
            // The defaults, two corners and three random points.
            assert_eq!(result["counts"]["points"], 6, "{result}");
            crate::mass::fresh_measures() - before
        };
        // The defaults and the two corners.
        assert_eq!(measured(&mut s, false), 3);
        assert_eq!(measured(&mut s, true), 6);
    }

    #[test]
    fn the_generator_is_seeded_and_stays_in_the_unit_interval() {
        let a: Vec<f64> = {
            let mut r = Rng(7);
            (0..100).map(|_| r.unit()).collect()
        };
        let b: Vec<f64> = {
            let mut r = Rng(7);
            (0..100).map(|_| r.unit()).collect()
        };
        assert_eq!(a, b);
        assert!(a.iter().all(|u| (0.0..1.0).contains(u)));
        let mut c = Rng(8);
        assert_ne!(a.first().copied(), Some(c.unit()));
    }

    #[test]
    fn a_step_never_takes_a_value_past_the_end_of_its_range() {
        // -0.3 + 3 * 0.1 is 5.6e-17 in floating point.
        let ends: Vec<f64> = [0.0, 0.3, 0.6, 0.99, 1.0]
            .iter()
            .filter_map(|u| pick(-0.3, 0.0, (-0.3, 0.1), *u))
            .collect();
        assert_eq!(ends.first(), Some(&-0.3));
        assert_eq!(ends.last(), Some(&0.0));
        assert!(ends.iter().all(|v| (-0.3..=0.0).contains(v)), "{ends:?}");
        assert_eq!(pick(1.0, 0.0, (0.0, 0.1), 0.5), None);
        assert_eq!(pick(0.0, 1.0, (0.0, 0.0), 0.5), None);
        // Counted from 0 under a minimum that is a formula: 26.3 mm to
        // 30 mm in steps of 1 mm gives 27, 28, 29 and 30.
        let picks: Vec<f64> = [0.0, 0.3, 0.6, 0.99]
            .iter()
            .filter_map(|u| pick(26.3, 30.0, (0.0, 1.0), *u))
            .collect();
        assert_eq!(picks, [27.0, 28.0, 29.0, 30.0]);
        // No grid value inside: only the ends.
        assert_eq!(pick(26.3, 26.7, (0.0, 1.0), 0.2), Some(26.3));
        assert_eq!(pick(26.3, 26.7, (0.0, 1.0), 0.8), Some(26.7));
    }
}
