//! Patterns inside a sketch: evaluating their counts and steps, the maps
//! that make image points for the solver, and the instances made from the
//! solved seed.
//!
//! An instance is a rigid image of the solved seed: a shift by `k * step`
//! (and `j * step2` in a grid), a shift by `k * (to - from)` along a
//! chained run, or a turn by `k * angle` about a centre. Instances have no
//! unknowns and nothing about them is stored; they are made again at every
//! regeneration (docs/design/document.md, *Patterns inside a sketch*).
//!
//! The joints of a chained run are worked out once, `J_0 = from`,
//! `J_1 = to`, `J_k = from + k (to - from)` (turned for a ring), and
//! instance `k` ends where instance `k + 1` starts on the very same
//! numbers, so neighbours share their end points bit for bit and the
//! profile closes at any count.
//!
//! A pattern may hold whole patterns in its seed (nested patterns, a tree
//! at most three levels deep). An instance of a curve then has an index on
//! every level, the innermost first, and its points are placed level by
//! level: each point of an instance has a key (`Key`), which is the point
//! itself, a joint of a chained level, or the image of the key one level
//! in under one instance of a level. A level's joints are worked out once
//! from the places of its chain ends, which are keys of the levels inside
//! it, and a point whose key one level in is a chain end of the level
//! around it takes that level's joint: so every cell starts on exactly the
//! numbers on which the cell before it ended, at every level. Places are
//! worked out once per key, so two instances that share a point share its
//! numbers.

use crate::sketch::ConstraintState;
use crate::{angle, length, number};
use craft3d_doc::{
    CenterRef, CurveIndex, CurvePath, Document, EntityId, Expr, Geometry, InstanceIndex,
    MAX_PATTERN_DEPTH, MAX_PATTERN_INSTANCES, PatternId, Sketch, SketchPattern,
    SketchPatternLayout,
};
use craft3d_expr::Evaluation;
use craft3d_kernel::profile::{Curve2, SketchCurve, Spline2};
use craft3d_solve::ImageMap;
use std::collections::BTreeMap;

/// Most curves the patterns of one sketch may make, instances only: a
/// bound on what a hostile count allocates. Profile detection takes more
/// in one call (`profile::MAX_CURVES`, 262 144), so the drawn curves and
/// spline pieces fit beside them.
pub const MAX_PATTERN_CURVES: usize = 100_000;

/// How the instances of a pattern lie, with its expressions evaluated.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Placed {
    /// Instance `(i, j)` is the seed moved by `i * step + j * step2`.
    Shift { step: [f64; 2], step2: [f64; 2] },
    /// A chained run from `from` to `to`: instance `k` is the seed moved
    /// by `k * (to - from)`, as solved.
    Chain { from: EntityId, to: EntityId },
    /// Instance `k` is the seed turned by `k * angle` about the point
    /// `center`, or about `at`; with `chain` a ring of runs.
    Turn {
        center: Option<EntityId>,
        at: [f64; 2],
        angle: f64,
        chain: Option<(EntityId, EntityId)>,
    },
}

/// One pattern of a sketch, as this regeneration evaluated it.
#[derive(Clone, Debug, PartialEq)]
pub struct PatternResult {
    pub id: PatternId,
    /// Instances along the first and the second direction (1 unless a
    /// grid), the seed's included.
    pub counts: [u32; 2],
    /// A grid: its instances are named with two numbers (`e7[2,0]`), also
    /// while it has one row, so that a name does not change with `count2`.
    pub grid: bool,
    pub placed: Placed,
    /// The run closes on itself: a ring whose instances make a whole turn
    /// and whose last joint lies on its first (then it is the first).
    pub closes: bool,
    /// For a ring of runs: how its own equation, that the end of the seed
    /// is its start turned by the angle, solved.
    pub chain: Option<ConstraintState>,
    /// The pattern that holds this one in its seed, if one does.
    pub parent: Option<PatternId>,
    /// The instances skipped that are below the count, in ascending order.
    pub skipped: Vec<(u32, u32)>,
    /// The skips past the count: kept, and back when the count grows.
    pub inactive: Vec<InstanceIndex>,
}

impl PatternResult {
    /// The instances there are: the seed's included, the skipped ones not.
    pub fn instances(&self) -> usize {
        let [a, b] = self.counts;
        (a as usize)
            .saturating_mul(b as usize)
            .saturating_sub(self.skipped.len())
    }

    /// Instance `(i, j)` is there: below the count and not skipped.
    pub fn exists(&self, (i, j): (u32, u32)) -> bool {
        let [a, b] = self.counts;
        i < a && j < b && self.skipped.binary_search(&(i, j)).is_err()
    }

