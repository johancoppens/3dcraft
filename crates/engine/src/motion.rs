//! Motion over the timeline `t` (docs/design/motion.md).
//!
//! The **timeline** of a document is its variable `t` when that is a plain
//! number (no unit): formulas of `t` (a crank angle `360 deg * t`) are the
//! drivers of its motion, and `t` runs from 0 to 1. A document whose `t` is
//! a length (a thickness, as several Jigs have it) has no timeline until
//! that variable is renamed: nothing is reserved behind a design's back.
//!
//! - `view.motion` shows the model at a `t` without changing the document
//!   (view state, as `view.scrub` is): a copy of the document with `t` set
//!   is regenerated from a copy of the document's regeneration cache, so
//!   only the features that depend on `t` are rebuilt and only their
//!   bodies are meshed again (the session's store of meshes keeps the
//!   others). Each state is a function of its `t` alone: its sketches are
//!   carried from the document's own solution in fixed steps of `t`
//!   ([`Sweep`]), so a linkage solved in a sketch stays on its branch and
//!   a seek shows what a play shows. With `collisions` the bodies that overlap at that `t`
//!   are drawn red, in the scene itself, so every renderer and every
//!   picture shows them.
//! - `view.trace` samples the path of a named point (a vertex by its
//!   persistent name, or a sketch point) over a range of `t`; the trace
//!   is drawn in the viewport and `file.export {trace}` writes it as SVG
//!   or DXF in its plane.
//! - `motion.check` checks any assertion at samples of a range of `t`:
//!   the first sample it fails at (and the last before it that passed),
//!   and where its measure is least and greatest at the samples (a
//!   clearance's minimum, the first sample of a collision).
//!
//! What is rebuilt, honestly: a part's features that depend on `t` are
//! regenerated at every frame (only those). An assembly's instances are
//! placed by transforms only: each moment of an assembly is a solve
//! carried along the motion from its rest state ([`Sweep::placed_at`]),
//! and a moment the solve cannot reach is a dead point, reported with
//! its `t`.

use crate::asm::{AsmState, Carried, Placed};
use crate::commands::{Run, fail, length_unit, spec, vec3};
use crate::params::Params;
use crate::selection::{Resolver, SelectionKind, feature_named};
use crate::{CommandError, CommandSpec, OpenDocument, Session, Workspace};
use craft3d_doc::{Appearance, Document, EntityId, FeatureId, FeatureKind};
use craft3d_kernel::boolean::{Interference, interference};
use craft3d_math::{Tolerance, Vec3};
use craft3d_regen::{Regen, RegenCache};
use craft3d_topo::Body;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::sync::Arc;

pub(crate) static SPECS: &[CommandSpec] = &[
    spec!(
        "view.motion",
        "Show Motion",
        r#"{"t": number from 0 to 1 or null, "collisions": boolean = as before (false)}"#,
        "Shows the model in the view at a moment t of its motion, without changing the document: not its variables, its undo history or its dirty state (view state, as view.scrub). The timeline is the document's variable t when it is a plain number (no unit): formulas of t drive the motion (a crank angle 360 deg * t). Only the features that depend on t are regenerated, and only their bodies are meshed again; each state is a function of t alone: its sketches are carried from the document's own solution to t in fixed steps of t (1/128), so a linkage solved in a sketch stays on its branch, and a moment reached by a jump is the one a play reaches. collisions: true draws the bodies that overlap at t red (in the scene, so view.render, the window and view.replay show it) and lists the pairs. t null shows the document as it is again. The viewport, view.render, view.pick and screenshots show the state; document.inspect, measures and exports still see the document itself. An edit keeps the motion (the state is made again from the edited document); a document whose t is gone or has a unit lets go of it. Without parameters it changes nothing. Returns t (null when not shown), the timeline (its variable and its value in the document), rebuilt (the features whose bodies differ from the document's at this t) and collisions ([{a, aName, b, bName, volume}], or unknown pairs with a reason) when asked for. An assembly plays t too: drivers are formulas of t for its joint variables (Drive_angle = 360 deg * t), each moment is a solve carried along the branch from the assembly at rest (never a regeneration), and where the mechanism cannot go further the result has deadPoint {t, why}: the t it stopped at, the parts shown where they stopped.",
        doc: true,
        view_motion
    ),
    spec!(
        "view.trace",
        "Trace Point",
        r#"{"point": vertex name or {sketch, entity}, "feature": id or name = the body that has the vertex, "t": [number, number] = [0, 1], "samples": integer = 65, "name": string = "Trace <n>", "grow": boolean = false, "clear": boolean or trace name}"#,
        "Traces the path of a point over the motion: the point is a vertex by its persistent name (document.edges lists them; feature says which body when several have it) or a point of a sketch ({sketch, entity}); its place is taken at samples values of t evenly from t[0] to t[1] (both included), each state made as view.motion makes it. The trace is view state of the document (not saved, not undone, let go of at the next edit): the viewport draws it as a line, and file.export {path: *.svg or *.dxf, trace: name} writes it in its plane. With grow: true the viewport draws the trace only up to the t it shows while it shows a moment of the motion (the player draws it as it goes), whole otherwise. A trace with the name of one kept replaces it; at most 32 are kept. clear: true removes every trace, clear: name one of them (nothing is traced then). The document needs a timeline (view.motion). A point that is gone at some t fails the trace, naming that t. Returns name, point, t, samples, grow, points ([x, y, z] in the document's unit), length (along the polyline), closed (more than 2 points and it ends where it began, within the tolerance), box and plane ({point, normal} when the path lies in one plane within the tolerance and within a millionth of its size, else null). In an assembly the point is named through an instance as a mate names it (Coupler/End2, Foot/F1:vertex(2)), without feature.",
        doc: true,
        names: ["point"],
        view_trace
    ),
    spec!(
        "motion.check",
        "Check Over Motion",
        r#"{"assertions": [name, ...] = every assertion (or only measure's), "measure": {the parameters of assert.add without name}, "t": [number, number] = [0, 1], "samples": integer = 33, "details": boolean = false}"#,
        "Checks assertions at samples of the motion: at samples values of t evenly from t[0] to t[1] (both included) the document is set to that t (as view.motion makes it) and the assertions are measured as document.check measures them, on a copy: the document does not change. What it proves is what held at the samples: a failure between two samples (a collision shorter than their spacing) can be missed, so the result says sampled: true. measure is an assertion written as for assert.add (without its name) and checked only here; assertions names the document's own (default: all of them, or none when measure is given). Any measure works: overlap with is false gives the first sample that collides, clearance with a min the least clearance at the samples and its t. Per assertion: status (failed when any sample fails; else error, notProven, passed, or inactive when it is never on), failed (how many samples), firstFailure ({t, after, measured, message}: the first sample that fails, and after the last sample before it that passed, null when none did, so the failure begins in (after, t]; null when no sample fails), firstError and firstNotProven the same for samples that could not be measured or not proven, min and max ({t, value} of a numeric measure at the samples), unit and expected; with details every sample ({t, status, measured}). ok is true when every assertion passed at every sample. The document needs a timeline (view.motion). At most 1000 samples. For an assembly each sample is measured at the moment as view.motion solves it, and deadPoints lists every dead point met ({t, why}).",
        doc: true,
        motion_check
    ),
];

/// The name of the timeline variable.
pub const TIMELINE: &str = "t";

/// The colour of a body that collides with another at the `t` shown.
pub const COLLISION_COLOR: [u8; 3] = [220, 38, 38];

/// Most samples of a trace or a check.
const MAX_SAMPLES: u64 = 1000;

/// Most traces kept per document.
const MAX_TRACES: usize = 32;

/// Most bodies the collisions of a state are looked for among.
const MAX_COLLISION_BODIES: usize = 500;

/// Most pairs of bodies whose boxes meet that one state sends to the
/// kernel for its collisions.
const MAX_COLLISION_PAIRS: usize = 2_000;

/// Most answers about pairs of bodies kept from one state for the next
/// (one state checks at most [`MAX_COLLISION_PAIRS`]).
const MAX_MEMO: usize = MAX_COLLISION_PAIRS;

