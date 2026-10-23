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
//! generator of our own (SplitMix64), so a seed gives the same points and
//! the same verdict everywhere; the times are reported, and only a
//! `budgetMs` makes them part of the verdict.

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
    r#"{"samples": integer = 32, "seed": integer = 1, "corners": boolean = true, "maxCorners": integer = 64, "choices": boolean = true, "deep": boolean = false, "budgetMs": number = none, "maxFailures": integer = 100, "values": boolean = false}"#,
    "Proves a Jig over its parameter range: on a copy of the active document (the document and its undo history do not change), sets the public parameters (param.list) to a series of points and checks each one: the defaults; the corners of the box of the ranges (every parameter at its min or max, a boolean off or on, a choice its first or last; all 2^n of them while that is at most maxCorners (at most 4096), else maxCorners of them chosen by the seed, the two all-low and all-high corners always among them); with choices every other choice of each choice parameter, the rest at the defaults; then samples (at most 10000) random points chosen by the seed (a number uniform over its range and on its step, an integer, a choice or a boolean uniform over its values). A range that uses other parameters is evaluated at each point after them. A side of a range that is not given is the default. Each point is set as param.set sets it (a point it refuses, such as a range that is empty there, fails with the reason), regenerated from nothing (timed: ms), and checked as document.check (deep with deep) checks: no failed variables or features, valid bodies, no invalid parameter, no broken assertion; and every feature whose body is a solid at the defaults has a solid there too. With budgetMs a point whose regeneration takes longer fails as well; without it times are reported and never part of the verdict. The same seed gives the same points and the same verdict. Returns ok, the number of points by kind, failed (how many failed), failures (the first maxFailures: index, kind default|corner|choice|random, values: every parameter as param.set takes it, to reproduce the point, problems, ms), points ({index, kind, ok, ms} each; with values also its values), the parameters, unbounded (numbers without min and max: only their default is tried) and timing (medianMs, maxMs, totalMs, slowest point; null where there is no clock, as in a browser).",
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

/// The value of `range` at fraction `u`, in its unit; none when its range
/// is empty or does not evaluate (the point then keeps the default, and
/// `param.set` says what is wrong).
fn value_at(range: &Range, u: f64) -> Option<f64> {
    let u = u.clamp(0.0, 1.0);
    // How many values from `lo` in steps of `step` lie within `hi`, and
    // which of them `u` picks.
    let pick = |lo: f64, hi: f64, step: f64| -> Option<f64> {
        if !(hi >= lo && step > 0.0) {
            return None;
        }
        let span = ((hi - lo) / step * (1.0 + 1e-12)).floor();
        if !span.is_finite() || span >= 9.0e15 {
            return None;
        }
        let k = (u * (span + 1.0)).floor().min(span);
        Some(lo + k * step)
    };
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
            pick(lo, hi, step)
        }
        ParamKind::Number => {
            let lo = range.min.or(range.default)?;
            let hi = range.max.or(range.default)?;
            if hi < lo {
                return None;
            }
            match range.step {
                Some(step) => pick(lo, hi, step),
                None if u >= 1.0 => Some(hi),
                None => Some(lo + u * (hi - lo)),
            }
        }
    }
}

/// The values of `point`, each as `param.set` takes it, in the order of
/// the parameters: worked out against the ranges at the point itself,
/// again until no value moves (a range that uses another parameter after
/// that one), at most once per parameter and once more.
fn resolve(base: &Document, point: &Point) -> Result<Map<String, Value>, String> {
    let mut doc = base.clone();
    let first = ranges(&doc);
    // Every parameter at its default first.
    for range in &first {
        doc.set_variable(&range.name, &range.default_source)
            .map_err(|e| format!("`{}`: its default: {e}", range.name))?;
    }
    for _ in 0..=first.len() {
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
                doc.set_variable(&range.name, &text)
                    .map_err(|e| format!("`{}` = `{text}`: {e}", range.name))?;
                moved = true;
            }
        }
        if !moved {
            break;
        }
    }
    let mut values = Map::new();
    for range in &first {
        let text = doc
            .variable(&range.name)
            .map(|v| v.expr.source().to_string())
            .unwrap_or_else(|| range.default_source.clone());
        values.insert(range.name.clone(), Value::String(text));
    }
    Ok(values)
}