    /// The instances there are, in order (`i`, then `j`).
    pub fn each(&self) -> impl Iterator<Item = (u32, u32)> + '_ {
        let [a, b] = self.counts;
        (0..a)
            .flat_map(move |i| (0..b).map(move |j| (i, j)))
            .filter(|ij| self.skipped.binary_search(ij).is_err())
    }

    /// The instance an index stands for; `None` when it is past the count
    /// or skipped. `last` is the last instance there is: the highest index
    /// that is not skipped (instance 0 never is).
    pub fn resolve(&self, index: InstanceIndex) -> Option<(u32, u32)> {
        let [a, b] = self.counts;
        let found = match index {
            InstanceIndex::At(k) => (!self.grid && k < a).then_some((k, 0)),
            InstanceIndex::Grid([i, j]) => (self.grid && i < a && j < b).then_some((i, j)),
            InstanceIndex::Last => {
                let last = (a.saturating_sub(1), b.saturating_sub(1));
                if self.skipped.is_empty() {
                    Some(last)
                } else {
                    // At most as many steps back as there are skips.
                    let mut at = last;
                    loop {
                        if self.skipped.binary_search(&at).is_err() {
                            break Some(at);
                        }
                        at = match at {
                            (0, 0) => break Some((0, 0)),
                            (i, 0) => (i - 1, b.saturating_sub(1)),
                            (i, j) => (i, j - 1),
                        };
                    }
                }
            }
        };
        found.filter(|ij| self.skipped.binary_search(ij).is_err())
    }

    /// How many instances there are, for messages: "5 instances (0 to 4)".
    pub fn describe(&self) -> String {
        let [a, b] = self.counts;
        let skipped = match self.skipped.as_slice() {
            [] => String::new(),
            list => {
                let shown: Vec<String> = list
                    .iter()
                    .take(5)
                    .map(|&(i, j)| {
                        if self.grid {
                            format!("[{i},{j}]")
                        } else {
                            i.to_string()
                        }
                    })
                    .collect();
                let more = list.len().saturating_sub(shown.len());
                format!(
                    ", without {}{}",
                    shown.join(", "),
                    if more > 0 {
                        format!(" and {more} more")
                    } else {
                        String::new()
                    }
                )
            }
        };
        if self.grid {
            format!(
                "{} has {a} by {b} instances ([0,0] to [{},{}]{skipped})",
                self.id,
                a.saturating_sub(1),
                b.saturating_sub(1)
            )
        } else {
            format!(
                "{} has {a} instance{} (0 to {}{skipped})",
                self.id,
                if a == 1 { "" } else { "s" },
                a.saturating_sub(1)
            )
        }
    }

    /// The ends of its run, for a chained layout.
    fn chain_ends(&self) -> Option<(EntityId, EntityId)> {
        match self.placed {
            Placed::Chain { from, to } => Some((from, to)),
            Placed::Turn { chain, .. } => chain,
            Placed::Shift { .. } => None,
        }
    }

    /// The map the solver gives the image of a seed point in instance
    /// `(i, j)`, with the sketch's points as `find` resolves them.
    pub(crate) fn map(
        &self,
        (i, j): (u32, u32),
        find: impl Fn(EntityId) -> Result<craft3d_solve::EntityId, String>,
    ) -> Result<ImageMap, String> {
        let (fi, fj) = (f64::from(i), f64::from(j));
        Ok(match self.placed {
            Placed::Shift { step, step2 } => ImageMap::Shift {
                offset: [fi * step[0] + fj * step2[0], fi * step[1] + fj * step2[1]],
            },
            Placed::Chain { from, to } => ImageMap::Chain {
                from: find(from)?,
                to: find(to)?,
                k: fi,
            },
            Placed::Turn {
                center, at, angle, ..
            } => ImageMap::Turn {
                center: match center {
                    Some(id) => Some(find(id)?),
                    None => None,
                },
                at,
                angle: angle * fi,
            },
        })
    }

    /// Where instance `(i, j)` puts the point at `at`, with the points the
    /// map uses as `point` gives them: [`ImageMap::apply`] on the map the
    /// solver is given, so both place an image with the same arithmetic.
    /// The map carries the sketch's own ids (below 2^32, `MAX_ENTITY_ID`).
    fn apply(
        &self,
        point: &impl Fn(EntityId) -> Option<[f64; 2]>,
        at: [f64; 2],
        (i, j): (u32, u32),
    ) -> Option<[f64; 2]> {
        let own = |id: EntityId| {
            u32::try_from(id.0)
                .map(craft3d_solve::EntityId)
                .map_err(|_| String::new())
        };
        let map = self.map((i, j), own).ok()?;
        map.apply(at, |id| point(EntityId(u64::from(id.0))))
    }
}

/// A whole number of at least 1, from a plain number within 1e-9 of it
/// (as a pattern feature's count).
fn whole(variables: &Evaluation, name: &str, expr: &Expr) -> Result<u32, String> {
    let value = number(variables, name, expr)?;
    let whole = value.round();
    if !(value.is_finite() && (value - whole).abs() <= 1e-9 && whole >= 1.0) {
        return Err(format!(
            "{name} must be a whole number of at least 1, but `{}` is {value}",
            expr.source()
        ));
    }
    if whole > MAX_PATTERN_INSTANCES as f64 {
        return Err(format!(
            "{name}: `{}` is {whole}; a pattern has at most {MAX_PATTERN_INSTANCES} instances",
            expr.source()
        ));
    }
    // Whole, at least 1 and at most the cap: the cast is exact.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(whole as u32)
}