/// The value of the timeline in `doc` (its variable `t` as the document
/// has it), or why the document has none: a sentence for whoever asked.
pub(crate) fn timeline(doc: &Document) -> Result<f64, String> {
    let Some(variable) = doc.variable(TIMELINE) else {
        return Err(format!(
            "the document has no timeline: a variable `{TIMELINE}`, a plain number from 0 to 1 that the formulas of the motion use (variable.set {{\"name\": \"{TIMELINE}\", \"expr\": \"0\"}}, then for example a crank angle `360 deg * {TIMELINE}`)"
        ));
    };
    let table = doc.variable_table().map_err(|e| e.to_string())?;
    let evaluated = table.evaluate();
    match evaluated.get(TIMELINE) {
        Some(Ok(q)) if q.dimension.is_dimensionless() && q.value.is_finite() => Ok(q.value),
        Some(Ok(q)) => Err(format!(
            "`{TIMELINE}` here is {} (`{}`), not the timeline: the timeline is a variable `{TIMELINE}` that is a plain number; rename this one (variable.rename) to give the document a timeline",
            q.dimension.described(),
            variable.expr.source()
        )),
        Some(Err(e)) => Err(format!("the timeline `{TIMELINE}` does not evaluate: {e}")),
        None => Err(format!("the timeline `{TIMELINE}` does not evaluate")),
    }
}

/// `t` as the text of a plain number that reads back as the same `f64`.
pub(crate) fn literal(t: f64) -> String {
    format!("{t}")
}

/// A value of `t` from a parameter: a number from 0 to 1.
fn t_value(command: &str, key: &str, value: &Value) -> Result<f64, CommandError> {
    match value.as_f64() {
        Some(t) if (0.0..=1.0).contains(&t) => Ok(t),
        _ => fail(format!(
            "{command}: `{key}` must be a number from 0 to 1, got {}",
            short(value)
        )),
    }
}

/// A value as text for an error, at most 80 characters.
fn short(value: &Value) -> String {
    value.to_string().chars().take(80).collect()
}

/// The range of `t` a trace or a check runs over (`[a, b]`, default
/// `[0, 1]`; `a` may be greater than `b`, to run backwards).
pub(crate) fn t_range(command: &str, p: &Params) -> Result<(f64, f64), CommandError> {
    match p.get("t") {
        None | Some(Value::Null) => Ok((0.0, 1.0)),
        Some(Value::Array(items)) if items.len() == 2 => {
            let a = items
                .first()
                .map_or(Ok(0.0), |v| t_value(command, "t", v))?;
            let b = items.get(1).map_or(Ok(1.0), |v| t_value(command, "t", v))?;
            Ok((a, b))
        }
        Some(other) => fail(format!(
            "{command}: `t` is a range [from, to] of numbers from 0 to 1, got {}",
            short(other)
        )),
    }
}

/// The `samples` values of `t` evenly from `a` to `b`, both included.
pub(crate) fn samples_of((a, b): (f64, f64), samples: u64) -> Vec<f64> {
    let last = samples.saturating_sub(1).max(1) as f64;
    (0..samples)
        .map(|i| {
            let u = i as f64 / last;
            // The ends exactly.
            if i + 1 == samples { b } else { a + (b - a) * u }
        })
        .collect()
}

/// The spacing of the fixed steps of `t` a moment's sketches are carried
/// along in, from the document's own solution (`Sweep`): 1/128, a crank
/// turned by `360 deg * t` moves 2.8 degrees a step.
pub(crate) const MOTION_STEP: f64 = 1.0 / 128.0;

/// Most fixed steps between the document's own `t` and a moment shown.
const MAX_STEPS: i64 = 4_096;

/// States of one document over `t`, each a function of its `t` alone, so
/// a seek and a play show the same state and the order the moments are
/// asked in does not matter (docs/design/motion.md › *Branch continuity*).
///
/// The reference is the document as given (its own `t`, `t0`, and its
/// sketches as last solved). The sketches at the **grid** point
/// `t0 + k·MOTION_STEP` are those at the grid point one step nearer to
/// `t0` settled at it (`k = 0` is the reference's own), each made once and
/// kept; a moment `t` is the grid point between `t0` and `t` nearest to it
/// (`k` = `(t - t0) / MOTION_STEP` rounded toward zero) settled at `t`,
/// then regenerated. So a linkage solved in a sketch is carried from its
/// own solution to any `t` in steps of at most `MOTION_STEP` and stays on
/// its branch, whatever jump the view made.
#[derive(Clone)]
pub(crate) struct Sweep {
    reference: Document,
    t0: f64,
    /// The indices of the reference's sketch features.
    sketches: Vec<usize>,
    /// The sketches at the grid points made so far, by `k`.
    grid: BTreeMap<i64, Arc<Vec<FeatureKind>>>,
    cache: RegenCache,
    /// For an assembly: its moments, solved rather than regenerated.
    assembly: Option<AsmSweep>,
}

/// The motion of an assembly over `t` (docs/design/motion.md ›
/// *Assemblies*): each moment is a solve, its instances placed by
/// transforms, never regenerated. The same grid as a part's sketches: the
/// solved state at grid point `k` is the one at `k ∓ 1` carried to its `t`
/// by the solver's continuation (`Solver::advance`), and a moment is its
/// grid point carried to `t`; so a moment is a function of `t` alone and
/// stays on the branch the assembly rests on.
#[derive(Clone)]
struct AsmSweep {
    /// The assembly's file, which its sources' paths are relative to.
    path: Option<String>,
    /// The files read for it and the configurations built: a copy of the
    /// open document's, so a sweep never changes what it shows.
    asm: AsmState,
    grid: BTreeMap<i64, Arc<GridState>>,
}

/// A solved grid point of an assembly's motion.
struct GridState {
    /// The state reached: at the grid point, or where the motion stopped.
    carried: Carried,
    /// Where the motion stopped before this grid point, and why.
    stuck: Option<Stuck>,
}

/// A dead point: the last `t` the motion reached and why it goes no
/// further.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Stuck {
    pub(crate) t: f64,
    pub(crate) why: String,
}

impl Sweep {
    pub(crate) fn new(base: &Document, cache: RegenCache) -> Self {
        let sketches: Vec<usize> = base
            .features
            .iter()
            .enumerate()
            .filter(|(_, f)| matches!(f.kind, FeatureKind::Sketch(_)))
            .map(|(index, _)| index)
            .collect();
        let own: Vec<FeatureKind> = sketches
            .iter()
            .filter_map(|&k| base.features.get(k).map(|f| f.kind.clone()))
            .collect();
        let mut grid = BTreeMap::new();
        grid.insert(0, Arc::new(own));
        Self {
            reference: base.clone(),
            // Without a timeline the first `at` says so.
            t0: timeline(base).unwrap_or(0.0),
            sketches,
            grid,
            cache,
            assembly: None,
        }
    }

    /// The sweep of `base`, a state of `open`'s document: an assembly's
    /// moments are solved with the files read for it, a part's
    /// regenerated with its cache.
    pub(crate) fn of(open: &mut OpenDocument, base: &Document) -> Self {
        if base.is_assembly() {
            let mut sweep = Self::new(base, RegenCache::default());
            sweep.assembly = Some(AsmSweep {
                path: open.path.clone(),
                asm: open.asm.clone(),
                grid: BTreeMap::new(),
            });
            return sweep;
        }
        open.regen();
        Self::new(base, open.cache.clone())
    }

    /// The grid point `k` of `t`: the one between the reference's `t0`
    /// and `t` nearest to `t`.
    fn grid_index(&self, t: f64) -> Result<i64, String> {
        let steps = ((t - self.t0) / MOTION_STEP).trunc();
        if !steps.is_finite() || steps.abs() > MAX_STEPS as f64 {
            return Err(format!(
                "the document's own t is {}, too far from {t} to carry the motion there in steps of {MOTION_STEP} (at most {MAX_STEPS}); set t in the document nearer to it",
                self.t0
            ));
        }
        // Within ±MAX_STEPS.
        Ok(steps as i64)
    }

    /// An assembly's state at grid point `k`, made from the nearest one
    /// made toward it from 0.
    fn asm_grid_point(&mut self, k: i64) -> Result<Arc<GridState>, String> {
        let reference = &self.reference;
        let t0 = self.t0;
        let sweep = self
            .assembly
            .as_mut()
            .ok_or("the motion is not an assembly's")?;
        if !sweep.grid.contains_key(&0) {
            let doc = at_t(reference, t0)?;
            let placed = sweep.asm.solve_moment(&doc, sweep.path.as_deref(), None);
            let carried = placed.carried.clone().ok_or_else(|| {
                format!(
                    "the assembly does not solve at its own t ({t0}): {}",
                    placed
                        .solve_error
                        .as_deref()
                        .unwrap_or("no instance takes part")
                )
            })?;
            sweep.grid.insert(
                0,
                Arc::new(GridState {
                    carried,
                    stuck: None,
                }),
            );
        }
        let step = k.signum();
        let mut at = k;
        let mut from = loop {
            if let Some(found) = sweep.grid.get(&at) {
                break Arc::clone(found);
            }
            if at == 0 {
                return Err("the motion lost its reference".to_string());
            }
            at -= step;
        };
        while at != k {
            at += step;
            let made = if from.stuck.is_some() {
                // Past a dead point the motion stays where it stopped.
                Arc::clone(&from)
            } else {
                let t = t0 + at as f64 * MOTION_STEP;
                let doc = at_t(reference, t)?;
                let placed =
                    sweep
                        .asm
                        .solve_moment(&doc, sweep.path.as_deref(), Some(&from.carried));
                Arc::new(match placed.carried.clone() {
                    Some(carried) => GridState {
                        carried,
                        stuck: None,
                    },
                    None => GridState {
                        carried: from.carried.clone(),
                        stuck: Some(Stuck {
                            t: t - step as f64 * MOTION_STEP,
                            why: placed
                                .solve_error
                                .clone()
                                .unwrap_or_else(|| "it does not solve".to_string()),
                        }),
                    },
                })
            };
            sweep.grid.insert(at, Arc::clone(&made));
            from = made;
        }
        Ok(from)
    }