/// The corners: every parameter that moves at its low or its high end;
/// all of them when there are at most `max`, else `max` chosen by `rng`,
/// all-low and all-high first.
fn corners(movers: &[usize], count: usize, max: usize, rng: &mut Rng) -> Vec<Point> {
    let n = movers.len();
    if n == 0 || max == 0 {
        return Vec::new();
    }
    let corner = |bits: &dyn Fn(usize) -> bool| {
        let mut pos = vec![Pos::Default; count];
        for (i, &at) in movers.iter().enumerate() {
            if let Some(slot) = pos.get_mut(at) {
                *slot = Pos::At(if bits(i) { 1.0 } else { 0.0 });
            }
        }
        Point {
            kind: Kind::Corner,
            pos,
        }
    };
    let all = (n < 63)
        .then(|| 1_u64 << n)
        .filter(|&all| all <= max as u64);
    if let Some(all) = all {
        return (0..all)
            .map(|mask| corner(&|i| mask >> i & 1 == 1))
            .collect();
    }
    let mut seen: BTreeSet<Vec<bool>> = BTreeSet::new();
    let mut out = Vec::new();
    let mut tries = 0_usize;
    let mut add = |bits: Vec<bool>, out: &mut Vec<Point>| {
        if seen.insert(bits.clone()) {
            out.push(corner(&|i| bits.get(i).copied().unwrap_or(false)));
        }
    };
    add(vec![false; n], &mut out);
    if max > 1 {
        add(vec![true; n], &mut out);
    }
    while out.len() < max && tries < max.saturating_mul(CORNER_TRIES) {
        tries += 1;
        let bits: Vec<bool> = (0..n).map(|_| rng.next() >> 63 == 1).collect();
        add(bits, &mut out);
    }
    out
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
/// from nothing and checks it; the kinds of its bodies go to `kinds`.
fn check_point(
    base: &Document,
    values: &Map<String, Value>,
    deep: bool,
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
    // A session of its own, with a cache of its own: the regeneration is
    // from nothing, timed honestly, and nothing of the open document's
    // changes (`Session::execute` is not used, so neither recovery nor
    // the kernel's release of trees sees this session).
    let mut scratch = Session::new(Workspace::none());
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
    let check = Session::command("document.check")
        .ok_or_else(|| CommandError::new("document.sample: document.check is not registered"))?;
    let result = (check.run)(&mut scratch, &json!({ "deep": deep }))?;
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
    let budget = match p.number("budgetMs")? {
        None => None,
        Some(ms) if ms.is_finite() && ms > 0.0 => Some(ms),
        Some(ms) => return fail(format!("{ID}: `budgetMs` must be positive, got {ms}")),
    };
    if budget.is_some() && Clock::start().is_none() {
        return fail(format!(
            "{ID}: `budgetMs` needs a clock, and there is none here (a browser); check the times with craft3d-cli check-range"
        ));
    }
    let max_failures = p.integer("maxFailures", 0, MAX_FAILURES)?.unwrap_or(100) as usize;
    let with_values = p.bool("values")?.unwrap_or(false);

    let base: Document = (**session.document()?.history.current()).clone();
    let list = ranges(&base);
    if list.is_empty() {
        return fail(format!(
            "{ID}: the document has no public parameters to check over; publish a variable with variable.publish first"
        ));
    }
    let count = list.len();
    let movers: Vec<usize> = (0..count)
        .filter(|&i| list.get(i).is_some_and(moves))
        .collect();
    let unbounded: Vec<&str> = list
        .iter()
        .filter(|r| !moves(r) && matches!(r.kind, ParamKind::Number | ParamKind::Integer))
        .map(|r| r.name.as_str())
        .collect();

    // The points, in a fixed order: defaults, corners, choices, random.
    let mut rng = Rng(seed);
    let mut points = vec![Point {
        kind: Kind::Default,
        pos: vec![Pos::Default; count],
    }];
    let corner_points = if with_corners {
        corners(&movers, count, max_corners, &mut rng)
    } else {
        Vec::new()
    };
    let all_corners = with_corners
        && movers.len() < 63
        && (1_u64 << movers.len()) as usize == corner_points.len();
    points.extend(corner_points);
    if with_choices {
        let mut added = 0_usize;
        for (i, range) in list.iter().enumerate() {
            if range.kind != ParamKind::Choice {
                continue;
            }
            let default = range.default.map(|d| d.round() as i64);
            for k in 1..=range.choices {
                if default == Some(k as i64) || added >= MAX_CHOICE_POINTS {
                    continue;
                }
                let mut pos = vec![Pos::Default; count];
                if let Some(slot) = pos.get_mut(i) {
                    *slot = Pos::Choice(k);
                }
                points.push(Point {
                    kind: Kind::Choice,
                    pos,
                });
                added += 1;
            }
        }
    }
    for _ in 0..samples {
        let pos = (0..count)
            .map(|i| {
                let u = rng.unit();
                if movers.contains(&i) {
                    Pos::At(u)
                } else {
                    Pos::Default
                }
            })
            .collect();
        points.push(Point {
            kind: Kind::Random,
            pos,
        });
    }

    // Each point checked; the defaults first, whose bodies' kinds the
    // others are held to.
    let mut solids: Option<Vec<(FeatureId, BodyKind)>> = None;
    let mut failures = Vec::new();
    let mut failed = 0_usize;
    let mut listed = Vec::with_capacity(points.len());
    let mut times: Vec<(f64, usize)> = Vec::new();
    let mut by_kind = [0_usize; 4];
    for (index, point) in points.iter().enumerate() {
        if let Some(n) = by_kind.get_mut(point.kind as usize) {
            *n += 1;
        }
        let (values, checked) = match resolve(&base, point) {
            Ok(values) => {
                let mut kinds = Vec::new();
                let checked = check_point(&base, &values, deep, solids.as_deref(), &mut kinds)?;
                if point.kind == Kind::Default {
                    solids = Some(kinds);
                }
                (values, checked)
            }
            Err(error) => (
                Map::new(),
                Checked {
                    problems: vec![
                        json!({ "problem": format!("the point cannot be set: {error}") }),
                    ],
                    ms: None,
                },
            ),
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
    let [defaults, corner_count, choice_count, random] = by_kind;
    Ok(json!({
        "ok": failed == 0,
        "seed": seed,
        "deep": deep,
        "budgetMs": budget,
        "parameters": list.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
        "unbounded": unbounded,
        "counts": {
            "points": points.len(),
            "default": defaults,
            "corner": corner_count,
            "allCorners": all_corners,
            "choice": choice_count,
            "random": random,
        },
        "passed": points.len() - failed,
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
    fn all_corners_while_they_fit_else_a_seeded_subset_with_both_ends() {
        let mut rng = Rng(1);
        let all = corners(&[0, 2], 3, 64, &mut rng);
        assert_eq!(all.len(), 4);
        assert!(all.iter().all(|p| p.pos.get(1) == Some(&Pos::Default)));
        let movers: Vec<usize> = (0..10).collect();
        let some = corners(&movers, 10, 16, &mut Rng(3));
        assert_eq!(some.len(), 16);
        assert!(some[0].pos.iter().all(|p| *p == Pos::At(0.0)));
        assert!(some[1].pos.iter().all(|p| *p == Pos::At(1.0)));
        let again = corners(&movers, 10, 16, &mut Rng(3));
        let bits = |ps: &[Point]| -> Vec<Vec<Pos>> { ps.iter().map(|p| p.pos.clone()).collect() };
        assert_eq!(bits(&some), bits(&again));
        assert!(corners(&[], 2, 64, &mut rng).is_empty());
    }
}