fn lengths(
    doc: &Document,
    variables: &Evaluation,
    name: &str,
    exprs: &[Expr; 2],
) -> Result<[f64; 2], String> {
    let [x, y] = exprs;
    let x = length(doc, variables, &format!("{name}.0"), x)?;
    let y = length(doc, variables, &format!("{name}.1"), y)?;
    if !(x.is_finite() && y.is_finite()) {
        return Err(format!("{name} is not finite"));
    }
    Ok([x, y])
}

/// The curve seeds of a pattern that are not construction.
fn curve_seeds<'a>(
    sketch: &'a Sketch,
    pattern: &'a SketchPattern,
) -> impl Iterator<Item = &'a craft3d_doc::Entity> + 'a {
    pattern
        .seed
        .iter()
        .filter_map(|id| sketch.entity(*id).ok())
        .filter(|e| !matches!(e.geometry, Geometry::Point { .. } | Geometry::Image { .. }))
}

/// Evaluates the patterns of a sketch: counts, steps, angles and centres.
pub(crate) fn evaluate(
    doc: &Document,
    variables: &Evaluation,
    sketch: &Sketch,
) -> Result<Vec<PatternResult>, String> {
    let mut out = Vec::with_capacity(sketch.patterns.len());
    for pattern in &sketch.patterns {
        let id = pattern.id;
        let first = whole(variables, &format!("{id}.count"), &pattern.count)?;
        let (second, placed) = match &pattern.layout {
            SketchPatternLayout::Linear { step, second } => {
                let step = lengths(doc, variables, &format!("{id}.step"), step)?;
                let (count, step2) = match second {
                    Some(second) => (
                        whole(variables, &format!("{id}.count2"), &second.count)?,
                        lengths(doc, variables, &format!("{id}.step2"), &second.step)?,
                    ),
                    None => (1, [0.0, 0.0]),
                };
                if first > 1 && step == [0.0, 0.0] {
                    return Err(format!(
                        "{id}.step is zero: every instance would lie on the seed"
                    ));
                }
                if count > 1 && step2 == [0.0, 0.0] {
                    return Err(format!(
                        "{id}.step2 is zero: every row would lie on the first"
                    ));
                }
                (count, Placed::Shift { step, step2 })
            }
            SketchPatternLayout::Chain { from, to } => (
                1,
                Placed::Chain {
                    from: *from,
                    to: *to,
                },
            ),
            SketchPatternLayout::Circular {
                center,
                angle: turn,
                chain,
            } => {
                let turn = angle(doc, variables, &format!("{id}.angle"), turn)?;
                if !turn.is_finite() {
                    return Err(format!("{id}.angle is not finite"));
                }
                if first > 1 && turn == 0.0 {
                    return Err(format!(
                        "{id}.angle is zero: every instance would lie on the seed"
                    ));
                }
                let (center, at) = match center {
                    CenterRef::Point { point } => (Some(*point), [0.0, 0.0]),
                    CenterRef::At { at } => {
                        (None, lengths(doc, variables, &format!("{id}.center"), at)?)
                    }
                };
                (
                    1,
                    Placed::Turn {
                        center,
                        at,
                        angle: turn,
                        chain: chain.map(|[a, b]| (a, b)),
                    },
                )
            }
        };
        let instances = (first as usize).saturating_mul(second as usize);
        if instances > MAX_PATTERN_INSTANCES {
            return Err(format!(
                "{id} has {first} by {second} instances; a pattern has at most {MAX_PATTERN_INSTANCES}"
            ));
        }
        // Skips: those below the count are left out, those past it kept.
        let mut skipped = Vec::new();
        let mut inactive = Vec::new();
        for skip in &pattern.skip {
            let ij = match *skip {
                InstanceIndex::At(k) => (k, 0),
                InstanceIndex::Grid([i, j]) => (i, j),
                InstanceIndex::Last => continue,
            };
            if ij.0 < first && ij.1 < second {
                skipped.push(ij);
            } else {
                inactive.push(*skip);
            }
        }
        skipped.sort_unstable();
        skipped.dedup();
        let angular = doc.tolerance.angular;
        let closes = match placed {
            Placed::Turn { angle, .. } => {
                let total = (angle * f64::from(first)).abs();
                (total - std::f64::consts::TAU).abs() <= angular
            }
            _ => false,
        };
        out.push(PatternResult {
            id,
            counts: [first, second],
            grid: pattern.layout.is_grid(),
            placed,
            closes,
            chain: None,
            parent: sketch.parent_pattern(id),
            skipped,
            inactive,
        });
    }
    // The curves the instances make, every level counted: the product of
    // the counts around a seed curve, worked out before anything is made.
    let mut curves = 0usize;
    let mut construction = 0usize;
    let too_many = || {
        format!(
            "the patterns of the sketch make more than {MAX_PATTERN_CURVES} curves; make fewer instances"
        )
    };
    for (pattern, result) in sketch.patterns.iter().zip(&out) {
        let mut product = 1usize;
        for level in sketch.pattern_levels(result.id) {
            let count = out
                .iter()
                .find(|r| r.id == level)
                .map_or(1, PatternResult::instances);
            product = product.checked_mul(count).ok_or_else(too_many)?;
        }
        let made = product.saturating_sub(1);
        for entity in curve_seeds(sketch, pattern) {
            let total = if entity.construction {
                &mut construction
            } else {
                &mut curves
            };
            *total = total.checked_add(made).ok_or_else(too_many)?;
        }
        if curves > MAX_PATTERN_CURVES || construction > MAX_PATTERN_CURVES {
            return Err(too_many());
        }
    }
    Ok(out)
}