    /// An assembly's moment at `t`: placed from its grid point, with the
    /// dead point the motion stopped at on the way, if any (then the
    /// instances stay where it stopped).
    pub(crate) fn placed_at(&mut self, t: f64) -> Result<(Arc<Placed>, Option<Stuck>), String> {
        timeline(&self.reference)?;
        let k = self.grid_index(t)?;
        let from = self.asm_grid_point(k)?;
        let doc = at_t(&self.reference, t)?;
        let sweep = self
            .assembly
            .as_mut()
            .ok_or("the motion is not an assembly's")?;
        if let Some(stuck) = &from.stuck {
            let doc = at_t(&self.reference, stuck.t)?;
            let placed = sweep
                .asm
                .place_moment(&doc, sweep.path.as_deref(), Some(&from.carried));
            return Ok((Arc::new(placed), Some(stuck.clone())));
        }
        let placed = sweep
            .asm
            .place_moment(&doc, sweep.path.as_deref(), Some(&from.carried));
        let stuck = match (&placed.carried, &placed.solve_error) {
            (None, why) => Some(Stuck {
                t: self.t0 + k as f64 * MOTION_STEP,
                why: why
                    .clone()
                    .unwrap_or_else(|| "it does not solve".to_string()),
            }),
            _ => None,
        };
        Ok((Arc::new(placed), stuck))
    }

    /// True when this is the motion of an assembly.
    pub(crate) fn is_assembly(&self) -> bool {
        self.assembly.is_some()
    }

    /// The reference with `sketches` put in and `t` set.
    fn with(&self, sketches: &[FeatureKind], t: f64) -> Result<Document, String> {
        let mut doc = self.reference.clone();
        for (&index, kind) in self.sketches.iter().zip(sketches) {
            if let Some(feature) = doc.features.get_mut(index) {
                feature.kind = kind.clone();
            }
        }
        doc.set_variable(TIMELINE, &literal(t))
            .map_err(|e| e.to_string())?;
        Ok(doc)
    }

    /// The sketches of `doc` (the reference's sketch features, in order).
    fn sketches_of(&self, doc: &Document) -> Vec<FeatureKind> {
        self.sketches
            .iter()
            .filter_map(|&k| doc.features.get(k).map(|f| f.kind.clone()))
            .collect()
    }

    /// The sketches at grid point `k`, made from the nearest one made
    /// toward it from 0.
    fn grid_point(&mut self, k: i64) -> Result<Arc<Vec<FeatureKind>>, String> {
        let step = k.signum();
        let mut at = k;
        // The nearest one made, on the way from k back to 0.
        let mut from = loop {
            if let Some(found) = self.grid.get(&at) {
                break Arc::clone(found);
            }
            if at == 0 {
                return Err("the motion lost its reference".to_string());
            }
            at -= step;
        };
        while at != k {
            at += step;
            let t = self.t0 + at as f64 * MOTION_STEP;
            let mut doc = self.with(&from, t)?;
            craft3d_regen::settle_sketches_cached(&mut doc, &mut self.cache);
            let made = Arc::new(self.sketches_of(&doc));
            self.grid.insert(at, Arc::clone(&made));
            from = made;
        }
        Ok(from)
    }

    /// The state at `t`, and its regeneration.
    pub(crate) fn at(&mut self, t: f64) -> Result<(Arc<Document>, Arc<Regen>), String> {
        if self.assembly.is_some() {
            let (placed, _) = self.placed_at(t)?;
            return Ok((Arc::clone(&placed.doc), Arc::clone(&placed.regen)));
        }
        let doc = self.settled(t)?;
        let regen = craft3d_regen::regenerate_cached(&doc, &mut self.cache);
        Ok((Arc::new(doc), Arc::new(regen)))
    }

    /// The state at `t`, its sketches settled, not regenerated.
    pub(crate) fn settled(&mut self, t: f64) -> Result<Document, String> {
        timeline(&self.reference)?;
        if self.assembly.is_some() {
            return at_t(&self.reference, t);
        }
        let k = if self.sketches.is_empty() {
            0
        } else {
            self.grid_index(t)?
        };
        let from = self.grid_point(k)?;
        let mut doc = self.with(&from, t)?;
        craft3d_regen::settle_sketches_cached(&mut doc, &mut self.cache);
        Ok(doc)
    }
}

/// `doc` with its timeline set to `t`.
fn at_t(doc: &Document, t: f64) -> Result<Document, String> {
    let mut doc = doc.clone();
    doc.set_variable(TIMELINE, &literal(t))
        .map_err(|e| e.to_string())?;
    Ok(doc)
}

/// Two bodies that overlap at a `t`, or a pair the kernel could not decide.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Collision {
    pub(crate) a: FeatureId,
    pub(crate) b: FeatureId,
    /// The common volume; `None` for a pair the kernel refused.
    pub(crate) volume: Option<f64>,
    /// Why the pair is unknown.
    pub(crate) unknown: Option<String>,
}

/// What the kernel said about a pair: their common volume (0 for none),
/// or why it could not say.
type Answer = Result<f64, String>;

/// An answer with the two bodies it is about, held.
type Held = (Arc<Body>, Arc<Body>, Answer);

/// The key of an answer: the two bodies' addresses and the tolerance
/// (its bits, linear then angular) the kernel was asked with.
type PairKey = (usize, usize, u64, u64);

/// Answers about pairs of bodies by the bodies' addresses, holding the
/// bodies so that the addresses are not taken by others. It holds only
/// pairs of the state asked about last (both bodies in it), so it is
/// bounded by what one state checks and never holds bodies no state
/// shows any more.
#[derive(Clone, Default)]
pub(crate) struct PairMemo {
    answers: BTreeMap<PairKey, Held>,
}

impl PairMemo {
    /// How many answers are kept.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.answers.len()
    }
}

fn address(body: &Arc<Body>) -> usize {
    Arc::as_ptr(body) as usize
}

/// The pairs of the visible bodies of `regen` that overlap (or that the
/// kernel could not decide), in the order of the bodies. Bodies whose
/// boxes do not meet are clear at the cost of the boxes; a pair of bodies
/// asked about in the state before (the same bodies: a pair `t` does not
/// move) is not asked again.
pub(crate) fn collisions(
    doc: &Document,
    regen: &Regen,
    memo: &mut PairMemo,
) -> Result<Vec<Collision>, String> {
    collisions_kept(doc, regen, memo, MAX_MEMO)
}

