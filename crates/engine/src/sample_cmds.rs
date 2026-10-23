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
use crate::param_cmds::{Range, prepare_set, ranges, value_text};
use crate::params::Params;
use crate::{CommandError, CommandSpec, OpenDocument, Session, Workspace};
use craft3d_doc::{Document, FeatureId, ParamKind};
use craft3d_topo::BodyKind;
use serde_json::{Map, Value, json};
use std::collections::BTreeSet;

pub(crate) static SPECS: &[CommandSpec] = &[spec!(
    "document.sample",
    "Check Over the Range",
    r#"{"samples": integer = 32, "seed": integer = 1, "corners": boolean = true, "maxCorners": integer = 64, "choices": boolean = true, "deep": boolean = false, "budgetMs": number = none, "timeLimitMs": number or null = 60000, "maxFailures": integer = 100, "values": boolean = false}"#,
    "Proves a Jig over its parameter range: on a copy of the active document (the document and its undo history do not change), sets the public parameters (param.list) to a series of points and checks each one: the defaults; the corners of the box of the ranges (every parameter at its min or max, a boolean off or on, a choice its first or last; all 2^n of them while that is at most maxCorners (at most 4096), else maxCorners of them chosen by the seed, the two all-low and all-high corners always among them); with choices every other choice of each choice parameter, the rest at the defaults; then samples (at most 10000) random points chosen by the seed (a number uniform over its range and on its step, an integer, a choice or a boolean uniform over its values; the same points whatever corners are asked for). A range that uses other parameters is evaluated at each point after them; ranges that keep moving each other fail the point (did not settle). A side of a range that is not given is the default. Each point is set as param.set sets it (a point it refuses, such as a range that is empty there, fails with the reason), regenerated without the open document's cache (timed: ms; the kernel's trees of freeform surfaces are shared within the process, so a surface the open document has met may be quicker), and checked as document.check (deep with deep) checks: no failed variables or features, valid bodies, no invalid parameter, no broken assertion (the bodies measured, mass properties from the exact faces, at the defaults and the corners, and with deep at every point: measuring costs several times a regeneration); and every feature whose body is a solid at the defaults has a solid there too. With budgetMs a point whose regeneration takes longer fails as well; without it times are reported and never part of the verdict. timeLimitMs bounds the whole run (checked between points; null: no limit, as craft3d-cli check-range runs it); in a browser, which has no clock, a run stops after 128 points instead. A run that stops early has complete false, stoppedBy (timeLimit or pointCap) and ok false when a point failed, else null: never true. The same seed gives the same points and the same verdict. Returns ok, complete, the number of points by kind, failed (how many failed), failures (the first maxFailures: index, kind default|corner|choice|random, values: every parameter as param.set takes it, to reproduce the point, problems, ms), points ({index, kind, ok, ms} each; with values also its values), the parameters, unbounded (numbers without min and max: only their default is tried) and timing (medianMs, maxMs, totalMs, slowest point; null where there is no clock, as in a browser).",
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

/// The value `u` picks of those from `lo` in steps of `step` within `hi`,
/// held to `[lo, hi]` (rounding must not step past an end: a range that
/// ends at 0 ends at 0, not at 5.6e-17).
fn pick(lo: f64, hi: f64, step: f64, u: f64) -> Option<f64> {
    if !(hi >= lo && step > 0.0) {
        return None;
    }
    let span = ((hi - lo) / step * (1.0 + 1e-12)).floor();
    if !span.is_finite() || span >= 9.0e15 {
        return None;
    }
    let k = (u * (span + 1.0)).floor().min(span);
    Some((lo + k * step).clamp(lo, hi))
}

/// The value of `range` at fraction `u`, in its unit; none when its range
/// is empty or does not evaluate (the point then keeps the default, and
/// `param.set` says what is wrong).
fn value_at(range: &Range, u: f64) -> Option<f64> {
    let u = u.clamp(0.0, 1.0);
    match range.kind {
        ParamKind::Boolean => Some(if u < 0.5 { 0.0 } else { 1.0 }),
        ParamKind::Choice => {
            let n = range.choices as f64;
            (n >= 1.0).then(|| ((u * n).floor() + 1.0).min(n))
        }
        ParamKind::Integer => {
            let lo = range.min.or(range.default)?.ceil();
            let hi = range.max.or(range.default)?.floor();
            let step = range.step.map_or(1.0, |s| s.round().max(1.0));
            pick(lo, hi, step, u)
        }
        ParamKind::Number => {
            let lo = range.min.or(range.default)?;
            let hi = range.max.or(range.default)?;
            if hi < lo {
                return None;
            }
            match range.step {
                Some(step) => pick(lo, hi, step, u),
                None if u >= 1.0 => Some(hi),
                None => Some((lo + u * (hi - lo)).clamp(lo, hi)),
            }
        }
    }
}

/// The values of `point`, each as `param.set` takes it, in the order of
/// the parameters: worked out against the ranges at the point itself,
/// again until no value moves (a range that uses another parameter after
/// that one), at most once per parameter and once more. With the values
/// as far as they got comes why they cannot be set, if they cannot: a
/// value the document refuses, or ranges that do not settle (two ranges
/// that push each other on).
fn resolve(base: &Document, point: &Point) -> (Map<String, Value>, Option<String>) {
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
            return (values_of(&doc), Some(why));
        }
    }
    let rounds = first.len().saturating_add(1);
    for _ in 0..rounds {
        let now = ranges(&doc);
        let mut moved = false;
        for (range, pos) in now.iter().zip(&point.pos) {
            let text = match pos {
                Pos::Default => None,
                Pos::At(u) => value_at(range, *u).map(|v| value_text(range, v)),
                Pos::Choice(k) => Some(value_text(range, *k as f64)),
            };
            let Some(text) = text else { continue };
            let same = doc
                .variable(&range.name)
                .is_some_and(|v| v.expr.source() == text);
            if !same {
                if let Err(e) = doc.set_variable(&range.name, &text) {
                    let why = format!("`{}` = `{text}`: {e}", range.name);
                    return (values_of(&doc), Some(why));
                }
                moved = true;
            }
        }
        if !moved {
            return (values_of(&doc), None);
        }
    }
    let why = format!(
        "the ranges did not settle after {rounds} rounds (ranges that use each other keep moving the values); the values are the last ones"
    );
    (values_of(&doc), Some(why))
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
    ms: Option<f64>,
}