/// The instance index of a curve on one level, as a name gives it.
fn curve_index(pattern: &PatternResult, (i, j): (u32, u32)) -> CurveIndex {
    if pattern.grid {
        CurveIndex::Grid(i, j)
    } else {
        CurveIndex::At(i)
    }
}

/// What a point of an instance is, so that two instances that share it
/// share its numbers: the point itself (instance 0 on every level), the
/// joint `k` of a chained level, or the image of a key one level in under
/// instance `(i, j)` of a level. Levels are given by their place in the
/// sketch's list of patterns.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Key {
    Own(EntityId),
    Joint(usize, u32),
    Inst(usize, usize, (u32, u32)),
}

/// The ends of the run of a chained level: their keys and places.
#[derive(Clone, Copy, Debug)]
struct Ends {
    from: (EntityId, usize, [f64; 2]),
    to: (EntityId, usize, [f64; 2]),
}

/// Places the points of the instances of a sketch's patterns, level by
/// level (see the module's documentation): keys are worked out once and
/// their places with them.
pub(crate) struct Placer<'a, F: Fn(EntityId) -> Option<[f64; 2]>> {
    sketch: &'a Sketch,
    patterns: &'a [PatternResult],
    at: BTreeMap<PatternId, usize>,
    point: F,
    keys: BTreeMap<Key, usize>,
    places: Vec<Option<[f64; 2]>>,
    points: BTreeMap<EntityId, Result<usize, String>>,
    ends: BTreeMap<usize, Result<Option<Ends>, String>>,
}

/// Most steps from a point to the levels around it: one per image point
/// on the way and per level, which a valid sketch keeps within this.
const MAX_STEPS: usize = 4 * MAX_PATTERN_DEPTH + 4;

impl<'a, F: Fn(EntityId) -> Option<[f64; 2]>> Placer<'a, F> {
    /// With the solved points (not image points) as `point` gives them.
    pub(crate) fn new(sketch: &'a Sketch, patterns: &'a [PatternResult], point: F) -> Self {
        Placer {
            sketch,
            patterns,
            at: patterns
                .iter()
                .enumerate()
                .map(|(n, p)| (p.id, n))
                .collect(),
            point,
            keys: BTreeMap::new(),
            places: Vec::new(),
            points: BTreeMap::new(),
            ends: BTreeMap::new(),
        }
    }

    fn intern(&mut self, key: Key, place: impl FnOnce(&Self) -> Option<[f64; 2]>) -> usize {
        if let Some(&id) = self.keys.get(&key) {
            return id;
        }
        let at = place(self);
        let id = self.places.len();
        self.places.push(at);
        self.keys.insert(key, id);
        id
    }

    /// Where the point with key `key` lies.
    pub(crate) fn place(&self, key: usize) -> Option<[f64; 2]> {
        self.places.get(key).copied().flatten()
    }

    /// The key of a point of the sketch: an image point's is that of the
    /// instance it stands for. An error names an image point whose
    /// instance is not there.
    pub(crate) fn point_key(&mut self, id: EntityId) -> Result<usize, String> {
        self.point_key_within(id, 0)
    }