/// [`collisions`], keeping at most `keep` answers for the next state.
/// Every answer of this state comes from this state's own map (the memo
/// only saves asking the kernel again), so how many are kept never
/// changes what is found.
fn collisions_kept(
    doc: &Document,
    regen: &Regen,
    memo: &mut PairMemo,
    keep: usize,
) -> Result<Vec<Collision>, String> {
    let tolerance = Tolerance::new(doc.tolerance.linear, doc.tolerance.angular)
        .ok_or("the document's tolerance is not positive and finite")?;
    let bits = (tolerance.linear.to_bits(), tolerance.angular.to_bits());
    let bodies: Vec<(FeatureId, &Arc<Body>, Option<craft3d_math::Bbox3>)> = regen
        .bodies()
        .filter(|(id, _)| !doc.is_hidden(*id))
        .map(|(id, body)| (id, body, body.bbox().ok()))
        .collect();
    if bodies.len() > MAX_COLLISION_BODIES {
        return Err(format!(
            "{} bodies are shown, more than the {MAX_COLLISION_BODIES} collisions are looked for among",
            bodies.len()
        ));
    }
    let margin = 2.0 * tolerance.linear;
    let key = |a: &Arc<Body>, b: &Arc<Body>| (address(a), address(b), bits.0, bits.1);
    // The pairs whose boxes meet, in order.
    let mut meeting = Vec::new();
    for (i, (_, _, abox)) in bodies.iter().enumerate() {
        for (j, (_, _, bbox)) in bodies.iter().enumerate().skip(i.saturating_add(1)) {
            if crate::interference_cmds::boxes_meet(abox.as_ref(), bbox.as_ref(), margin) {
                meeting.push((i, j));
            }
        }
    }
    if meeting.len() > MAX_COLLISION_PAIRS {
        return Err(format!(
            "{} pairs of bodies have boxes that meet, more than the {MAX_COLLISION_PAIRS} one state checks",
            meeting.len()
        ));
    }
    let mut answers: BTreeMap<(usize, usize), Answer> = BTreeMap::new();
    let mut ask = Vec::new();
    for &(i, j) in &meeting {
        let (Some(a), Some(b)) = (bodies.get(i), bodies.get(j)) else {
            continue;
        };
        match memo.answers.get(&key(a.1, b.1)) {
            Some(held) => {
                answers.insert((i, j), held.2.clone());
            }
            None => ask.push((i, j)),
        }
    }
    let indices: Vec<usize> = (0..ask.len()).collect();
    let found = crate::interference_cmds::in_order(&indices, |n| {
        let (i, j) = ask.get(n).copied()?;
        let (a, b) = (bodies.get(i)?.1, bodies.get(j)?.1);
        Some(match interference(a, b, &tolerance) {
            Ok(Interference::Overlap(common)) => craft3d_topo::mass_properties(&common.body)
                .map(|mass| mass.volume)
                .map_err(|e| format!("the common volume could not be measured: {e}")),
            Ok(_) => Ok(0.0),
            Err(e) => Err(e.to_string()),
        })
    });
    for (pair, answer) in ask.iter().zip(found) {
        let answer = answer.unwrap_or_else(|| Err("the query stopped unexpectedly".to_string()));
        answers.insert(*pair, answer);
    }
    // Kept for the next state: this state's pairs only, the first `keep`.
    memo.answers = meeting
        .iter()
        .take(keep)
        .filter_map(|&(i, j)| {
            let (a, b) = (bodies.get(i)?.1, bodies.get(j)?.1);
            let answer = answers.get(&(i, j))?.clone();
            Some((key(a, b), (Arc::clone(a), Arc::clone(b), answer)))
        })
        .collect();
    let mut out = Vec::new();
    for (&(i, j), answer) in &answers {
        let (Some((ida, ..)), Some((idb, ..))) = (bodies.get(i), bodies.get(j)) else {
            continue;
        };
        match answer {
            Ok(volume) if *volume > 0.0 => out.push(Collision {
                a: *ida,
                b: *idb,
                volume: Some(*volume),
                unknown: None,
            }),
            Err(reason) => out.push(Collision {
                a: *ida,
                b: *idb,
                volume: None,
                unknown: Some(reason.clone()),
            }),
            Ok(_) => {}
        }
    }
    Ok(out)
}

/// `doc` with the bodies of the overlapping pairs of `collisions` drawn
/// in [`COLLISION_COLOR`]: their own colour and their faces' colours at
/// the feature that has them give way to it (the copy only).
pub(crate) fn marked(doc: &Document, collisions: &[Collision]) -> Document {
    let mut out = doc.clone();
    let red: Vec<FeatureId> = collisions
        .iter()
        .filter(|c| c.volume.is_some())
        .flat_map(|c| [c.a, c.b])
        .collect();
    for id in &red {
        // A feature that has a body was given out by the document, so
        // the entry is valid; one that is refused leaves the colour.
        let _ = out.set_appearance(*id, Some(Appearance::color(COLLISION_COLOR)));
    }
    out.faces.retain(|entry| !red.contains(&entry.feature));
    out
}

/// One state of the motion the view shows.
#[derive(Clone)]
struct State {
    /// The document state it is a state of (the document, or the state
    /// the view is scrubbed to).
    base: Arc<Document>,
    /// The generation of the assembly's files it was made with
    /// (`AsmState::generation`): `assembly.reload` changes what a moment
    /// places without a new document state.
    generation: u64,
    t: f64,
    collisions_asked: bool,
    /// What is drawn: the state with its colliding bodies marked.
    doc: Arc<Document>,
    regen: Arc<Regen>,
    rebuilt: Vec<FeatureId>,
    collisions: Option<Result<Vec<Collision>, String>>,
    /// An assembly's dead point on the way to `t`: there the motion
    /// stopped, and the state shown is where it stopped.
    stuck: Option<Stuck>,
    /// An assembly's moment as placed: its instances, mates and solve.
    placed: Option<Arc<Placed>>,
}

/// A trace of a point over the motion: view state of one document state.
#[derive(Clone)]
pub(crate) struct Trace {
    base: Arc<Document>,
    /// As [`State::generation`].
    generation: u64,
    pub(crate) name: String,
    pub(crate) point: String,
    pub(crate) points: Vec<Vec3>,
    /// The `t` of the first and of the last point.
    pub(crate) t: (f64, f64),
    /// Drawn only up to the `t` the view shows.
    pub(crate) grow: bool,
}

/// Where the motion of an open document is: view state, not saved and
/// not undone.
#[derive(Clone, Default)]
pub(crate) struct Motion {
    /// The `t` the view shows; `None`: the document as it is.
    t: Option<f64>,
    collisions: bool,
    /// The run of states the view goes through, with the document state
    /// it runs over
    /// and the generation of the assembly's files it was made with.
    sweep: Option<(Arc<Document>, u64, Sweep)>,
    state: Option<State>,
    memo: PairMemo,
    traces: Vec<Trace>,
}

/// The features whose bodies differ between two regenerations: what a
/// state of the motion rebuilt.
fn rebuilt(base: &Regen, now: &Regen) -> Vec<FeatureId> {
    now.features
        .iter()
        .filter(|f| {
            let before = base.feature(f.id).and_then(|b| b.body.as_ref());
            match (before, f.body.as_ref()) {
                (Some(a), Some(b)) => !Arc::ptr_eq(a, b),
                (None, None) => false,
                _ => true,
            }
        })
        .map(|f| f.id)
        .collect()
}

impl OpenDocument {
    /// The state of the motion the view shows, made when it is not there
    /// for the state shown below it (an edit, a scrub); `None` when the
    /// view shows no motion. Lets go of the motion when the document has
    /// no timeline any more.
    fn motion_state(&mut self) -> Option<State> {
        let t = self.motion.t?;
        let base = self.scrubbed();
        if let Some(state) = &self.motion.state
            && Arc::ptr_eq(&state.base, &base)
            && state.generation == self.asm.generation
            && state.t == t
            && state.collisions_asked == self.motion.collisions
        {
            return Some(state.clone());
        }
        match self.make_motion_state(&base, t) {
            Ok(state) => {
                self.motion.state = Some(state.clone());
                Some(state)
            }
            Err(_) => {
                self.motion = Motion {
                    traces: std::mem::take(&mut self.motion.traces),
                    ..Motion::default()
                };
                None
            }
        }
    }

    fn make_motion_state(&mut self, base: &Arc<Document>, t: f64) -> Result<State, String> {
        timeline(base)?;
        let base_regen = self.scrubbed_regen();
        let generation = self.asm.generation;
        let reuse = self
            .motion
            .sweep
            .as_ref()
            .is_some_and(|(of, g, _)| Arc::ptr_eq(of, base) && *g == generation);
        if !reuse {
            let sweep = Sweep::of(self, base);
            self.motion.sweep = Some((Arc::clone(base), generation, sweep));
            // The answers were about the bodies of another base.
            self.motion.memo = PairMemo::default();
        }
        let (_, _, sweep) = self
            .motion
            .sweep
            .as_mut()
            .ok_or("the motion lost its states")?;
        let (plain, regen, stuck, placed) = if sweep.is_assembly() {
            let (placed, stuck) = sweep.placed_at(t)?;
            (
                Arc::clone(&placed.doc),
                Arc::clone(&placed.regen),
                stuck,
                Some(placed),
            )
        } else {
            let (plain, regen) = sweep.at(t)?;
            (plain, regen, None, None)
        };
        // An assembly's moment places its parts by transforms: nothing is
        // regenerated.
        let rebuilt = if sweep.is_assembly() {
            Vec::new()
        } else {
            rebuilt(&base_regen, &regen)
        };
        let collisions = self
            .motion
            .collisions
            .then(|| collisions(&plain, &regen, &mut self.motion.memo));
        let doc = match &collisions {
            Some(Ok(found)) if !found.is_empty() => Arc::new(marked(&plain, found)),
            _ => plain,
        };
        Ok(State {
            base: Arc::clone(base),
            generation,
            t,
            collisions_asked: self.motion.collisions,
            rebuilt,
            doc,
            regen,
            collisions,
            stuck,
            placed,
        })
    }

