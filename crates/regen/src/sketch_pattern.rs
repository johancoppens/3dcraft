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

use crate::sketch::ConstraintState;
use crate::{angle, length, number};
use craft3d_doc::{
    CenterRef, CurveIndex, CurveRef, Document, EntityId, Expr, Geometry, InstanceIndex,
    MAX_PATTERN_INSTANCES, PatternId, Sketch, SketchPattern, SketchPatternLayout,
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
}

impl PatternResult {
    /// All instances, the seed's included.
    pub fn instances(&self) -> usize {
        let [a, b] = self.counts;
        (a as usize).saturating_mul(b as usize)
    }

    /// The instance an index stands for; `None` when it is past the count.
    pub fn resolve(&self, index: InstanceIndex) -> Option<(u32, u32)> {
        let [a, b] = self.counts;
        match index {
            InstanceIndex::At(k) => (!self.grid && k < a).then_some((k, 0)),
            InstanceIndex::Grid([i, j]) => (self.grid && i < a && j < b).then_some((i, j)),
            InstanceIndex::Last => Some((a.saturating_sub(1), b.saturating_sub(1))),
        }
    }

    /// How many instances there are, for messages: "5 instances (0 to 4)".
    pub fn describe(&self) -> String {
        let [a, b] = self.counts;
        if self.grid {
            format!(
                "{} has {a} by {b} instances ([0,0] to [{},{}])",
                self.id,
                a.saturating_sub(1),
                b.saturating_sub(1)
            )
        } else {
            format!(
                "{} has {a} instance{} (0 to {})",
                self.id,
                if a == 1 { "" } else { "s" },
                a.saturating_sub(1)
            )
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

    /// Where the image of seed point `of` lies in instance `(i, j)`, with
    /// the solved points as `point` gives them. The joints of a chained run
    /// are the same numbers in the two instances they join.
    pub fn image(
        &self,
        point: &impl Fn(EntityId) -> Option<[f64; 2]>,
        of: EntityId,
        (i, j): (u32, u32),
    ) -> Option<[f64; 2]> {
        if (i, j) == (0, 0) {
            return point(of);
        }
        let chain = match self.placed {
            Placed::Chain { from, to } => Some((from, to)),
            Placed::Turn { chain, .. } => chain,
            Placed::Shift { .. } => None,
        };
        if let Some((from, to)) = chain {
            if of == from {
                return self.joint(point, i);
            }
            if of == to {
                return self.joint(point, i.checked_add(1)?);
            }
        }
        self.apply(point, point(of)?, (i, j))
    }

    /// Joint `k` of a chained run: where instance `k` starts.
    fn joint(&self, point: &impl Fn(EntityId) -> Option<[f64; 2]>, k: u32) -> Option<[f64; 2]> {
        let (from, to) = match self.placed {
            Placed::Chain { from, to } => (from, to),
            Placed::Turn {
                chain: Some(chain), ..
            } => chain,
            _ => return None,
        };
        match k {
            0 => point(from),
            1 => point(to),
            k if k == self.counts[0] && self.closes => point(from),
            k => self.apply(point, point(from)?, (k, 0)),
        }
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
    let mut curves = 0usize;
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
        let seeds = curve_seeds(sketch, pattern)
            .filter(|e| !e.construction)
            .count();
        curves = curves.saturating_add(seeds.saturating_mul(instances.saturating_sub(1)));
        if curves > MAX_PATTERN_CURVES {
            return Err(format!(
                "the patterns of the sketch make more than {MAX_PATTERN_CURVES} curves; make fewer instances"
            ));
        }
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
        });
    }
    Ok(out)
}

/// The instance index of a curve made by a pattern, as a name gives it.
fn curve_index(pattern: &PatternResult, (i, j): (u32, u32)) -> CurveIndex {
    if pattern.grid {
        CurveIndex::Grid(i, j)
    } else {
        CurveIndex::At(i)
    }
}