    fn point_key_within(&mut self, id: EntityId, steps: usize) -> Result<usize, String> {
        if let Some(found) = self.points.get(&id) {
            return found.clone();
        }
        if steps > MAX_STEPS {
            return Err(format!(
                "{id} is an image of an image of ... more than {MAX_STEPS} deep"
            ));
        }
        let found = match self.sketch.entity(id).map(|e| e.geometry.clone()) {
            Ok(Geometry::Point { .. }) => {
                let place = (self.point)(id);
                Ok(self.intern(Key::Own(id), |_| place))
            }
            Ok(Geometry::Image { of, pattern, index }) => match self.at.get(&pattern).copied() {
                None => Err(format!(
                    "{id} = {of}[{index}]: the sketch has no pattern {pattern}"
                )),
                Some(level) => {
                    let inner = self.point_key_within(of, steps + 1)?;
                    match self.patterns.get(level).and_then(|p| p.resolve(index)) {
                        Some(instance) => self.level_key(level, inner, instance, steps + 1),
                        None => Err(format!(
                            "{id} = {of}[{index}] is not there: {}",
                            self.patterns
                                .get(level)
                                .map_or_else(String::new, PatternResult::describe)
                        )),
                    }
                }
            },
            Ok(other) => Err(format!("{id} is a {}, not a point", other.type_name())),
            Err(error) => Err(error.to_string()),
        };
        self.points.insert(id, found.clone());
        found
    }

    /// The ends of the run of level `level`, when it is chained.
    fn ends(&mut self, level: usize, steps: usize) -> Result<Option<Ends>, String> {
        if let Some(found) = self.ends.get(&level) {
            return found.clone();
        }
        let found = match self.patterns.get(level).and_then(PatternResult::chain_ends) {
            None => Ok(None),
            Some((from, to)) => {
                let end = |placer: &mut Self, id: EntityId| -> Result<_, String> {
                    let key = placer.point_key_within(id, steps + 1)?;
                    let place = placer
                        .place(key)
                        .ok_or_else(|| format!("the chain end {id} has no solved position"))?;
                    Ok((id, key, place))
                };
                let a = end(self, from);
                let b = end(self, to);
                a.and_then(|from| b.map(|to| Some(Ends { from, to })))
            }
        };
        self.ends.insert(level, found.clone());
        found
    }

    /// Where instance `(i, j)` of level `level` puts the point at `at`.
    fn apply(
        &self,
        level: usize,
        at: [f64; 2],
        instance: (u32, u32),
        ends: Option<Ends>,
    ) -> Option<[f64; 2]> {
        let pattern = self.patterns.get(level)?;
        let point = |id: EntityId| match ends {
            Some(e) if id == e.from.0 => Some(e.from.2),
            Some(e) if id == e.to.0 => Some(e.to.2),
            _ => (self.point)(id),
        };
        pattern.apply(&point, at, instance)
    }

    /// Joint `k` of chained level `level`: where its instance `k` starts.
    fn joint(&mut self, level: usize, k: u32, ends: Ends) -> usize {
        let (count, closes) = self
            .patterns
            .get(level)
            .map_or((0, false), |p| (p.counts[0], p.closes));
        match k {
            0 => ends.from.1,
            1 => ends.to.1,
            k if k == count && closes => ends.from.1,
            k => self.intern(Key::Joint(level, k), |placer| {
                placer.apply(level, ends.from.2, (k, 0), Some(ends))
            }),
        }
    }

    /// The key of the point with key `inner` one level in, in instance
    /// `instance` of level `level`.
    fn level_key(
        &mut self,
        level: usize,
        inner: usize,
        instance: (u32, u32),
        steps: usize,
    ) -> Result<usize, String> {
        if instance == (0, 0) {
            return Ok(inner);
        }
        let ends = self.ends(level, steps)?;
        if let Some(ends) = ends {
            let (i, _) = instance;
            if inner == ends.from.1 {
                return Ok(self.joint(level, i, ends));
            }
            if inner == ends.to.1 {
                return Ok(self.joint(level, i.saturating_add(1), ends));
            }
        }
        Ok(self.intern(Key::Inst(level, inner, instance), |placer| {
            let at = placer.place(inner)?;
            placer.apply(level, at, instance, ends)
        }))
    }

    /// Where a point of an instance lies: point `id` of a seed (or an image
    /// point in it) with the instance of each level around it, the
    /// innermost first.
    pub(crate) fn at(
        &mut self,
        id: EntityId,
        levels: &[(usize, (u32, u32))],
    ) -> Result<Option<[f64; 2]>, String> {
        let mut key = self.point_key(id)?;
        for &(level, instance) in levels {
            key = self.level_key(level, key, instance, 0)?;
        }
        Ok(self.place(key))
    }

    /// For a ring of runs that makes a whole turn: whether its last joint,
    /// as the turns put it, lies within `linear` of its first.
    pub(crate) fn ring_closes(&mut self, level: usize, linear: f64) -> bool {
        let Some(pattern) = self.patterns.get(level) else {
            return false;
        };
        let count = pattern.counts[0];
        match self.ends(level, 0) {
            Ok(Some(ends)) => {
                let [ax, ay] = ends.from.2;
                match self.apply(level, ends.from.2, (count, 0), Some(ends)) {
                    Some([bx, by]) => (ax - bx).hypot(ay - by) <= linear,
                    None => false,
                }
            }
            _ => false,
        }
    }
}

/// The curve seeds of a pattern (lines, arcs, circles, splines).
fn curve_entities<'a>(
    sketch: &'a Sketch,
    pattern: &'a SketchPattern,
) -> impl Iterator<Item = &'a craft3d_doc::Entity> + 'a {
    curve_seeds(sketch, pattern)
}