    /// The state of the document the view shows: the document itself,
    /// the state it is scrubbed to (`view.scrub`), or that at the `t` of
    /// its motion (`view.motion`).
    pub(crate) fn shown(&mut self) -> Arc<Document> {
        if let Some(state) = self.motion_state() {
            return state.doc;
        }
        // An assembly shows its placed bodies, as a part that holds them.
        if let Some(placed) = self.placed() {
            return Arc::clone(&placed.doc);
        }
        self.scrubbed()
    }

    /// For an assembly: its placement as the view shows it, at the moment
    /// of its motion when one is shown, else at rest.
    pub(crate) fn shown_placed(&mut self) -> Option<Arc<Placed>> {
        if let Some(placed) = self.motion_state().and_then(|state| state.placed) {
            return Some(placed);
        }
        self.placed()
    }

    /// The regeneration of [`Self::shown`].
    pub(crate) fn shown_regen(&mut self) -> Arc<Regen> {
        if let Some(state) = self.motion_state() {
            return state.regen;
        }
        if let Some(placed) = self.placed() {
            return Arc::clone(&placed.regen);
        }
        self.scrubbed_regen()
    }

    /// The traces of the document as it is; lets go of those of another
    /// state of it.
    fn traces(&mut self) -> &mut Vec<Trace> {
        let current = Arc::clone(self.history.current());
        let generation = self.asm.generation;
        self.motion
            .traces
            .retain(|trace| Arc::ptr_eq(&trace.base, &current) && trace.generation == generation);
        &mut self.motion.traces
    }
}

/// The motion of the active document as a viewer shows it
/// ([`Session::motion`]).
#[derive(Clone, Debug, PartialEq)]
pub struct MotionView {
    /// The `t` shown.
    pub t: f64,
    /// The pairs of bodies drawn red, by name.
    pub collisions: Vec<(String, String)>,
    /// How many features this `t` rebuilt.
    pub rebuilt: usize,
    /// For an assembly: the `t` of the dead point the motion stopped at on
    /// the way to `t`, when it did (the state shown is where it stopped).
    pub dead_point: Option<f64>,
}

/// A trace as a viewer draws it ([`Session::traces`]).
#[derive(Clone, Debug, PartialEq)]
pub struct TraceView {
    pub name: String,
    /// At `samples` values of `t` evenly from `t[0]` to `t[1]`, both
    /// included.
    pub points: Vec<Vec3>,
    pub t: [f64; 2],
    /// Drawn only up to the `t` the view shows ([`TraceView::up_to`]).
    pub grow: bool,
}

impl TraceView {
    /// The polyline as far as the motion has gone at `t`: the points
    /// sampled at or before it (in the trace's own direction, so a range
    /// that runs backwards grows backwards too), and the point between
    /// the last of those and the next one where `t` lies, along their
    /// segment. Nothing before the first `t`, the whole trace from the
    /// last one on; the whole trace when `t` is not a number or the range
    /// has no length.
    pub fn up_to(&self, t: f64) -> Vec<Vec3> {
        let [a, b] = self.t;
        let span = b - a;
        let last = self.points.len().saturating_sub(1);
        if !t.is_finite() || !span.is_finite() || span == 0.0 || last == 0 {
            return self.points.clone();
        }
        let at = (t - a) / span * last as f64;
        if !at.is_finite() || at >= last as f64 {
            return self.points.clone();
        }
        if at < 0.0 {
            return Vec::new();
        }
        let whole = at.floor();
        // `whole` lies in [0, last): a valid index.
        let index = whole as usize;
        let mut out: Vec<Vec3> = self.points.iter().take(index + 1).copied().collect();
        if let (Some(p), Some(q)) = (self.points.get(index), self.points.get(index + 1)) {
            let u = at - whole;
            if u > 0.0 {
                out.push(*p + (*q - *p) * u);
            }
        }
        out
    }
}

impl Session {
    /// The `t` the view of the active document shows (`view.motion`), with
    /// the pairs it draws red; `None` when it shows no motion.
    pub fn motion(&mut self) -> Option<MotionView> {
        let open = self.documents.get_mut(self.active)?;
        let state = open.motion_state()?;
        let name = |id: FeatureId| {
            state
                .doc
                .feature(id)
                .map_or_else(|| id.to_string(), |f| f.name.clone())
        };
        let collisions = match &state.collisions {
            Some(Ok(found)) => found
                .iter()
                .filter(|c| c.volume.is_some())
                .map(|c| (name(c.a), name(c.b)))
                .collect(),
            _ => Vec::new(),
        };
        Some(MotionView {
            t: state.t,
            collisions,
            rebuilt: state.rebuilt.len(),
            dead_point: state.stuck.as_ref().map(|stuck| stuck.t),
        })
    }

    /// The value of the timeline of the active document, or why it has
    /// none ([`timeline`]).
    pub fn timeline(&self) -> Result<f64, String> {
        let open = self
            .documents
            .get(self.active)
            .ok_or("no document is open")?;
        timeline(open.history.current())
    }

    /// The traces of the active document (`view.trace`), in the order
    /// they were made.
    pub fn traces(&mut self) -> Vec<TraceView> {
        let Some(open) = self.documents.get_mut(self.active) else {
            return Vec::new();
        };
        open.traces()
            .iter()
            .map(|trace| TraceView {
                name: trace.name.clone(),
                points: trace.points.clone(),
                t: [trace.t.0, trace.t.1],
                grow: trace.grow,
            })
            .collect()
    }
}

fn view_motion(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "view.motion";
    let p = Params::new(ID, params, &["t", "collisions"])?;
    let collisions = p.bool("collisions")?;
    // Null is asked for (the document as it is), not absent.
    let t = match (p.has("t"), p.get("t")) {
        (false, _) => None,
        (true, None) => Some(None),
        (true, Some(value)) => Some(Some(t_value(ID, "t", value)?)),
    };
    let open = session.document_mut()?;
    let rest = timeline(open.history.current());
    if let Some(Some(_)) = t {
        rest.clone()
            .map_err(|e| CommandError::new(format!("{ID}: {e}")))?;
    }
    if let Some(t) = t {
        open.motion.t = t;
        if t.is_none() {
            open.motion.state = None;
            open.motion.sweep = None;
            open.motion.memo = PairMemo::default();
        }
    }
    if let Some(collisions) = collisions {
        open.motion.collisions = collisions;
    }
    if open.motion.t.is_some() {
        // Made now, so the state's errors are this command's.
        let base = open.scrubbed();
        let t = open.motion.t.unwrap_or(0.0);
        let generation = open.asm.generation;
        let stale = open.motion.state.as_ref().is_none_or(|s| {
            !Arc::ptr_eq(&s.base, &base)
                || s.generation != generation
                || s.t != t
                || s.collisions_asked != open.motion.collisions
        });
        if stale {
            let state = open
                .make_motion_state(&base, t)
                .map_err(|e| CommandError::new(format!("{ID}: {e}")))?;
            open.motion.state = Some(state);
        }
    }
    let state = open.motion_state();
    let doc = open.history.current();
    let timeline_json = match &rest {
        Ok(value) => json!({"variable": TIMELINE, "value": value}),
        Err(why) => json!({"variable": TIMELINE, "error": why}),
    };
    let Some(state) = state else {
        return Ok(json!({"t": Value::Null, "timeline": timeline_json, "collisions": Value::Null}));
    };
    // Named as the state shows them: an assembly's bodies by instance.
    let name = |id: FeatureId| {
        state
            .doc
            .feature(id)
            .map_or_else(|| id.to_string(), |f| f.name.clone())
    };
    let collisions = match &state.collisions {
        None => Value::Null,
        Some(Err(why)) => json!({"error": why}),
        Some(Ok(found)) => found
            .iter()
            .map(|c| {
                let mut entry = json!({"a": c.a.to_string(), "aName": name(c.a), "b": c.b.to_string(), "bName": name(c.b)});
                if let Some(map) = entry.as_object_mut() {
                    match (&c.volume, &c.unknown) {
                        (Some(volume), _) => {
                            map.insert("volume".into(), (*volume).into());
                        }
                        (None, Some(why)) => {
                            map.insert("unknown".into(), why.clone().into());
                        }
                        _ => {}
                    }
                }
                entry
            })
            .collect(),
    };
    Ok(json!({
        "t": state.t,
        "timeline": timeline_json,
        "rebuilt": state.rebuilt.iter().map(ToString::to_string).collect::<Vec<_>>(),
        "collisions": collisions,
        "volumeUnit": format!("{}^3", length_unit(doc)),
        "deadPoint": state.stuck.as_ref().map_or(Value::Null, |stuck| json!({"t": stuck.t, "why": stuck.why})),
        "solveNote": state.placed.as_ref().and_then(|p| p.solve_note.clone()),
    }))
}