/// The kinds of the bodies of the features that have one of their own.
fn body_kinds(regen: &craft3d_regen::Regen) -> Vec<(FeatureId, BodyKind)> {
    regen
        .features
        .iter()
        .filter(|f| matches!(f.status, craft3d_regen::Status::Ok))
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
            return Ok(Checked { problems, ms: None });
        }
    };
    let mut doc = base.clone();
    for (name, source) in &sources {
        if let Err(error) = doc.set_variable(name, source) {
            problems.push(json!({ "parameter": name, "problem": error.to_string() }));
            return Ok(Checked { problems, ms: None });
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
    Ok(Checked { problems, ms })
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

    // Each point made and checked in turn; the defaults first, whose
    // bodies' kinds the others are held to.
    let clock = Clock::start();
    let mut solids: Option<Vec<(FeatureId, BodyKind)>> = None;
    let mut failures = Vec::new();
    let mut failed = 0_usize;
    let mut listed = Vec::new();
    let mut times: Vec<(f64, usize)> = Vec::new();
    let mut by_kind = [0_usize; 4];
    let mut stopped: Option<&str> = None;
    let mut index = 0_usize;
    for point in &mut points {
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
        if let Some(n) = by_kind.get_mut(point.kind.index()) {
            *n += 1;
        }
        let (values, unset) = resolve(&base, &point);
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
                )?;
                if point.kind == Kind::Default {
                    solids = Some(kinds);
                }
                // What the point's bodies alone held in the kernel goes.
                session.release_trees_now();
                checked
            }
            Some(error) => Checked {
                problems: vec![json!({ "problem": format!("the point cannot be set: {error}") })],
                ms: None,
            },
        };
        let mut problems = checked.problems;
        if let (Some(ms), Some(budget)) = (checked.ms, budget)
            && ms > budget
        {
            problems.push(json!({
                "problem": format!("regeneration took {ms:.1} ms, over the budget of {budget} ms"),
            }));
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
        if let (true, Some(map)) = (with_values, entry.as_object_mut()) {
            map.insert("values".into(), Value::Object(values.clone()));
        }
        listed.push(entry);
        if !ok {
            failed += 1;
            if failures.len() < max_failures {
                failures.push(json!({
                    "index": index,
                    "kind": point.kind.name(),
                    "values": values,
                    "problems": problems,
                    "ms": checked.ms,
                }));
            }
        }
        index += 1;
    }

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
        "budgetMs": budget,
        "timeLimitMs": time_limit,
        "parameters": list.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
        "unbounded": unbounded,
        "counts": {
            "points": index,
            "default": defaults,
            "corner": corner_count,
            "allCorners": all_corners,
            "choice": choice_count,
            "random": random,
        },
        "passed": index - failed,
        "failed": failed,
        "failures": failures,
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
            .filter_map(|u| pick(-0.3, 0.0, 0.1, *u))
            .collect();
        assert_eq!(ends.first(), Some(&-0.3));
        assert_eq!(ends.last(), Some(&0.0));
        assert!(ends.iter().all(|v| (-0.3..=0.0).contains(v)), "{ends:?}");
        assert_eq!(pick(1.0, 0.0, 0.1, 0.5), None);
        assert_eq!(pick(0.0, 1.0, 0.0, 0.5), None);
    }
}