/// What [`each_instance`] calls for an instance: the seed curve and its
/// index on each level (the level's place in the list of patterns), the
/// innermost first.
type Found<'a> = dyn FnMut(&craft3d_doc::Entity, &[(usize, (u32, u32))]) -> Result<(), String> + 'a;

/// Calls `found` for every instance of a seed curve the patterns make (the
/// seeds themselves are drawn), with its index on each level, the
/// innermost first: the patterns that are in no seed in the sketch's
/// order, and within one its instances in order, the outermost level
/// first, each with its own curves and then the patterns in its seed.
fn each_instance(
    sketch: &Sketch,
    patterns: &[PatternResult],
    found: &mut Found<'_>,
) -> Result<(), String> {
    fn walk(
        sketch: &Sketch,
        patterns: &[PatternResult],
        level: usize,
        outer: &mut Vec<(usize, (u32, u32))>,
        found: &mut Found<'_>,
    ) -> Result<(), String> {
        let Some(result) = patterns.get(level) else {
            return Ok(());
        };
        let Some(pattern) = sketch.pattern(result.id) else {
            return Ok(());
        };
        if outer.len() >= MAX_PATTERN_DEPTH {
            return Err(format!(
                "{}: patterns nest at most {MAX_PATTERN_DEPTH} levels deep",
                result.id
            ));
        }
        let inner: Vec<usize> = pattern
            .patterns
            .iter()
            .filter_map(|q| patterns.iter().position(|p| p.id == *q))
            .collect();
        for instance in result.each() {
            outer.push((level, instance));
            // The levels innermost first.
            let levels: Vec<(usize, (u32, u32))> = outer.iter().rev().copied().collect();
            if levels.iter().any(|(_, ij)| *ij != (0, 0)) {
                for entity in curve_entities(sketch, pattern) {
                    found(entity, &levels)?;
                }
            }
            for &child in &inner {
                walk(sketch, patterns, child, outer, found)?;
            }
            outer.pop();
        }
        Ok(())
    }
    for (level, result) in patterns.iter().enumerate() {
        if result.parent.is_none() {
            walk(sketch, patterns, level, &mut Vec::new(), found)?;
        }
    }
    Ok(())
}

/// The numbers a name keeps for instance `levels` of seed curve `entity`
/// (`e3[2][1]`): one word, as before nesting, unless an outer level is
/// not at instance 0.
fn words_of(
    patterns: &[PatternResult],
    entity: EntityId,
    levels: &[(usize, (u32, u32))],
) -> Result<Vec<u64>, String> {
    let indices: Vec<CurveIndex> = levels
        .iter()
        .map(|&(level, instance)| {
            patterns
                .get(level)
                .map_or(CurveIndex::At(instance.0), |p| curve_index(p, instance))
        })
        .collect();
    CurvePath::of_levels(entity.0, &indices)
        .encode()
        .ok_or_else(|| {
            format!("{entity} cannot be named in an instance (its id or an index is too large)")
        })
}

/// The kernel's numbers for the curves of nested patterns that are in an
/// outer instance other than 0, whose names take more than one number
/// (`e3[2][1]`): a sketch-local number with bit 63 set, dense, in the
/// order of the names. Every other curve's number is the one its name
/// keeps ([`CurveRef`]); no name word has bit 63 set with bits 32 to 47
/// zero, so the two never meet.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CurveIds {
    nested: Vec<Vec<u64>>,
}

const NESTED: u64 = 1 << 63;

impl CurveIds {
    /// The numbers of the curves of the patterns of `sketch` as evaluated.
    pub(crate) fn of(sketch: &Sketch, patterns: &[PatternResult]) -> Result<Self, String> {
        if patterns.iter().all(|p| p.parent.is_none()) {
            return Ok(Self::default());
        }
        let mut nested = Vec::new();
        each_instance(sketch, patterns, &mut |entity, levels| {
            let words = words_of(patterns, entity.id, levels)?;
            if words.len() > 1 {
                nested.push(words);
            }
            Ok(())
        })?;
        nested.sort_unstable();
        nested.dedup();
        Ok(Self { nested })
    }

    /// The kernel's number for a curve with these name words.
    pub fn id(&self, words: &[u64]) -> Option<u64> {
        match words {
            [one] => Some(*one),
            _ => self
                .nested
                .binary_search_by(|w| w.as_slice().cmp(words))
                .ok()
                .and_then(|at| u64::try_from(at).ok())
                .map(|at| NESTED | at),
        }
    }

    /// The name words of a kernel number: the number itself unless it is
    /// one of the nested curves'.
    pub fn words(&self, id: u64) -> Vec<u64> {
        if id & NESTED != 0
            && (id >> 32) & 0xFFFF == 0
            && let Some(words) = usize::try_from(id & !NESTED)
                .ok()
                .and_then(|at| self.nested.get(at))
        {
            return words.clone();
        }
        vec![id]
    }