/// A point to trace.
#[derive(Clone, Debug)]
enum PointRef {
    /// A vertex by its persistent name, in the body of `feature` when
    /// given.
    Vertex {
        name: String,
        feature: Option<String>,
    },
    /// A point of a sketch.
    Sketch { sketch: FeatureId, entity: EntityId },
}

impl PointRef {
    fn parse(command: &str, doc: &Document, p: &Params) -> Result<Self, CommandError> {
        let feature = p.str("feature")?.map(str::to_string);
        match p.get("point") {
            Some(Value::String(name)) => Ok(Self::Vertex {
                name: name.clone(),
                feature,
            }),
            Some(Value::Object(map)) => {
                if feature.is_some() {
                    return fail(format!(
                        "{command}: `feature` says whose vertex is meant; a sketch point is named by its sketch"
                    ));
                }
                let sketch = match map.get("sketch") {
                    Some(Value::String(text)) => feature_named(doc, text),
                    Some(Value::Number(n)) => n
                        .as_u64()
                        .map(FeatureId)
                        .filter(|id| doc.feature(*id).is_some()),
                    _ => None,
                }
                .ok_or_else(|| {
                    CommandError::new(format!(
                        "{command}: `point.sketch` must name a sketch of the document (its id or name), got {}",
                        map.get("sketch").map_or("nothing".to_string(), short)
                    ))
                })?;
                let entity = map
                    .get("entity")
                    .ok_or_else(|| {
                        CommandError::new(format!(
                            "{command}: `point` needs `entity`, the id of a point of the sketch (e3)"
                        ))
                    })
                    .and_then(|v| crate::sketch_cmds::entity_id(command, "point.entity", v))?;
                Ok(Self::Sketch { sketch, entity })
            }
            Some(other) => fail(format!(
                "{command}: `point` is a vertex's persistent name or {{\"sketch\", \"entity\"}}, got {}",
                short(other)
            )),
            None => Err(p.missing("point")),
        }
    }

    fn describe(&self) -> String {
        match self {
            Self::Vertex { name, .. } => name.clone(),
            Self::Sketch { sketch, entity } => format!("{sketch}/{entity}"),
        }
    }

    /// Where the point is in `doc` as `regen` built it.
    fn at(&self, doc: &Document, regen: &Regen) -> Result<Vec3, String> {
        match self {
            Self::Vertex { name, feature } => {
                let mut resolver = Resolver::new(doc, regen);
                let found =
                    resolver.resolve(Some(SelectionKind::Vertex), name, feature.as_deref())?;
                let id = feature_named(doc, &found.item.feature)
                    .ok_or_else(|| format!("there is no feature {}", found.item.feature))?;
                let result = regen
                    .feature(id)
                    .ok_or_else(|| format!("{id} was not regenerated"))?;
                let (Some(body), Some(names)) = (result.body.as_ref(), result.names.as_ref())
                else {
                    return Err(format!("{id} has no body"));
                };
                (0..body.vertices.len())
                    .find(|&k| {
                        names
                            .vertex_name(k)
                            .is_some_and(|n| n.to_string() == found.item.name)
                    })
                    .and_then(|k| body.vertices.get(k))
                    .map(|v| v.point)
                    .ok_or_else(|| format!("no body has the vertex `{}`", found.item.name))
            }
            Self::Sketch { sketch, entity } => {
                let solved = regen
                    .feature(*sketch)
                    .and_then(|f| f.sketch.as_ref())
                    .ok_or_else(|| format!("{sketch} is not a solved sketch"))?;
                let (_, x, y) = solved
                    .points
                    .iter()
                    .find(|(id, ..)| id == entity)
                    .ok_or_else(|| format!("{entity} is not a point of the sketch {sketch}"))?;
                Ok(solved.origin + solved.x_axis * *x + solved.y_axis * *y)
            }
        }
    }
}

/// How far from its plane a trace may be, as a part of its size (the
/// greatest distance of a point from its centre), besides the document's
/// tolerance: a path much smaller than the tolerance is not flat just
/// because all of it is within the tolerance.
const FLAT_RELATIVE: f64 = 1e-6;

/// The plane a polyline lies in, within `tolerance` and within
/// [`FLAT_RELATIVE`] of its size (whichever is less, never below the
/// rounding of its coordinates): a point of it and its normal, `None`
/// when it does not lie in one or spans none (a point, a straight line).
fn plane_of(points: &[Vec3], tolerance: f64) -> Option<(Vec3, Vec3)> {
    let count = points.len() as f64;
    let sum = points.iter().fold(Vec3::new(0.0, 0.0, 0.0), |s, p| s + *p);
    let center = sum * (1.0 / count.max(1.0));
    // The point farthest from the centre, then the one farthest from the
    // line through both: they span the plane, if there is one.
    let far = points
        .iter()
        .copied()
        .max_by(|a, b| (*a - center).length().total_cmp(&(*b - center).length()))?;
    let size = (far - center).length();
    let magnitude = points.iter().fold(0.0_f64, |m, p| {
        m.max(p.x.abs()).max(p.y.abs()).max(p.z.abs())
    });
    let tolerance = tolerance
        .min(FLAT_RELATIVE * size)
        .max(64.0 * f64::EPSILON * magnitude);
    let along = (far - center).normalized()?;
    let off = points.iter().copied().max_by(|a, b| {
        let d = |p: Vec3| (p - center).cross(along).length();
        d(*a).total_cmp(&d(*b))
    })?;
    if (off - center).cross(along).length() <= tolerance {
        return None;
    }
    let normal = along.cross(off - center).normalized()?;
    // Pointing along the world axis it is most along, so the drawing is
    // not mirrored by chance.
    let main = [normal.x, normal.y, normal.z]
        .into_iter()
        .max_by(|a, b| a.abs().total_cmp(&b.abs()))
        .unwrap_or(1.0);
    let normal = if main < 0.0 { normal * -1.0 } else { normal };
    points
        .iter()
        .all(|p| (*p - center).dot(normal).abs() <= tolerance)
        .then_some((center, normal))
}

/// Whether a polyline is closed: more than 2 points (two are a segment
/// there and back, not a loop) and its last within `tolerance` of its
/// first. `view.trace` and its export say the same.
fn ends_where_it_began(points: &[Vec3], tolerance: f64) -> bool {
    match (points.first(), points.last()) {
        (Some(a), Some(b)) => points.len() > 2 && (*a - *b).length() <= tolerance,
        _ => false,
    }
}

fn polyline_length(points: &[Vec3]) -> f64 {
    points
        .windows(2)
        .filter_map(|w| Some((*w.get(1)? - *w.first()?).length()))
        .sum()
}