/// The instances of every pattern of a solved sketch as curves for the
/// kernel, each with its name's number (`e7[3]`, [`CurveRef`]); with
/// `construction` the construction curves too. The seed itself is not
/// among them: it is drawn as it is.
pub(crate) fn instance_curves(
    sketch: &Sketch,
    solved: &crate::SketchResult,
    construction: bool,
) -> Result<Vec<(SketchCurve, bool)>, String> {
    let point = |id: EntityId| solved.point(id).map(|(x, y)| [x, y]);
    let mut out = Vec::new();
    for result in &solved.patterns {
        let Some(pattern) = sketch.pattern(result.id) else {
            continue;
        };
        let [a, b] = result.counts;
        for i in 0..a {
            for j in 0..b {
                if (i, j) == (0, 0) {
                    continue;
                }
                let at = |id: EntityId| {
                    result.image(&point, id, (i, j)).ok_or_else(|| {
                        format!(
                            "{}: {id} in instance {} has no solved position",
                            result.id,
                            curve_index(result, (i, j)).describe()
                        )
                    })
                };
                for entity in curve_seeds(sketch, pattern) {
                    if entity.construction && !construction {
                        continue;
                    }
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
                        Geometry::Point { .. } | Geometry::Image { .. } => continue,
                    };
                    let id = CurveRef {
                        entity: entity.id.0,
                        index: curve_index(result, (i, j)),
                    }
                    .encode()
                    .ok_or_else(|| {
                        format!(
                            "{}: {} cannot be named in an instance (its id or the index is too large)",
                            result.id, entity.id
                        )
                    })?;
                    out.push((SketchCurve { id, curve }, entity.construction));
                }
            }
        }
    }
    Ok(out)
}

trait Describe {
    fn describe(&self) -> String;
}

impl Describe for CurveIndex {
    fn describe(&self) -> String {
        match self {
            CurveIndex::Seed => "0".to_string(),
            CurveIndex::At(k) => k.to_string(),
            CurveIndex::Grid(i, j) => format!("{i},{j}"),
            CurveIndex::All => "*".to_string(),
        }
    }
}

/// A curve id of the kernel as a sketch calls it: `e7`, `e7[3]`.
pub(crate) fn curve_name(id: u64) -> String {
    CurveRef::decode(id).map_or_else(|| format!("e{id}"), |c| c.to_string())
}

/// For a sketch whose patterns made curves: per seed curve, how many
/// instances it has in all (the seed's included). A cap whose loop holds
/// every one of them is named after the seed once, as `e7[*]`.
pub(crate) fn full_runs(sketch: &Sketch, solved: &crate::SketchResult) -> BTreeMap<u64, usize> {
    let mut out = BTreeMap::new();
    for result in &solved.patterns {
        // A chained run of one instance is a whole run too: the loop it is
        // part of keeps its cap name when the count goes down to 1.
        // Separate instances (a row of discs) are each their own profile:
        // at one instance the seed's cap is named after the seed, as it is
        // at every count.
        let chained = matches!(
            result.placed,
            Placed::Chain { .. } | Placed::Turn { chain: Some(_), .. }
        );
        if result.instances() < 2 && !chained {
            continue;
        }
        if let Some(pattern) = sketch.pattern(result.id) {
            for entity in curve_seeds(sketch, pattern) {
                out.insert(entity.id.0, result.instances());
            }
        }
    }
    out
}

/// The sources of a cap with every full run of a seed curve written once
/// as `e7[*]` (see [`full_runs`]); `None` when nothing changes.
pub(crate) fn collapse(sources: &[u64], runs: &BTreeMap<u64, usize>) -> Option<Vec<u64>> {
    if runs.is_empty() {
        return None;
    }
    let mut per: BTreeMap<u64, usize> = BTreeMap::new();
    for source in sources {
        if let Some(curve) = CurveRef::decode(*source)
            && runs.contains_key(&curve.entity)
        {
            *per.entry(curve.entity).or_default() += 1;
        }
    }
    let full: Vec<u64> = per
        .into_iter()
        .filter(|(entity, n)| runs.get(entity) == Some(n))
        .map(|(entity, _)| entity)
        .collect();
    if full.is_empty() {
        return None;
    }
    let mut out: Vec<u64> = sources
        .iter()
        .copied()
        .filter(|source| CurveRef::decode(*source).is_none_or(|c| !full.contains(&c.entity)))
        .collect();
    for entity in full {
        let all = CurveRef {
            entity,
            index: CurveIndex::All,
        };
        out.extend(all.encode());
    }
    out.sort_unstable();
    Some(out)
}