    /// A kernel number as the sketch calls the curve: `e7`, `e7[3]`,
    /// `e3[2][1]`.
    pub fn name(&self, id: u64) -> String {
        let words = self.words(id);
        match CurvePath::read(&words) {
            Some((path, n)) if n == words.len() => path.to_string(),
            _ => format!("e{id}"),
        }
    }

    /// The sources of a name made of kernel numbers, in name words.
    pub(crate) fn expand(&self, sources: &[u64]) -> Vec<u64> {
        if self.nested.is_empty() {
            return sources.to_vec();
        }
        sources.iter().flat_map(|&s| self.words(s)).collect()
    }

    /// True when no curve has a number of its own.
    pub fn is_empty(&self) -> bool {
        self.nested.is_empty()
    }
}

/// The instances of every pattern of a solved sketch as curves for the
/// kernel, each with its number ([`CurveIds`]); with `construction` the
/// construction curves too. The seed itself is not among them: it is drawn
/// as it is.
pub(crate) fn instance_curves(
    sketch: &Sketch,
    solved: &crate::SketchResult,
    construction: bool,
) -> Result<Vec<(SketchCurve, bool)>, String> {
    let point = |id: EntityId| {
        // Points with coordinates only: the placer places the images.
        match sketch.entity(id).map(|e| &e.geometry) {
            Ok(Geometry::Point { .. }) => solved.point(id).map(|(x, y)| [x, y]),
            _ => None,
        }
    };
    let patterns = &solved.patterns;
    let mut placer = Placer::new(sketch, patterns, point);
    let ids = &solved.curve_ids;
    let mut out = Vec::new();
    each_instance(sketch, patterns, &mut |entity, levels| {
        if entity.construction && !construction {
            return Ok(());
        }
        let name = || {
            words_of(patterns, entity.id, levels).map_or_else(
                |_| entity.id.to_string(),
                |w| ids.name(ids.id(&w).unwrap_or_default()),
            )
        };
        let mut at = |id: EntityId| -> Result<[f64; 2], String> {
            placer
                .at(id, levels)?
                .ok_or_else(|| format!("{id} in {} has no solved position", name()))
        };
        let curve = match &entity.geometry {
            &Geometry::Line { start, end } => Curve2::Line {
                start: at(start)?,
                end: at(end)?,
            },
            &Geometry::Arc { center, start, end } => Curve2::Arc {
                center: at(center)?,
                start: at(start)?,
                end: at(end)?,
            },
            &Geometry::Circle { center, .. } => Curve2::Circle {
                center: at(center)?,
                radius: solved
                    .radius(entity.id)
                    .ok_or_else(|| format!("{} has no solved radius", entity.id))?,
            },
            Geometry::Spline { degree, control } => {
                let knots = craft3d_doc::clamped_uniform_knots(*degree, control.len());
                let degree = usize::try_from(*degree).unwrap_or(usize::MAX);
                let control = control
                    .iter()
                    .map(|id| at(*id))
                    .collect::<Result<Vec<_>, _>>()?;
                Curve2::Spline(Spline2 {
                    degree,
                    knots,
                    control,
                })
            }
            Geometry::Point { .. } | Geometry::Image { .. } => return Ok(()),
        };
        let words = words_of(patterns, entity.id, levels)?;
        let id = ids.id(&words).ok_or_else(|| {
            format!(
                "{} has no number of its own; this is a bug",
                CurvePath::read(&words)
                    .map_or_else(|| entity.id.to_string(), |(p, _)| p.to_string())
            )
        })?;
        out.push((SketchCurve { id, curve }, entity.construction));
        Ok(())
    })?;
    Ok(out)
}

/// Where each image point of a sketch lies (those whose instance is
/// there), with the solved points as `point` gives them: the same numbers
/// as the curves of that instance.
pub(crate) fn image_places(
    sketch: &Sketch,
    patterns: &[PatternResult],
    point: impl Fn(EntityId) -> Option<[f64; 2]>,
) -> Vec<(EntityId, f64, f64)> {
    let mut placer = Placer::new(sketch, patterns, point);
    let mut out = Vec::new();
    for entity in &sketch.entities {
        if !matches!(entity.geometry, Geometry::Image { .. }) {
            continue;
        }
        if let Ok(Some([x, y])) = placer.at(entity.id, &[]) {
            out.push((entity.id, x, y));
        }
    }
    out
}

/// A curve as the sketch calls it, from the kernel's number: `e7`,
/// `e7[3]`, `e3[2][1]`.
pub(crate) fn curve_name(id: u64, ids: &CurveIds) -> String {
    ids.name(id)
}

/// One level of a seed curve's patterns, for naming caps: how many
/// instances it has, whether a loop that holds all of them is a run (a
/// chained level, or one of two instances or more), and whether it is a
/// grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Level {
    count: usize,
    run: bool,
    grid: bool,
}