fn view_trace(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "view.trace";
    let p = Params::new(
        ID,
        params,
        &["point", "feature", "t", "samples", "name", "grow", "clear"],
    )?;
    if let Some(clear) = p.get("clear") {
        let others: Vec<&str> = ["point", "feature", "t", "samples", "name", "grow"]
            .into_iter()
            .filter(|k| p.has(k))
            .collect();
        if let Some(key) = others.first() {
            return fail(format!(
                "{ID}: `clear` removes traces; it does not go with `{key}`"
            ));
        }
        let open = session.document_mut()?;
        let traces = open.traces();
        let before = traces.len();
        match clear {
            Value::Bool(true) => traces.clear(),
            Value::Bool(false) => {}
            Value::String(name) => {
                traces.retain(|trace| &trace.name != name);
                if traces.len() == before {
                    return fail(format!("{ID}: there is no trace `{}`", short(clear)));
                }
            }
            other => {
                return fail(format!(
                    "{ID}: `clear` is true (every trace) or the name of a trace, got {}",
                    short(other)
                ));
            }
        }
        let after = traces.len();
        return Ok(json!({"removed": before.saturating_sub(after), "traces": after}));
    }
    let range = t_range(ID, &p)?;
    let samples = p.integer("samples", 2, MAX_SAMPLES)?.unwrap_or(65);
    let grow = p.bool("grow")?.unwrap_or(false);
    let given_name = p.str("name")?.map(str::to_string);
    if given_name
        .as_deref()
        .is_some_and(|n| n.is_empty() || n.len() > 200)
    {
        return fail(format!("{ID}: `name` must have 1 to 200 bytes"));
    }
    let open = session.document_mut()?;
    let base = Arc::clone(open.history.current());
    timeline(&base).map_err(|e| CommandError::new(format!("{ID}: {e}")))?;
    let point = PointRef::parse(ID, &base, &p)?;
    let mut sweep = Sweep::of(open, &base);
    let mut points = Vec::new();
    let in_assembly = match (&point, sweep.is_assembly()) {
        (_, false) => None,
        (
            PointRef::Vertex {
                name,
                feature: None,
            },
            true,
        ) => Some(name.clone()),
        _ => {
            return fail(format!(
                "{ID}: in an assembly the point is named through its instance, as a mate names it (`Coupler/End2`, `Foot/F1:vertex(2)`), without `feature`"
            ));
        }
    };
    for t in samples_of(range, samples) {
        let err =
            |e: String| CommandError::new(format!("{ID}: `{}` at t = {t}: {e}", point.describe()));
        let at = if let Some(name) = &in_assembly {
            let (placed, _) = sweep
                .placed_at(t)
                .map_err(|e| CommandError::new(format!("{ID}: at t = {t}: {e}")))?;
            crate::asm::point_in(&placed, name).map_err(err)?
        } else {
            let (doc, regen) = sweep
                .at(t)
                .map_err(|e| CommandError::new(format!("{ID}: at t = {t}: {e}")))?;
            point.at(&doc, &regen).map_err(err)?
        };
        points.push(at);
    }
    let tolerance = base.tolerance.linear;
    let generation = open.asm.generation;
    let traces = open.traces();
    let name = given_name.unwrap_or_else(|| {
        (1..)
            .map(|n| format!("Trace {n}"))
            .find(|n| traces.iter().all(|t| &t.name != n))
            .unwrap_or_else(|| "Trace".to_string())
    });
    traces.retain(|trace| trace.name != name);
    if traces.len() >= MAX_TRACES {
        return fail(format!(
            "{ID}: {MAX_TRACES} traces are kept already; remove some with clear"
        ));
    }
    traces.push(Trace {
        base: Arc::clone(&base),
        generation,
        name: name.clone(),
        point: point.describe(),
        points: points.clone(),
        t: range,
        grow,
    });
    let closed = ends_where_it_began(&points, tolerance);
    let bbox = craft3d_math::Bbox3::from_points(points.iter().copied());
    let plane = plane_of(&points, tolerance).map_or(
        Value::Null,
        |(at, n)| json!({"point": vec3(at), "normal": vec3(n)}),
    );
    Ok(json!({
        "name": name,
        "point": point.describe(),
        "t": [range.0, range.1],
        "samples": samples,
        "grow": grow,
        "points": points.iter().map(|p| vec3(*p)).collect::<Vec<_>>(),
        "length": polyline_length(&points),
        "closed": closed,
        "box": bbox.corners().map(|(lo, hi)| json!([vec3(lo), vec3(hi)])),
        "plane": plane,
        "unit": length_unit(&base),
    }))
}

/// The drawing of a trace for `file.export {trace}`: the polyline in its
/// plane, its x axis the world axis least along the plane's normal, laid
/// into the plane, and y the normal crossed with it.
pub(crate) fn trace_drawing(
    session: &mut Session,
    p: &Params,
    command: &str,
) -> Result<crate::drawing_cmds::Source, CommandError> {
    let Some(Value::String(name)) = p.get("trace") else {
        return fail(format!(
            "{command}: `trace` is the name of a trace (view.trace), got {}",
            p.get("trace").map_or("nothing".to_string(), short)
        ));
    };
    let open = session.document_mut()?;
    let tolerance = open.history.current().tolerance.linear;
    let traces = open.traces();
    let Some(trace) = traces.iter().find(|t| &t.name == name) else {
        let kept: Vec<String> = traces.iter().map(|t| format!("`{}`", t.name)).collect();
        return fail(format!(
            "{command}: there is no trace `{}` (kept: {}); make it with view.trace (traces are let go of at an edit)",
            name.chars().take(80).collect::<String>(),
            if kept.is_empty() {
                "none".to_string()
            } else {
                kept.join(", ")
            }
        ));
    };
    let (center, normal) = plane_of(&trace.points, tolerance).ok_or_else(|| {
        CommandError::new(format!(
            "{command}: the trace `{}` does not lie in one plane within the tolerance (or is a point or a straight line), so it has no flat drawing",
            trace.name
        ))
    })?;
    let axes = [
        Vec3::new(1.0, 0.0, 0.0),
        Vec3::new(0.0, 1.0, 0.0),
        Vec3::new(0.0, 0.0, 1.0),
    ];
    let x = axes
        .into_iter()
        .min_by(|a, b| a.dot(normal).abs().total_cmp(&b.dot(normal).abs()))
        .and_then(|a| (a - normal * a.dot(normal)).normalized())
        .ok_or_else(|| CommandError::new(format!("{command}: the trace's plane has no axes")))?;
    let y = normal.cross(x);
    let flat: Vec<[f64; 2]> = trace
        .points
        .iter()
        .map(|p| [(*p - center).dot(x), (*p - center).dot(y)])
        .collect();
    let closed = ends_where_it_began(&trace.points, tolerance);
    let mut ends = flat.clone();
    if closed {
        ends.pop();
    }
    let mut chain = ends.into_iter();
    let start = chain
        .next()
        .ok_or_else(|| CommandError::new(format!("{command}: the trace has no points")))?;
    let segments: Vec<craft3d_drawing::Segment> = chain
        .map(|to| craft3d_drawing::Segment::Line { to })
        .collect();
    Ok(crate::drawing_cmds::Source {
        json: json!({
            "trace": trace.name,
            "point": trace.point,
            "plane": {"point": vec3(center), "normal": vec3(normal), "xAxis": vec3(x)},
        }),
        layers: Vec::new(),
        paths: vec![craft3d_drawing::Path::Chain {
            start,
            segments,
            closed,
        }],
        construction: Vec::new(),
    })
}

/// One assertion over the motion: what each sample found.
struct Over {
    name: String,
    measure: String,
    expected: String,
    unit: Option<String>,
    samples: Vec<(f64, crate::assert_cmds::AssertionStatus, Value, String)>,
}