/// For a sketch whose patterns made curves: per seed curve, its levels,
/// the innermost first. A cap whose loop holds every instance of a curve
/// on a level (with `*` on every level inside it) is named after it once
/// there: `e7[*]`, `e3[*][1]`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Runs {
    curves: BTreeMap<u64, Vec<Level>>,
}

impl Runs {
    fn is_empty(&self) -> bool {
        self.curves.is_empty()
    }
}

pub(crate) fn full_runs(sketch: &Sketch, solved: &crate::SketchResult) -> Runs {
    let mut curves = BTreeMap::new();
    let level = |id: PatternId| -> Option<Level> {
        let result = solved.patterns.iter().find(|r| r.id == id)?;
        // A chained run of one instance is a whole run too: the loop it
        // is part of keeps its cap name when the count goes down to 1.
        // Separate instances (a row of discs) are each their own profile:
        // at one instance the seed's cap is named after the seed, as it is
        // at every count.
        let chained = result.chain_ends().is_some();
        Some(Level {
            count: result.instances(),
            run: chained || result.instances() >= 2,
            grid: result.grid,
        })
    };
    for result in &solved.patterns {
        let Some(pattern) = sketch.pattern(result.id) else {
            continue;
        };
        let levels: Vec<Level> = sketch
            .pattern_levels(result.id)
            .into_iter()
            .filter_map(level)
            .collect();
        if levels.iter().all(|l| !l.run) {
            continue;
        }
        for entity in curve_seeds(sketch, pattern) {
            curves.insert(entity.id.0, levels.clone());
        }
    }
    Runs { curves }
}

/// The sources of a cap (name words) with every full run written once,
/// level by level, the innermost first (see [`Runs`]), and the curves in
/// the order names keep; `None` when the sketch has no runs.
pub(crate) fn collapse(sources: &[u64], runs: &Runs) -> Option<Vec<u64>> {
    if runs.is_empty() {
        return None;
    }
    let groups = craft3d_doc::curve_groups(sources).ok()?;
    // Each curve of a run with its index on every level (None for `*`).
    type Indices = Vec<Option<(u32, u32)>>;
    let mut items: Vec<(u64, Indices)> = Vec::new();
    let mut kept: Vec<Vec<u64>> = Vec::new();
    for group in groups {
        let read = CurvePath::read(group).filter(|(_, n)| *n == group.len());
        let Some((path, levels)) =
            read.and_then(|(p, _)| runs.curves.get(&p.curve.entity).map(|l| (p, l)))
        else {
            kept.push(group.to_vec());
            continue;
        };
        let indices = (0..levels.len())
            .map(|level| match path.level(level) {
                CurveIndex::All => None,
                CurveIndex::Seed => Some((0, 0)),
                CurveIndex::At(k) => Some((k, 0)),
                CurveIndex::Grid(i, j) => Some((i, j)),
            })
            .collect();
        items.push((path.curve.entity, indices));
    }
    let depth = runs.curves.values().map(Vec::len).max().unwrap_or(0);
    for level in 0..depth {
        // The curves whole on every level inside this one, by the curve
        // and their indices around it.
        let mut by: BTreeMap<(u64, Indices), std::collections::BTreeSet<(u32, u32)>> =
            BTreeMap::new();
        for (entity, indices) in &items {
            if indices.len() <= level || indices.iter().take(level).any(Option::is_some) {
                continue;
            }
            if let Some(Some(here)) = indices.get(level) {
                let around = indices.get(level + 1..).unwrap_or_default().to_vec();
                by.entry((*entity, around)).or_default().insert(*here);
            }
        }
        for ((entity, around), here) in by {
            let Some(info) = runs.curves.get(&entity).and_then(|l| l.get(level)) else {
                continue;
            };
            if !(info.run && here.len() == info.count) {
                continue;
            }
            items.retain(|(e, indices)| {
                !(*e == entity
                    && indices.get(level + 1..).unwrap_or_default() == around.as_slice()
                    && indices.iter().take(level).all(Option::is_none)
                    && indices.get(level).is_some_and(Option::is_some))
            });
            let mut whole: Indices = vec![None; level + 1];
            whole.extend(around);
            items.push((entity, whole));
        }
    }
    for (entity, indices) in items {
        let levels = runs.curves.get(&entity);
        let path: Vec<CurveIndex> = indices
            .iter()
            .enumerate()
            .map(|(level, index)| match index {
                None => CurveIndex::All,
                Some((i, j)) => {
                    if levels.and_then(|l| l.get(level)).is_some_and(|l| l.grid) {
                        CurveIndex::Grid(*i, *j)
                    } else {
                        CurveIndex::At(*i)
                    }
                }
            })
            .collect();
        kept.push(CurvePath::of_levels(entity, &path).encode()?);
    }
    kept.sort_unstable();
    kept.dedup();
    Some(kept.concat())
}