fn motion_check(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "motion.check";
    use crate::assert_cmds::AssertionStatus as S;
    let p = Params::new(
        ID,
        params,
        &["assertions", "measure", "t", "samples", "details"],
    )?;
    let range = t_range(ID, &p)?;
    let samples = p.integer("samples", 2, MAX_SAMPLES)?.unwrap_or(33);
    let details = p.bool("details")?.unwrap_or(false);
    let open = session.document_mut()?;
    let base = Document::clone(open.history.current());
    timeline(&base).map_err(|e| CommandError::new(format!("{ID}: {e}")))?;
    open.regen();
    let cache = open.cache.clone();

    // A session of its own on a copy, so the document and its checks do
    // not change; recovery does not see it.
    let mut scratch = Session::new(Workspace::none());
    // An assembly's copy places its parts from the files read for it.
    let (path, asm) = (open.path.clone(), open.asm.clone());
    let mut copy = OpenDocument::new(base, None);
    copy.cache = cache;
    copy.path = path;
    copy.asm = asm;
    scratch.add_document(copy)?;
    let mut names: Vec<String> = Vec::new();
    if let Some(measure) = p.get("measure") {
        let Value::Object(map) = measure else {
            return fail(format!(
                "{ID}: `measure` is an assertion as assert.add takes it, without its name, got {}",
                short(measure)
            ));
        };
        if map.contains_key("name") {
            return fail(format!(
                "{ID}: `measure` has no name; it is checked here only"
            ));
        }
        let doc = scratch.document()?.history.current();
        let name = (1..)
            .map(|n| format!("overMotion{n}"))
            .find(|n| doc.assertion(n).is_none())
            .unwrap_or_else(|| "overMotion".to_string());
        let mut map = map.clone();
        map.insert("name".into(), name.clone().into());
        scratch
            .execute("assert.add", &Value::Object(map))
            .map_err(|e| CommandError::new(format!("{ID}: measure: {}", e.message())))?;
        names.push(name);
    }
    let doc = scratch.document()?.history.current();
    match p.get("assertions") {
        None | Some(Value::Null) if names.is_empty() => {
            names = doc.assertions.iter().map(|a| a.name.clone()).collect();
            if names.is_empty() {
                return fail(format!(
                    "{ID}: the document has no assertions; give `measure` (an assertion as assert.add takes it) or add some with assert.add"
                ));
            }
        }
        None | Some(Value::Null) => {}
        Some(Value::Array(items)) => {
            for item in items {
                let Some(name) = item.as_str() else {
                    return fail(format!(
                        "{ID}: `assertions` is a list of assertion names, got {}",
                        short(item)
                    ));
                };
                if doc.assertion(name).is_none() {
                    return fail(format!(
                        "{ID}: there is no assertion `{}`",
                        name.chars().take(80).collect::<String>()
                    ));
                }
                if !names.iter().any(|n| n == name) {
                    names.push(name.to_string());
                }
            }
            if names.is_empty() {
                return fail(format!("{ID}: `assertions` names none"));
            }
        }
        Some(other) => {
            return fail(format!(
                "{ID}: `assertions` is a list of assertion names, got {}",
                short(other)
            ));
        }
    }

    let mut over: Vec<Over> = names
        .iter()
        .map(|name| Over {
            name: name.clone(),
            measure: String::new(),
            expected: String::new(),
            unit: None,
            samples: Vec::new(),
        })
        .collect();
    // Each sample is the state view.motion shows at its t (a function of
    // t alone), put into the copy as one undo step merged over the run.
    let mut sweep = {
        let open = scratch.document_mut()?;
        let base = Arc::clone(open.history.current());
        Sweep::of(open, &base)
    };
    let mut dead_points: Vec<Stuck> = Vec::new();
    for t in samples_of(range, samples) {
        let at = |e: String| CommandError::new(format!("{ID}: at t = {t}: {e}"));
        // An assembly's moment is solved along the motion, and the copy
        // measures that placement, not a solve from its placements.
        let moment = if sweep.is_assembly() {
            let (placed, stuck) = sweep.placed_at(t).map_err(at)?;
            if let Some(stuck) = stuck
                && !dead_points.contains(&stuck)
            {
                dead_points.push(stuck);
            }
            Some(placed)
        } else {
            None
        };
        let state = sweep.settled(t).map_err(at)?;
        let open = scratch.document_mut()?;
        open.history
            .apply_merging("Motion", Some("motion.check.t"), true, |doc| {
                *doc = state;
                Ok::<(), CommandError>(())
            })?;
        if let Some(placed) = moment {
            let revision = open.history.revision();
            open.asm.hold(revision, placed);
        }
        let checks = scratch.assertion_checks(crate::assert_check::Wait::All)?;
        for entry in &mut over {
            let Some(checked) = checks.iter().find(|c| c.state.name == entry.name) else {
                continue;
            };
            if entry.measure.is_empty() {
                entry.measure.clone_from(&checked.state.measure);
                entry.expected.clone_from(&checked.state.expected);
                entry.unit.clone_from(&checked.unit);
            }
            entry.samples.push((
                t,
                checked.state.status,
                checked.value.clone(),
                checked.state.message.clone(),
            ));
        }
    }

    let mut all_passed = true;
    let results: Vec<Value> = over
        .iter()
        .map(|entry| {
            let any = |s: S| entry.samples.iter().any(|x| x.1 == s);
            let status = if any(S::Failed) {
                S::Failed
            } else if any(S::Error) {
                S::Error
            } else if any(S::NotProven) {
                S::NotProven
            } else if entry.samples.iter().all(|x| x.1 == S::Inactive) {
                S::Inactive
            } else {
                S::Passed
            };
            all_passed &= status == S::Passed;
            // The first sample of a status, with the last one before it
            // that passed: the change lies between the two.
            let first = |status: S| {
                let Some(at) = entry.samples.iter().position(|x| x.1 == status) else {
                    return Value::Null;
                };
                let after = entry
                    .samples
                    .iter()
                    .take(at)
                    .rev()
                    .find(|x| x.1 == S::Passed)
                    .map_or(Value::Null, |x| x.0.into());
                entry.samples.get(at).map_or(Value::Null, |(t, _, value, message)| {
                    json!({"t": t, "after": after, "measured": value, "message": message})
                })
            };
            let numbers = entry
                .samples
                .iter()
                .filter_map(|(t, _, value, _)| Some((*t, value.as_f64()?)));
            let min = numbers
                .clone()
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map_or(Value::Null, |(t, v)| json!({"t": t, "value": v}));
            let max = numbers
                .max_by(|a, b| a.1.total_cmp(&b.1))
                .map_or(Value::Null, |(t, v)| json!({"t": t, "value": v}));
            let mut out = Map::new();
            out.insert("name".into(), entry.name.clone().into());
            out.insert("measure".into(), entry.measure.clone().into());
            out.insert("status".into(), status.name().into());
            out.insert(
                "failed".into(),
                entry
                    .samples
                    .iter()
                    .filter(|x| x.1 == S::Failed)
                    .count()
                    .into(),
            );
            out.insert("firstFailure".into(), first(S::Failed));
            out.insert("firstError".into(), first(S::Error));
            out.insert("firstNotProven".into(), first(S::NotProven));
            out.insert("min".into(), min);
            out.insert("max".into(), max);
            out.insert("unit".into(), entry.unit.clone().into());
            out.insert("expected".into(), entry.expected.clone().into());
            if details {
                out.insert(
                    "samples".into(),
                    entry
                        .samples
                        .iter()
                        .map(|(t, s, v, _)| json!({"t": t, "status": s.name(), "measured": v}))
                        .collect(),
                );
            }
            Value::Object(out)
        })
        .collect();
    Ok(json!({
        "t": [range.0, range.1],
        "samples": samples,
        "sampled": true,
        "ok": all_passed,
        "assertions": results,
        "deadPoints": dead_points.iter().map(|d| json!({"t": d.t, "why": d.why})).collect::<Vec<_>>(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft3d_math::Portable;

    /// Every pair of three bodies in one place collides, however few
    /// answers are kept between states: an answer kept or not is asked
    /// again or taken from the memo, never taken for clear.
    #[test]
    fn collisions_do_not_vanish_when_few_answers_are_kept() {
        let mut s = Session::new(Workspace::none());
        s.execute("file.new", &json!({})).unwrap();
        for name in ["A", "B", "C"] {
            s.execute(
                "feature.block",
                &json!({"name": name, "size": ["10 mm", "10 mm", "10 mm"]}),
            )
            .unwrap();
        }
        let open = s.document_mut().unwrap();
        let doc = Arc::clone(open.history.current());
        let regen = open.regen();
        let mut memo = PairMemo::default();
        for keep in [1, 1, 0, 1, 3, 3, 1] {
            let found = collisions_kept(&doc, &regen, &mut memo, keep).unwrap();
            assert_eq!(found.len(), 3, "keeping {keep}: {found:?}");
            assert!(found.iter().all(|c| c.volume.is_some()));
            assert!(memo.len() <= keep, "{} kept, at most {keep}", memo.len());
        }
        // A state without those bodies lets go of their answers.
        s.execute("feature.delete", &json!({"feature": "C"}))
            .unwrap();
        s.execute("feature.delete", &json!({"feature": "B"}))
            .unwrap();
        let open = s.document_mut().unwrap();
        let doc = Arc::clone(open.history.current());
        let regen = open.regen();
        assert!(
            collisions_kept(&doc, &regen, &mut memo, 3)
                .unwrap()
                .is_empty()
        );
        assert_eq!(memo.len(), 0);
    }

    #[test]
    fn samples_run_from_end_to_end() {
        assert_eq!(samples_of((0.0, 1.0), 5), vec![0.0, 0.25, 0.5, 0.75, 1.0]);
        assert_eq!(samples_of((1.0, 0.0), 2), vec![1.0, 0.0]);
        let odd = samples_of((0.1, 0.7), 7);
        assert_eq!(odd.first(), Some(&0.1));
        assert_eq!(odd.last(), Some(&0.7));
    }

    #[test]
    fn a_literal_reads_back_as_the_same_number() {
        for t in [0.0, 1.0, 0.1, 1.0 / 3.0, 1e-9, 0.999_999_999_999] {
            let back: f64 = literal(t).parse().unwrap();
            assert_eq!(back, t);
        }
    }

    #[test]
    fn a_plane_is_found_only_where_the_points_span_one() {
        let circle: Vec<Vec3> = (0..16)
            .map(|k| {
                let a = f64::from(k) * std::f64::consts::TAU / 16.0;
                Vec3::new(3.0 + a.pcos(), 5.0, a.psin())
            })
            .collect();
        let (_, n) = plane_of(&circle, 1e-9).unwrap();
        assert!((n - Vec3::new(0.0, 1.0, 0.0)).length() < 1e-12);
        let line: Vec<Vec3> = (0..5).map(|k| Vec3::new(f64::from(k), 0.0, 0.0)).collect();
        assert!(plane_of(&line, 1e-9).is_none());
        let mut bent = circle.clone();
        bent.push(Vec3::new(3.0, 6.0, 0.0));
        assert!(plane_of(&bent, 1e-9).is_none());
        assert!(plane_of(&[], 1e-9).is_none());
    }
}
