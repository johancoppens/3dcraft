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
use craft3d_kernel::boolean::common_volume;
use craft3d_math::{Tolerance, Vec3};
use craft3d_regen::{Regen, RegenCache};
use craft3d_topo::Body;
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

pub(crate) static SPECS: &[CommandSpec] = &[
    spec!(
        "view.motion",
        "Show Motion",
        r#"{"t": number from 0 to 1 or null, "collisions": boolean = as before (false), "shapes": "exact" or "ready" = "exact"}"#,
        "Shows the model in the view at a moment t of its motion, without changing the document: not its variables, its undo history or its dirty state (view state, as view.scrub). The timeline is the document's variable t when it is a plain number (no unit): formulas of t drive the motion (a crank angle 360 deg * t). Only the features that depend on t are regenerated, and only their bodies are meshed again; each state is a function of t alone: its sketches are carried from the document's own solution to t in fixed steps of t (1/128), so a linkage solved in a sketch stays on its branch, and a moment reached by a jump is the one a play reaches. collisions: true draws the bodies that overlap at t red (in the scene, so view.render, the window and view.replay show it) and lists the pairs. t null shows the document as it is again. The viewport, view.render, view.pick and screenshots show the state; document.inspect, measures and exports still see the document itself. An edit keeps the motion (the state is made again from the edited document); a document whose t is gone or has a unit lets go of it. Without parameters it changes nothing. Returns t (null when not shown), the timeline (its variable and its value in the document), rebuilt (the features whose bodies differ from the document's at this t) and collisions ([{a, aName, b, bName, volume}], or unknown pairs with a reason) when asked for. An assembly plays t too: drivers are formulas of t for its joint variables (Drive_angle = 360 deg * t), each moment is a solve carried along the branch from the assembly at rest (never a regeneration), and where the mechanism cannot go further the result has deadPoint {t, why}: the t it stopped at, the parts shown where they stopped. shapes: \"ready\" (for a viewer that plays the motion; for this call only) answers an assembly's moment at once: an instance whose parameters follow t and whose configuration at t is not built yet is shown with the configuration of its own built nearest in t (listed in stale), the solve carried from the nearest fixed step that is solved, while the configurations it lacks are built in the background (natively; in a browser the moment is made exactly, as without it); the result then has exact: false, and the moment settles, the same as with shapes: \"exact\", once what it lacks is built (the next view.motion, or a viewer's poll, takes it in). Only that answer and the window's own viewport show a provisional moment: view.render, view.pick, screenshots, recordings, exports and every later command show the exact moment. Also returned: exact, stale (the instances shown with another configuration of theirs), building (configurations being built in the background) and baked (the frames of the last motion.bake, which a player rounds its t to, or null). Where the view is made beside the host (the browser's worker), the moment is made there: the answer has elsewhere: true and pending: true until it came (then what view.motion answered there), and error when it could not be made there.",
        doc: true,
        view_motion
    ),
    spec!(
        "view.trace",
        "Trace Point",
        r#"{"point": vertex name or {sketch, entity}, "feature": id or name = the body that has the vertex, "t": [number, number] = [0, 1], "samples": integer = 65, "name": string = "Trace <n>", "grow": boolean = false, "clear": boolean or trace name, "background": boolean = true in a window, else false, "wait": boolean = false} or {"name": string, "wait": boolean = false}"#,
        "Traces the path of a point over the motion: the point is a vertex by its persistent name (document.edges lists them; feature says which body when several have it) or a point of a sketch ({sketch, entity}); its place is taken at samples values of t evenly from t[0] to t[1] (both included), each state made as view.motion makes it. The trace is view state of the document (not saved, not undone, let go of at the next edit): the viewport draws it as a line, and file.export {path: *.svg or *.dxf, trace: name} writes it in its plane. With grow: true the viewport draws the trace only up to the t it shows while it shows a moment of the motion (the player draws it as it goes), whole otherwise. A trace with the name of one kept replaces it; at most 32 are kept. clear: true removes every trace, clear: name one of them (nothing is traced then). The document needs a timeline (view.motion). A point that is gone at some t fails the trace, naming that t. Returns name, point, t, samples, grow, points ([x, y, z] in the document's unit), length (along the polyline), closed (more than 2 points and it ends where it began, within the tolerance), box and plane ({point, normal} when the path lies in one plane within the tolerance and within a millionth of its size, else null). In an assembly the point is named through an instance as a mate names it (Coupler/End2, Foot/F1:vertex(2)), without feature. With background: true (a window's default) the points are worked out by the motion's background builder, so a window goes on playing meanwhile: the result says pending: true and leaves out the points and what is measured of them, and the trace is drawn once it is done (view.trace with background: false works it out in the command and returns them). view.trace {name} without point reads a trace asked for before: its full answer once it is there, the error that stopped it (a wrong point is an error at once; work let go of, by motion.bake {cancel: true}, view.motion {t: null} or an edit, says let go of), or pending: true; wait: true waits for it, in a window at most 2 s (it holds the window's engine thread: an agent asks again, or follows ui.get traces), elsewhere at most 600 s.",
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
    /// A part's moments made ahead (`motion.bake`), by the bits of their
    /// `t`: the state and its regeneration, as [`Sweep::at`] makes them,
    /// and about how many bytes the regeneration's bodies take.
    kept: BTreeMap<u64, (Arc<Document>, Arc<Regen>, usize)>,
    kept_bytes: usize,
    /// The collisions of moments found ahead (`motion.bake`), by the bits
    /// of their `t`: what [`collisions`] finds of that moment (the memo of
    /// answers only saves time, so they are the same).
    collided: BTreeMap<u64, Result<Vec<Collision>, String>>,
    /// The moments `motion.bake` made (by the bits of their `t`): a part's
    /// regenerated (kept or not, the budget full), an assembly's
    /// configurations built (kept until the budget lets them go).
    baked: BTreeSet<u64>,
    /// The frames of the last bake asked for (`motion.bake {frames}`): a
    /// player rounds its `t` to them, so playing asks for nothing new.
    bake_frames: Option<u64>,
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
    /// The solves of the frames of a bake (by the bits of their `t`), once
    /// each was placed exactly: the `t` it was placed at (a dead point's
    /// when it stopped there), the solve without bodies
    /// ([`AsmState::solved_of`]) and the dead point. Showing such a frame
    /// again places its bodies without solving (the same placement).
    solves: BTreeMap<u64, (f64, Arc<Placed>, Option<Stuck>)>,
    /// The `t` of the frames baked (`motion.bake`), whose solves are kept.
    bake_ts: BTreeSet<u64>,
    /// Per instance, the configurations it was seen to ask for, each with
    /// the first `t` it was seen at: a provisional moment shows an
    /// instance whose configuration is not built yet with the kept one
    /// nearest in `t` ([`Sweep::placed_ready`]). By configuration, so an
    /// instance whose configuration does not change has one entry however
    /// many moments were made (the router's 176 instances).
    seen: BTreeMap<String, BTreeMap<String, f64>>,
    /// A run of the grid in the background failed: no other is started
    /// for this sweep, and its moments are made where they are asked for.
    grid_stopped: bool,
    /// The configurations the frames of a bake ask for: kept when the
    /// configurations only grid points used are let go of.
    frame_keys: BTreeSet<String>,
    /// The grid point the background solves toward: the farthest a bake's
    /// frames reach, else the last provisional moment's.
    grid_target: Option<i64>,
    /// Frames of a bake in the background whose configurations are not
    /// asked for yet ([`Sweep::queue_baked`]).
    bake_queue: std::collections::VecDeque<f64>,
    /// The state the last moment made at once reached, and its `t`: a
    /// provisional moment near it is carried from there (a short step)
    /// rather than from a grid point far behind.
    last_carried: Option<(f64, Carried)>,
}

/// The frames of a bake in the background built by one run of the
/// builder ([`Sweep::queue_baked`]).
const BAKE_FRAMES_PER_RUN: usize = 8;

/// Most frames whose solves a motion keeps: a bake asks for at most 1001.
const MAX_SOLVES: usize = 4_096;

/// Most configurations a motion remembers per instance, for the
/// substitutes of provisional moments.
const MAX_SEEN: usize = 8_192;

/// Most grid points one background run solves before it hands them in.
const GRID_RUN: usize = 16;

/// A moment of an assembly made at once ([`Sweep::placed_ready`]).
pub(crate) struct Ready {
    pub placed: Arc<Placed>,
    pub stuck: Option<Stuck>,
    /// True when it is the moment [`Sweep::placed_at`] makes; false for a
    /// provisional one (an instance shown with another configuration of
    /// its own, or the solve carried from a grid point short of its own).
    pub exact: bool,
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
            kept: BTreeMap::new(),
            kept_bytes: 0,
            collided: BTreeMap::new(),
            baked: BTreeSet::new(),
            bake_frames: None,
        }
    }

    /// The sweep of `base`, a state of `open`'s document: an assembly's
    /// moments are solved with the files read for it, a part's
    /// regenerated with its cache.
    pub(crate) fn of(open: &mut OpenDocument, base: &Document) -> Self {
        if base.is_assembly() {
            let mut sweep = Self::new(base, RegenCache::default());
            let mut asm = open.asm.clone();
            asm.slim = true;
            sweep.assembly = Some(AsmSweep {
                path: open.path.clone(),
                asm,
                grid: BTreeMap::new(),
                solves: BTreeMap::new(),
                bake_ts: BTreeSet::new(),
                seen: BTreeMap::new(),
                grid_stopped: false,
                frame_keys: BTreeSet::new(),
                grid_target: None,
                bake_queue: std::collections::VecDeque::new(),
                last_carried: None,
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
        let mut solved = false;
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
            solved = true;
        }
        if solved {
            // The grid points solved now need their configurations no
            // more, unless a frame of a bake or the last one asks for them
            // (a moment on the grid).
            let mut keep = sweep.frame_keys.clone();
            let last = at_t(reference, t0 + k as f64 * MOTION_STEP)?;
            keep.extend(
                sweep
                    .asm
                    .instance_keys(&last, sweep.path.as_deref())
                    .into_iter()
                    .map(|(_, key)| key),
            );
            sweep.asm.drop_solved_only(&keep);
        }
        Ok(from)
    }

    /// An assembly's moment at `t`: placed from its grid point, with the
    /// dead point the motion stopped at on the way, if any (then the
    /// instances stay where it stopped).
    pub(crate) fn placed_at(&mut self, t: f64) -> Result<(Arc<Placed>, Option<Stuck>), String> {
        timeline(&self.reference)?;
        // Exact: everything it asks for is built here if it must be.
        if let Some(sweep) = self.assembly.as_mut() {
            sweep.asm.deferred.clear();
        }
        let bits = t.to_bits();
        // A frame of a bake solved before: its bodies placed again.
        if let Some(sweep) = self.assembly.as_mut()
            && let Some((at, solved, stuck)) = sweep.solves.get(&bits).cloned()
        {
            let doc = at_t(&self.reference, at)?;
            if let Some(placed) = sweep.asm.show_solved(&doc, &solved) {
                return Ok((Arc::new(placed), stuck));
            }
            sweep.solves.remove(&bits);
        }
        let k = self.grid_index(t)?;
        let from = self.asm_grid_point(k)?;
        let (at, placed, stuck) = {
            let sweep = self
                .assembly
                .as_mut()
                .ok_or("the motion is not an assembly's")?;
            if let Some(stuck) = &from.stuck {
                let doc = at_t(&self.reference, stuck.t)?;
                let placed =
                    sweep
                        .asm
                        .place_moment(&doc, sweep.path.as_deref(), Some(&from.carried));
                (stuck.t, placed, Some(stuck.clone()))
            } else {
                let doc = at_t(&self.reference, t)?;
                let placed =
                    sweep
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
                (t, placed, stuck)
            }
        };
        self.note_configs(t, &placed);
        if let (Some(sweep), Some(carried)) = (self.assembly.as_mut(), placed.carried.clone()) {
            sweep.last_carried = Some((t, carried));
        }
        if let Some(sweep) = self.assembly.as_mut()
            && sweep.bake_ts.contains(&bits)
            && sweep.solves.len() < MAX_SOLVES
            && let Some(solved) = AsmState::solved_of(&placed)
        {
            sweep
                .solves
                .insert(bits, (at, Arc::new(solved), stuck.clone()));
        }
        Ok((Arc::new(placed), stuck))
    }

    /// Remembers which configuration each instance of `placed` (a moment
    /// at `t`) asked for, for the substitutes of provisional moments.
    fn note_configs(&mut self, t: f64, placed: &Placed) {
        let pairs: Vec<(String, String)> = placed
            .instances
            .iter()
            .filter(|st| !placed.stale.contains(&st.name))
            .filter_map(|st| Some((st.name.clone(), st.config.clone()?)))
            .collect();
        self.note_pairs(t, pairs);
    }

    fn note_pairs(&mut self, t: f64, pairs: Vec<(String, String)>) {
        let Some(sweep) = self.assembly.as_mut() else {
            return;
        };
        for (name, key) in pairs {
            let seen = sweep.seen.entry(name).or_default();
            if seen.len() < MAX_SEEN {
                seen.entry(key).or_insert(t);
            }
        }
    }

    /// For each instance seen, the kept configuration of its own nearest
    /// in `t` to `t` (ties: the smaller `t`, then the key).
    fn substitutes(&self, t: f64) -> BTreeMap<String, String> {
        let Some(sweep) = self.assembly.as_ref() else {
            return BTreeMap::new();
        };
        sweep
            .seen
            .iter()
            .filter_map(|(name, seen)| {
                let best = seen
                    .iter()
                    .filter(|(key, _)| sweep.asm.has_config(key))
                    .min_by(|(_, a), (_, b)| {
                        (*a - t)
                            .abs()
                            .total_cmp(&(*b - t).abs())
                            .then(a.total_cmp(b))
                    })?;
                Some((name.clone(), best.0.clone()))
            })
            .collect()
    }

    /// An assembly's moment at `t` made at once (`view.motion {shapes:
    /// "ready"}`): the moment [`Self::placed_at`] makes when everything it
    /// needs is kept; else a provisional one, whose instances lacking their
    /// configuration show the kept one of their own nearest in `t` and
    /// whose solve is carried from the grid point nearest `t` that is
    /// solved, while `builder` builds what is missing. On the way it solves
    /// at most [`READY_GRID_SOLVES`] grid points whose configurations are
    /// kept; a provisional moment is never kept in the grid, so the grid
    /// stays exact (seek = play) and the moment settles to the exact one.
    /// Without a builder (the browser) it is [`Self::placed_at`].
    pub(crate) fn placed_ready(
        &mut self,
        t: f64,
        builder: &mut crate::motion_build::Builder,
    ) -> Result<Ready, String> {
        timeline(&self.reference)?;
        let exact = |(placed, stuck): (Arc<Placed>, Option<Stuck>)| Ready {
            placed,
            stuck,
            exact: true,
        };
        if !crate::motion_build::Builder::available()
            || self
                .assembly
                .as_ref()
                .is_some_and(|a| a.solves.contains_key(&t.to_bits()))
        {
            return self.placed_at(t).map(exact);
        }
        let k = self.grid_index(t)?;
        let t0 = self.t0;
        // Grid point 0, the assembly at rest, is made where it is asked
        // for: its configurations are those shown at rest, the
        // substitutes of last resort.
        if !self.asm_grid_solved(0) {
            self.asm_grid_point(0)?;
            let doc = at_t(&self.reference, t0)?;
            let pairs = {
                let sweep = self
                    .assembly
                    .as_mut()
                    .ok_or("the motion is not an assembly's")?;
                sweep.asm.instance_keys(&doc, sweep.path.as_deref())
            };
            self.note_pairs(t0, pairs);
        }
        let j = self.nearest_solved(k);
        if self.asm_grid_stuck(j) {
            // Past a dead point nothing is solved: placed as usual.
            return self.placed_at(t).map(exact);
        }
        let doc = at_t(&self.reference, t)?;
        // What the builder has already is neither worked out nor built
        // here again: for this placement only, given back whatever
        // happens in it.
        let in_flight = builder.in_flight().clone();
        self.with_deferred(in_flight, |sweep| {
            sweep.placed_ready_deferred(t, k, j, &doc, builder)
        })
    }

    /// Runs `work` with `keys` as the configurations the builder has
    /// ([`AsmState::deferred`]), which are empty again afterwards also when
    /// `work` fails or panics: a grid point solved later with them still
    /// set would skip their configurations and be kept so for good.
    fn with_deferred<R>(&mut self, keys: BTreeSet<String>, work: impl FnOnce(&mut Self) -> R) -> R {
        if let Some(sweep) = self.assembly.as_mut() {
            sweep.asm.deferred = keys;
        }
        let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(&mut *self)));
        if let Some(sweep) = self.assembly.as_mut() {
            sweep.asm.deferred.clear();
        }
        out.unwrap_or_else(|payload| std::panic::resume_unwind(payload))
    }

    /// The rest of [`Self::placed_ready`], within [`Self::with_deferred`].
    fn placed_ready_deferred(
        &mut self,
        t: f64,
        k: i64,
        j: i64,
        doc: &Document,
        builder: &mut crate::motion_build::Builder,
    ) -> Result<Ready, String> {
        let exact = |(placed, stuck): (Arc<Placed>, Option<Stuck>)| Ready {
            placed,
            stuck,
            exact: true,
        };
        let t0 = self.t0;
        let (own, subs) = self.asm_missing(std::slice::from_ref(doc))?;
        let waiting = self.asm_waiting(doc)?;
        if j == k && own.is_empty() && !waiting {
            return self.placed_at(t).map(exact);
        }
        // Asked for first: this moment's configurations (the grid's are
        // built by the run that solves it).
        builder.queue(own, true);
        // The grid is solved in the background, toward this moment unless
        // a bake reaches farther.
        if let Some(sweep) = self.assembly.as_mut()
            && sweep
                .grid_target
                .is_none_or(|g| g.signum() != k.signum() || g.abs() < k.abs())
        {
            sweep.grid_target = Some(k);
        }
        self.queue_grid(builder)?;
        if subs {
            // A subassembly's configuration is built where it is asked
            // for: placed as usual.
            return self.placed_at(t).map(exact);
        }
        let substitutes = self.substitutes(t);
        let grid_t = t0 + j as f64 * MOTION_STEP;
        let from = self
            .assembly
            .as_ref()
            .and_then(|a| match &a.last_carried {
                Some((at, carried)) if (at - t).abs() < (grid_t - t).abs() => Some(carried.clone()),
                _ => a.grid.get(&j).map(|g| g.carried.clone()),
            })
            .ok_or("the motion lost its grid")?;
        let sweep = self
            .assembly
            .as_mut()
            .ok_or("the motion is not an assembly's")?;
        sweep.asm.deferred = builder.in_flight().clone();
        let placed = sweep
            .asm
            .place_ready(doc, sweep.path.as_deref(), Some(&from), &substitutes);
        self.note_configs(t, &placed);
        if let (Some(sweep), Some(carried)) = (self.assembly.as_mut(), placed.carried.clone()) {
            sweep.last_carried = Some((t, carried));
        }
        Ok(Ready {
            placed: Arc::new(placed),
            stuck: None,
            exact: false,
        })
    }

    /// The grid point nearest `k` on the way from 0 that is solved (0,
    /// the assembly at rest, at worst).
    fn nearest_solved(&self, k: i64) -> i64 {
        let step = k.signum();
        let mut j = k;
        while j != 0 && !self.asm_grid_solved(j) {
            j -= step;
        }
        j
    }

    /// Starts a run of grid points solved in the background toward the
    /// sweep's grid target (one run at a time), from the nearest solved
    /// one, as far as their configurations are kept: on a copy of the
    /// motion's state, each solved as [`Self::asm_grid_point`] solves it
    /// (the same state, bit for bit), taken in by [`Self::take_built`].
    fn queue_grid(&mut self, builder: &mut crate::motion_build::Builder) -> Result<(), String> {
        if builder.grid_busy()
            || !crate::motion_build::Builder::available()
            || self.assembly.as_ref().is_none_or(|a| a.grid_stopped)
        {
            return Ok(());
        }
        let Some(target) = self.assembly.as_ref().and_then(|a| a.grid_target) else {
            return Ok(());
        };
        if !self.asm_grid_solved(0) {
            return Ok(());
        }
        let j = self.nearest_solved(target);
        if j == target || self.asm_grid_stuck(j) {
            return Ok(());
        }
        let step = target.signum();
        let t0 = self.t0;
        let reference = self.reference.clone();
        let sweep = self
            .assembly
            .as_ref()
            .ok_or("the motion is not an assembly's")?;
        let from = sweep
            .grid
            .get(&j)
            .map(|g| g.carried.clone())
            .ok_or("the motion lost its grid")?;
        let mut asm = sweep.asm.clone();
        asm.deferred.clear();
        let path = sweep.path.clone();
        builder.queue_grid(Box::new(move |let_go: crate::motion_build::Cancel| {
            asm.cancel = Some(std::sync::Arc::clone(&let_go));
            let before = asm.config_keys();
            let mut out = Vec::new();
            let mut from = from;
            let mut at = j;
            let mut failed = false;
            while at != target && out.len() < GRID_RUN && !let_go() {
                at += step;
                let t = t0 + at as f64 * MOTION_STEP;
                let Ok(doc) = at_t(&reference, t) else {
                    failed = true;
                    break;
                };
                let placed = asm.solve_moment(&doc, path.as_deref(), Some(&from));
                match placed.carried.clone() {
                    Some(carried) => {
                        out.push(crate::motion_build::GridStep {
                            k: at,
                            carried: carried.clone(),
                            stuck: None,
                        });
                        from = carried;
                    }
                    None => {
                        out.push(crate::motion_build::GridStep {
                            k: at,
                            carried: from.clone(),
                            stuck: Some(Stuck {
                                t: t - step as f64 * MOTION_STEP,
                                why: placed
                                    .solve_error
                                    .clone()
                                    .unwrap_or_else(|| "it does not solve".to_string()),
                            }),
                        });
                        break;
                    }
                }
            }
            let built = asm.configs_since(&before);
            crate::motion_build::Worked {
                // A run that solved nothing and was not let go of goes no
                // further than its last.
                failed: failed || (out.is_empty() && !let_go()),
                steps: out,
                built,
                seen: Vec::new(),
                ..crate::motion_build::Worked::default()
            }
        }));
        Ok(())
    }

    /// Queues the configurations of a few more frames of a bake in the
    /// background (`motion.bake {background: true}`), a few at a time so
    /// no call takes long: what each instance asks for at each frame is
    /// worked out here, the building is the builder's.
    fn queue_baked(&mut self, builder: &mut crate::motion_build::Builder) -> Result<(), String> {
        // Frames per run, and runs at once: the frames' configurations are
        // worked out and built on copies of the motion's state, off the
        // thread that asked.
        const PER_RUN: usize = BAKE_FRAMES_PER_RUN;
        const RUNS: usize = 1;
        while builder.frames_busy() < RUNS {
            let ts: Vec<f64> = match self.assembly.as_mut() {
                Some(sweep) => {
                    let n = sweep.bake_queue.len().min(PER_RUN);
                    sweep.bake_queue.drain(..n).collect()
                }
                None => return Ok(()),
            };
            if ts.is_empty() {
                return Ok(());
            }
            let reference = self.reference.clone();
            let sweep = self
                .assembly
                .as_ref()
                .ok_or("the motion is not an assembly's")?;
            let mut asm = sweep.asm.clone();
            asm.deferred.clear();
            let path = sweep.path.clone();
            let queued =
                builder.queue_frames(Box::new(move |let_go: crate::motion_build::Cancel| {
                    asm.cancel = Some(std::sync::Arc::clone(&let_go));
                    let before = asm.config_keys();
                    let mut seen = Vec::new();
                    let mut failed = false;
                    // A frame at a time, so that a run let go of stops at the
                    // next frame (the configurations are the same built
                    // together or one frame after the other).
                    for &t in &ts {
                        if let_go() {
                            break;
                        }
                        let Ok(doc) = at_t(&reference, t) else {
                            failed = true;
                            continue;
                        };
                        seen.push((t, asm.instance_keys(&doc, path.as_deref())));
                        asm.prebuild(std::slice::from_ref(&doc), path.as_deref(), usize::MAX);
                    }
                    crate::motion_build::Worked {
                        steps: Vec::new(),
                        built: asm.configs_since(&before),
                        seen,
                        failed,
                        ..crate::motion_build::Worked::default()
                    }
                }));
            if !queued {
                return Ok(());
            }
        }
        Ok(())
    }

    /// Grid points between the nearest solved one and the target the
    /// background solves toward: what a run of the grid has left at most.
    pub(crate) fn grid_left(&self) -> usize {
        let Some(target) = self.assembly.as_ref().and_then(|a| a.grid_target) else {
            return 0;
        };
        let solved = self.nearest_solved(target);
        usize::try_from(target.abs_diff(solved)).unwrap_or(usize::MAX)
    }

    /// Frames of a bake in the background not looked at yet.
    pub(crate) fn bake_queued(&self) -> usize {
        self.assembly.as_ref().map_or(0, |a| a.bake_queue.len())
    }

    /// True when the assembly's grid point `k` is solved and stopped at a
    /// dead point.
    fn asm_grid_stuck(&self, k: i64) -> bool {
        self.assembly
            .as_ref()
            .and_then(|a| a.grid.get(&k))
            .is_some_and(|g| g.stuck.is_some())
    }

    /// Pins the configurations `docs` ask for (a batch of a bake), in
    /// place of what was pinned; `&[]` lets go of the pins.
    fn asm_pin(&mut self, docs: &[Document]) -> Result<(), String> {
        let sweep = self
            .assembly
            .as_mut()
            .ok_or("the motion is not an assembly's")?;
        let mut keys = BTreeSet::new();
        for doc in docs {
            keys.extend(
                sweep
                    .asm
                    .instance_keys(doc, sweep.path.as_deref())
                    .into_iter()
                    .map(|(_, key)| key),
            );
        }
        sweep.asm.pin(keys);
        Ok(())
    }

    /// True when a configuration `doc` asks for is being built by the
    /// builder ([`AsmState::deferred`]) and not kept yet.
    fn asm_waiting(&mut self, doc: &Document) -> Result<bool, String> {
        let sweep = self
            .assembly
            .as_mut()
            .ok_or("the motion is not an assembly's")?;
        let keys = sweep.asm.instance_keys(doc, sweep.path.as_deref());
        Ok(keys
            .iter()
            .any(|(_, key)| sweep.asm.deferred.contains(key) && !sweep.asm.has_config(key)))
    }

    /// [`AsmState::missing_parts`] in the sweep's own state.
    fn asm_missing(
        &mut self,
        docs: &[Document],
    ) -> Result<(Vec<crate::motion_build::PartJob>, bool), String> {
        let sweep = self
            .assembly
            .as_mut()
            .ok_or("the motion is not an assembly's")?;
        Ok(sweep.asm.missing_parts(docs, sweep.path.as_deref()))
    }

    /// Takes in what `builder` built for this motion. True when anything
    /// was taken.
    pub(crate) fn take_built(&mut self, builder: &mut crate::motion_build::Builder) -> bool {
        let done = builder.take();
        let steps = builder.take_grid();
        let grid_failed = builder.take_grid_failed();
        for (t, pairs) in builder.take_seen() {
            if let Some(sweep) = self.assembly.as_mut() {
                sweep
                    .frame_keys
                    .extend(pairs.iter().map(|(_, key)| key.clone()));
            }
            self.note_pairs(t, pairs);
        }
        let Some(sweep) = self.assembly.as_mut() else {
            return false;
        };
        if grid_failed {
            // Not asked for again every poll: the moments it was for are
            // made where they are asked for (exactly, with the error).
            sweep.grid_stopped = true;
        }
        let taken = !done.is_empty() || !steps.is_empty() || grid_failed;
        if !done.is_empty() {
            sweep.asm.take_built(done);
        }
        let mut solved = false;
        for step in steps {
            let before = step.k - step.k.signum();
            if !sweep.grid.contains_key(&step.k) && sweep.grid.contains_key(&before) {
                sweep.grid.insert(
                    step.k,
                    Arc::new(GridState {
                        carried: step.carried,
                        stuck: step.stuck,
                    }),
                );
                solved = true;
            }
        }
        if solved || taken {
            // As after grid points solved here: their configurations go,
            // but those the frames of a bake ask for.
            let keep = sweep.frame_keys.clone();
            sweep.asm.drop_solved_only(&keep);
        }
        // Whatever came in, the grid goes on, and the frames of a bake.
        let _ = self.queue_grid(builder);
        let _ = self.queue_baked(builder);
        taken
    }

    /// Queues every configuration a play at `ts` asks for (the grid points
    /// the frames are carried through, outward from the reference, then
    /// the frames') with `builder`, behind what is asked for at once
    /// (`motion.bake {background: true}`). Returns how many were queued
    /// now; 0 without a builder or for a part.
    pub(crate) fn bake_background(
        &mut self,
        ts: &[f64],
        builder: &mut crate::motion_build::Builder,
    ) -> Result<usize, String> {
        timeline(&self.reference)?;
        if self.assembly.is_none() {
            return Ok(0);
        }
        let mut farthest = 0i64;
        for &t in ts {
            let k = self.grid_index(t)?;
            if k.abs() > farthest.abs() {
                farthest = k;
            }
        }
        // The rest, grid point 0, is solved here (its configurations are
        // those at rest); the grid beyond it by runs in the background,
        // which build what it asks for, and the frames' configurations a
        // few at a time as the session polls.
        self.asm_grid_point(0)?;
        if let Some(sweep) = self.assembly.as_mut() {
            sweep.bake_ts.extend(ts.iter().map(|t| t.to_bits()));
            sweep.grid_target = Some(farthest);
            sweep.bake_queue.extend(ts.iter().copied());
        }
        self.queue_grid(builder)?;
        self.queue_baked(builder)?;
        Ok(ts.len())
    }

    /// Notes what each instance asks for at the frames `ts` of a bake:
    /// kept as substitutes and as configurations the grid does not let go
    /// of.
    fn note_frames(&mut self, ts: &[f64]) -> Result<(), String> {
        for &t in ts {
            let doc = at_t(&self.reference, t)?;
            let pairs = {
                let sweep = self
                    .assembly
                    .as_mut()
                    .ok_or("the motion is not an assembly's")?;
                let pairs = sweep.asm.instance_keys(&doc, sweep.path.as_deref());
                sweep
                    .frame_keys
                    .extend(pairs.iter().map(|(_, key)| key.clone()));
                pairs
            };
            self.note_pairs(t, pairs);
        }
        Ok(())
    }

    /// The frames of the last bake, if any.
    pub(crate) fn bake_frames(&self) -> Option<u64> {
        self.bake_frames
    }

    /// Says that a bake at `frames` frames was asked for.
    pub(crate) fn set_bake_frames(&mut self, frames: u64) {
        self.bake_frames = Some(frames);
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
        if let Some((doc, regen, _)) = self.kept.get(&t.to_bits()) {
            return Ok((Arc::clone(doc), Arc::clone(regen)));
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

/// How many states (grid points or frames) a bake builds the
/// configurations of side by side at a time: their configurations are
/// held together, so this bounds what a bake holds beyond the budget.
const BAKE_BATCH: usize = 32;

/// What [`Sweep::bake`] made ahead.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Baked {
    /// For an assembly: the configurations its moments (and the grid
    /// points they are carried through) ask for.
    pub configs: Option<crate::asm::Prebuilt>,
    /// For a part: the moments made now and kept.
    pub moments: usize,
    /// Moments made but not kept, the budget being full (a part), or
    /// configurations the frames ask for that are not kept (an assembly).
    pub over_budget: usize,
    /// Frames whose collisions were found now.
    pub collisions: usize,
    /// Configurations (assembly) or moments (part) left for a next call,
    /// beyond the limit; 0 when the motion is baked.
    pub remaining: usize,
    /// Configurations (assembly) or moments (part) kept, and about how many
    /// bytes they take.
    pub kept: usize,
    pub bytes: usize,
}

impl Sweep {
    /// Makes the moments `ts` ahead, so that showing them later only
    /// places (an assembly) or looks them up (a part): for an assembly
    /// every configuration they and their grid points ask for, built side
    /// by side ([`AsmState::prebuild`], at most `limit` this call), then
    /// the grid solved up to each; for a part the moments themselves,
    /// regenerated side by side from copies of the sweep's cache (at most
    /// `limit` this call), kept within the budget of configurations. With
    /// `collisions`, once nothing is left, each frame's collisions too. A
    /// moment made ahead is the moment [`Sweep::at`] makes: nothing here
    /// changes what is shown, only when it is made.
    pub(crate) fn bake(
        &mut self,
        ts: &[f64],
        limit: usize,
        collisions_too: bool,
        memo: &mut PairMemo,
        budget: usize,
    ) -> Result<Baked, String> {
        timeline(&self.reference)?;
        if let Some(sweep) = self.assembly.as_mut() {
            sweep.bake_ts.extend(ts.iter().map(|t| t.to_bits()));
        }
        let mut baked = if self.assembly.is_some() {
            self.bake_assembly(ts, limit)?
        } else {
            self.bake_part(ts, limit, budget)?
        };
        if collisions_too && baked.remaining == 0 {
            for &t in ts {
                if self.collided.contains_key(&t.to_bits()) {
                    continue;
                }
                let (doc, regen) = self.at(t)?;
                let found = collisions(&doc, &regen, memo);
                self.collided.insert(t.to_bits(), found);
                baked.collisions = baked.collisions.saturating_add(1);
            }
        }
        Ok(baked)
    }

    fn bake_assembly(&mut self, ts: &[f64], limit: usize) -> Result<Baked, String> {
        // What each instance asks for at each frame: the substitutes of
        // provisional moments, and what the grid lets go of keeps.
        self.note_frames(ts)?;
        let t0 = self.t0;
        let mut left = limit;
        let mut configs = crate::asm::Prebuilt::default();
        // 1. The grid, outward from the reference as the frames reach it,
        // a batch at a time: its configurations built side by side, then
        // its points solved one after the other. A grid point's
        // configurations are needed for its solve alone, so they are the
        // first to go when the budget is full (`AsmState::evict`).
        let mut seen: BTreeSet<i64> = BTreeSet::new();
        let mut grid: Vec<i64> = Vec::new();
        for &t in ts {
            let k = self.grid_index(t)?;
            let step = k.signum();
            let mut at = 0i64;
            loop {
                if seen.insert(at) && !self.asm_grid_solved(at) {
                    grid.push(at);
                }
                if at == k {
                    break;
                }
                at += step;
            }
        }
        let mut unsolved = grid.len();
        for batch in grid.chunks(BAKE_BATCH) {
            if left == 0 {
                break;
            }
            let docs = batch
                .iter()
                // As `asm_grid_point` writes a grid point's t.
                .map(|&k| at_t(&self.reference, t0 + k as f64 * MOTION_STEP))
                .collect::<Result<Vec<_>, _>>()?;
            self.asm_pin(&docs)?;
            let pre = self.asm_prebuild(&docs, left)?;
            configs.add(&pre);
            left = left.saturating_sub(pre.built);
            if pre.remaining > 0 {
                break;
            }
            for &k in batch {
                self.asm_grid_point(k)?;
            }
            self.asm_pin(&[])?;
            unsolved = unsolved.saturating_sub(batch.len());
        }
        // 2. The frames' own configurations, kept for the play.
        let mut frames_left: usize = 0;
        let mut frames: Vec<f64> = Vec::new();
        let mut kept: BTreeSet<u64> = BTreeSet::new();
        for &t in ts {
            if !self.baked.contains(&t.to_bits()) && kept.insert(t.to_bits()) {
                frames.push(t);
            }
        }
        if unsolved == 0 {
            for batch in frames.chunks(BAKE_BATCH) {
                if left == 0 {
                    frames_left = frames_left.saturating_add(batch.len());
                    continue;
                }
                let docs = batch
                    .iter()
                    .map(|&t| at_t(&self.reference, t))
                    .collect::<Result<Vec<_>, _>>()?;
                self.asm_pin(&docs)?;
                let pre = self.asm_prebuild(&docs, left)?;
                configs.add(&pre);
                left = left.saturating_sub(pre.built);
                if pre.remaining > 0 {
                    frames_left = frames_left.saturating_add(batch.len());
                    continue;
                }
                self.asm_pin(&[])?;
                self.baked.extend(batch.iter().map(|t| t.to_bits()));
            }
        } else {
            frames_left = frames.len();
        }
        // What the frames ask for that is not kept now (the budget full).
        if let Some(sweep) = self.assembly.as_mut() {
            sweep.asm.settle_budget();
        }
        let docs = ts
            .iter()
            .map(|&t| at_t(&self.reference, t))
            .collect::<Result<Vec<_>, _>>()?;
        let now = self.asm_prebuild(&docs, 0)?;
        configs.wanted = now.wanted;
        let (kept, bytes) = self
            .assembly
            .as_ref()
            .map_or((0, 0), |a| a.asm.configs_held());
        Ok(Baked {
            remaining: unsolved.saturating_add(frames_left),
            over_budget: if unsolved == 0 && frames_left == 0 {
                now.remaining
            } else {
                0
            },
            configs: Some(configs),
            kept,
            bytes,
            ..Baked::default()
        })
    }

    /// True when the assembly's grid point `k` is solved.
    fn asm_grid_solved(&self, k: i64) -> bool {
        self.assembly
            .as_ref()
            .is_some_and(|a| a.grid.contains_key(&k))
    }

    /// [`AsmState::prebuild`] in the sweep's own state.
    fn asm_prebuild(
        &mut self,
        docs: &[Document],
        limit: usize,
    ) -> Result<crate::asm::Prebuilt, String> {
        let sweep = self
            .assembly
            .as_mut()
            .ok_or("the motion is not an assembly's")?;
        Ok(sweep.asm.prebuild(docs, sweep.path.as_deref(), limit))
    }

    fn bake_part(&mut self, ts: &[f64], limit: usize, budget: usize) -> Result<Baked, String> {
        let mut seen: BTreeSet<u64> = BTreeSet::new();
        let mut todo: Vec<f64> = ts
            .iter()
            .copied()
            .filter(|t| !self.baked.contains(&t.to_bits()) && seen.insert(t.to_bits()))
            .collect();
        let remaining = todo.len().saturating_sub(limit);
        todo.truncate(limit);
        let mut baked = Baked {
            remaining,
            ..Baked::default()
        };
        // A batch at a time, so what is held at once is a batch beyond the
        // budget, never every moment; once the budget is full the moments
        // left are not made (they would not be kept: made when shown).
        let mut full = false;
        for batch in todo.chunks(BAKE_BATCH) {
            if full {
                for t in batch {
                    self.baked.insert(t.to_bits());
                }
                baked.over_budget = baked.over_budget.saturating_add(batch.len());
                continue;
            }
            // The sketches are carried along the grid one after the other;
            // the regenerations then go side by side.
            let mut docs = Vec::with_capacity(batch.len());
            for &t in batch {
                docs.push(self.settled(t)?);
            }
            let cache = &self.cache;
            let indices: Vec<usize> = (0..docs.len()).collect();
            let made = crate::interference_cmds::in_order(&indices, |i| {
                let doc = docs.get(i)?;
                let mut cache = cache.clone();
                Some(craft3d_regen::regenerate_cached(doc, &mut cache))
            });
            for ((t, doc), regen) in batch.iter().zip(docs).zip(made) {
                self.baked.insert(t.to_bits());
                let Some(regen) = regen else { continue };
                let bytes = crate::asm::regen_bytes(&regen);
                if full || self.kept_bytes.saturating_add(bytes) > budget {
                    full = true;
                    baked.over_budget = baked.over_budget.saturating_add(1);
                    continue;
                }
                self.kept_bytes = self.kept_bytes.saturating_add(bytes);
                self.kept
                    .insert(t.to_bits(), (Arc::new(doc), Arc::new(regen), bytes));
                baked.moments = baked.moments.saturating_add(1);
            }
        }
        baked.kept = self.kept.len();
        baked.bytes = self.kept_bytes;
        Ok(baked)
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
        // The common volume alone: how two bodies that do not overlap
        // touch is not asked (`common_volume`, not `interference`).
        Some(match common_volume(a, b, &tolerance) {
            Ok(Some(common)) => craft3d_topo::mass_properties(&common.body)
                .map(|mass| mass.volume)
                .map_err(|e| format!("the common volume could not be measured: {e}")),
            Ok(None) => Ok(0.0),
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
    /// False for a provisional moment (`view.motion {shapes: "ready"}`):
    /// made again, exact, once what it lacks is built.
    exact: bool,
    /// The builder had work when a provisional moment was made: once it
    /// has none, the moment is made again (it may be exact now). A
    /// provisional moment made while the builder was idle is made again
    /// only when something is taken in, so a moment that cannot settle in
    /// the background is not made again every poll.
    made_busy: bool,
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
    /// The command running asked for its moment at once with what is
    /// kept, the rest built in the background (`view.motion {shapes:
    /// "ready"}`): for that command only.
    ready: bool,
    /// The window's own viewport is drawing ([`Session::set_motion_live`]):
    /// it may show a provisional moment, and settles it by polling. Every
    /// other picture (a command's: `view.render`, `ui.screenshot`, a
    /// frame-exact recording, an export, an assertion) is of the exact
    /// moment.
    pub(crate) live: bool,
    /// What builds the configurations of the sweep in the background.
    builder: crate::motion_build::Builder,
    /// The traces asked for in the background (`view.trace` in a
    /// window), by name, until they are delivered: pending, or why they
    /// could not be made.
    trace_jobs: BTreeMap<String, TraceJob>,
    /// Counts the traces asked for and the clears: a trace worked out in
    /// the background is taken only when it is the one last asked for its
    /// name (an older one finishing later, or one cleared, is let go of).
    trace_serial: u64,
}

/// A trace asked for in the background ([`Motion::trace_jobs`]).
#[derive(Clone)]
pub(crate) struct TraceJob {
    serial: u64,
    /// The document state and files it was asked for in: an edit lets go
    /// of it.
    base: Arc<Document>,
    generation: u64,
    /// Why it could not be made; `None` while pending.
    error: Option<String>,
}

impl TraceJob {
    fn pending(&self) -> bool {
        self.error.is_none()
    }
}

/// A trace as `ui.get` and a viewer list it ([`Session::trace_states`]).
#[derive(Clone, Debug, PartialEq)]
pub struct TraceState {
    pub name: String,
    /// Being worked out in the background.
    pub pending: bool,
    /// Why a trace asked for in the background could not be made.
    pub error: Option<String>,
    /// Of a trace that is there: its length, in the document's unit, and
    /// whether it ends where it began.
    pub length: Option<f64>,
    pub closed: Option<bool>,
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
            && (state.exact || self.motion.ready || self.motion.live)
        {
            return Some(state.clone());
        }
        match self.make_motion_state(&base, t) {
            Ok(state) => {
                self.motion.state = Some(state.clone());
                Some(state)
            }
            Err(why) => {
                self.reset_builder(&format!("the motion could not be shown: {why}"));
                self.motion = Motion {
                    traces: std::mem::take(&mut self.motion.traces),
                    builder: std::mem::take(&mut self.motion.builder),
                    live: self.motion.live,
                    trace_jobs: std::mem::take(&mut self.motion.trace_jobs),
                    trace_serial: self.motion.trace_serial,
                    ..Motion::default()
                };
                None
            }
        }
    }

    /// The `t` the view shows, when it shows the motion.
    pub(crate) fn motion_t(&self) -> Option<f64> {
        self.motion.t
    }

    /// Whether the moments shown are drawn with their collisions.
    pub(crate) fn motion_collisions(&self) -> bool {
        self.motion.collisions
    }

    /// Shows the moment `t` (with its collisions when asked), or the
    /// document as it is, as `view.motion {t, collisions}` does: what a
    /// session that makes the view for another one is told
    /// (`view_elsewhere.rs`).
    pub(crate) fn set_motion_view(&mut self, t: Option<f64>, collisions: bool) {
        if t.is_none() && self.motion.t.is_some() {
            self.motion.state = None;
            self.motion.sweep = None;
            self.reset_builder("the motion was let go of (view.motion {t: null})");
            self.motion.memo = PairMemo::default();
        }
        self.motion.t = t;
        self.motion.collisions = collisions;
    }

    /// Lets go of the motion's sweep and its state (what was made ahead
    /// with it too); the next moment shown makes them again.
    pub(crate) fn motion_forget_sweep(&mut self) {
        self.motion.sweep = None;
        self.motion.state = None;
        self.motion.memo = PairMemo::default();
        self.reset_builder("the motion's states were let go of");
    }

    /// Moves the builder to a new epoch ([`crate::motion_build::Builder::
    /// reset`]): what it was doing is let go of, and a trace it was working
    /// out fails with `why` (`view.trace {name}` says so), never silently.
    pub(crate) fn reset_builder(&mut self, why: &str) {
        self.motion.builder.reset();
        for job in self.motion.trace_jobs.values_mut() {
            if job.pending() {
                job.error = Some(format!("let go of: {why}"));
            }
        }
    }

    /// Lets go of the traces being worked out for another state of the
    /// document than the one there now (an edit, a reload): they fail, and
    /// the builder stops working on them.
    pub(crate) fn check_trace_base(&mut self) {
        let current = Arc::clone(self.history.current());
        let generation = self.asm.generation;
        let stale = self.motion.trace_jobs.values().any(|job| {
            job.pending() && (!Arc::ptr_eq(&job.base, &current) || job.generation != generation)
        });
        if stale {
            self.reset_builder("the document changed before it was done");
        }
        self.motion.trace_jobs.retain(|_, job| {
            job.pending() || (Arc::ptr_eq(&job.base, &current) && job.generation == generation)
        });
    }

    /// How much the background has left: configurations queued or being
    /// built, frames of a bake not asked for yet and a run of the grid.
    pub(crate) fn motion_building(&self) -> usize {
        let queued = self
            .motion
            .sweep
            .as_ref()
            .map_or(0, |(_, _, sweep)| sweep.bake_queued());
        self.motion
            .builder
            .pending()
            .saturating_add(queued)
            .saturating_add(usize::from(self.motion.builder.grid_busy()))
            .saturating_add(self.motion.builder.frames_busy())
            .saturating_add(self.motion.builder.traces_busy())
    }

    /// [`Self::motion_building`] counted in steps of about equal work, for
    /// a progress bar: a run of the grid as the grid points it has left, a
    /// run of a bake's frames as its frames. Zero exactly when
    /// `motion_building` is.
    pub(crate) fn motion_work_left(&self) -> usize {
        let builder = &self.motion.builder;
        let sweep = self.motion.sweep.as_ref().map(|(_, _, sweep)| sweep);
        let grid = if builder.grid_busy() {
            sweep.map_or(0, Sweep::grid_left).max(1)
        } else {
            0
        };
        builder
            .pending()
            .saturating_add(sweep.map_or(0, Sweep::bake_queued))
            .saturating_add(grid)
            .saturating_add(builder.frames_busy().saturating_mul(BAKE_FRAMES_PER_RUN))
            .saturating_add(builder.traces_busy())
    }

    /// Drops what the background builder has queued (`motion.bake
    /// {cancel: true}`); returns how much was left
    /// ([`Self::motion_building`]: configurations, runs, frames of a bake
    /// and traces being worked out, which fail, `let go of`).
    pub(crate) fn motion_cancel_background(&mut self) -> usize {
        let dropped = self.motion_building();
        self.reset_builder("the background work was cancelled (motion.bake {cancel: true})");
        if let Some((_, _, sweep)) = self.motion.sweep.as_mut()
            && let Some(asm) = sweep.assembly.as_mut()
        {
            asm.bake_queue.clear();
            asm.grid_target = None;
        }
        dropped
    }

    /// The background builder of the motion's configurations, and its
    /// sweep over `base`.
    pub(crate) fn motion_sweep_and_builder(
        &mut self,
        base: &Arc<Document>,
    ) -> Result<(&mut Sweep, &mut crate::motion_build::Builder), String> {
        self.motion_sweep(base)?;
        let Motion { sweep, builder, .. } = &mut self.motion;
        let (_, _, sweep) = sweep.as_mut().ok_or("the motion lost its states")?;
        Ok((sweep, builder))
    }

    /// Takes in the configurations built in the background for the
    /// motion shown. True when the moment shown may change: something was
    /// taken, or the moment is provisional and can go further now (its
    /// grid is solved a little at a time). A provisional moment is then
    /// made again, at once, the next time it is shown.
    pub(crate) fn poll_motion(&mut self) -> bool {
        // Traces worked out in the background (`view.trace`): drawn once
        // they are here, if they are of the document as it is.
        let traced = self.take_traces();
        let Motion {
            sweep,
            builder,
            state,
            ..
        } = &mut self.motion;
        let Some((_, _, sweep)) = sweep.as_mut() else {
            return traced;
        };
        let taken = sweep.take_built(builder);
        let idle = builder.pending() == 0 && !builder.grid_busy();
        let again = state
            .as_ref()
            .is_some_and(|s| !s.exact && (taken || (idle && s.made_busy)));
        if again {
            *state = None;
        }
        again || traced
    }

    /// The answers about pairs of bodies kept from the last state's
    /// collisions.
    pub(crate) fn motion_memo(&mut self) -> &mut PairMemo {
        &mut self.motion.memo
    }

    /// The sweep the motion over `base` (the state the view shows below
    /// the motion) is made with: the one kept when it is of `base` and of
    /// the files as they are, a new one otherwise.
    pub(crate) fn motion_sweep(&mut self, base: &Arc<Document>) -> Result<&mut Sweep, String> {
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
            // What was being built was for another base.
            self.reset_builder("the motion was made again for another state of the document");
        }
        let (_, _, sweep) = self
            .motion
            .sweep
            .as_mut()
            .ok_or("the motion lost its states")?;
        Ok(sweep)
    }

    fn make_motion_state(&mut self, base: &Arc<Document>, t: f64) -> Result<State, String> {
        timeline(base)?;
        let base_regen = self.scrubbed_regen();
        let generation = self.asm.generation;
        let asked = self.motion.collisions;
        let ready = self.motion.ready || self.motion.live;
        let (sweep, builder) = self.motion_sweep_and_builder(base)?;
        let baked = asked
            .then(|| sweep.collided.get(&t.to_bits()).cloned())
            .flatten();
        let (plain, regen, stuck, placed, exact) = if sweep.is_assembly() {
            let made = if ready {
                sweep.placed_ready(t, builder)?
            } else {
                let (placed, stuck) = sweep.placed_at(t)?;
                Ready {
                    placed,
                    stuck,
                    exact: true,
                }
            };
            (
                Arc::clone(&made.placed.doc),
                Arc::clone(&made.placed.regen),
                made.stuck,
                Some(made.placed),
                made.exact,
            )
        } else {
            let (plain, regen) = sweep.at(t)?;
            (plain, regen, None, None, true)
        };
        // An assembly's moment places its parts by transforms: nothing is
        // regenerated.
        let rebuilt = if sweep.is_assembly() {
            Vec::new()
        } else {
            rebuilt(&base_regen, &regen)
        };
        let collisions = match baked {
            Some(found) => Some(found),
            None => asked.then(|| collisions(&plain, &regen, &mut self.motion.memo)),
        };
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
            exact,
            made_busy: !exact
                && (self.motion.builder.pending() > 0 || self.motion.builder.grid_busy()),
        })
    }

    /// The state of the document the view shows: the document itself,
    /// the state it is scrubbed to (`view.scrub`), or that at the `t` of
    /// its motion (`view.motion`).
    pub(crate) fn shown(&mut self) -> Arc<Document> {
        // Made elsewhere: the stand-in its scenes are kept under.
        if let Some(stand_in) = self.remote_shown() {
            return stand_in;
        }
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
        if self.tried_without_view() {
            return None;
        }
        if let Some(placed) = self.motion_state().and_then(|state| state.placed) {
            return Some(placed);
        }
        self.placed()
    }

    /// The regeneration of [`Self::shown`].
    pub(crate) fn shown_regen(&mut self) -> Arc<Regen> {
        if self.tried_without_view() {
            return Arc::new(Regen::default());
        }
        if let Some(state) = self.motion_state() {
            return state.regen;
        }
        if let Some(placed) = self.placed() {
            return Arc::clone(&placed.regen);
        }
        self.scrubbed_regen()
    }

    /// A command tried without the bodies (`commands_elsewhere.rs`) asks
    /// for the view, which is made elsewhere: it runs there instead
    /// (noted), and is handed nothing here. False everywhere else: a
    /// command run here makes the view here, as it always did.
    fn tried_without_view(&mut self) -> bool {
        if self.forbid_regen.is_some() && self.remote_wanted() {
            self.forbid_regen = Some(true);
            return true;
        }
        false
    }

    /// Takes in the traces worked out in the background; true when one
    /// was added or failed. Only the one last asked for its name is taken
    /// ([`Motion::trace_serial`]), and only for the document as it is.
    fn take_traces(&mut self) -> bool {
        self.check_trace_base();
        let done = self.motion.builder.take_traces();
        let mut changed = false;
        for done in done {
            let Some(job) = self.motion.trace_jobs.get_mut(&done.name) else {
                continue;
            };
            if job.serial != done.serial || !job.pending() {
                continue;
            }
            changed = true;
            match done.result {
                Ok(trace) => {
                    let pending = self
                        .motion
                        .trace_jobs
                        .values()
                        .filter(|j| j.pending())
                        .count()
                        .saturating_sub(1);
                    let traces = self.traces();
                    traces.retain(|t| t.name != trace.name);
                    if traces.len().saturating_add(pending) < MAX_TRACES {
                        traces.push(trace);
                        self.motion.trace_jobs.remove(&done.name);
                    } else if let Some(job) = self.motion.trace_jobs.get_mut(&done.name) {
                        job.error = Some(format!("{MAX_TRACES} traces are kept already"));
                    }
                }
                Err(why) => job.error = Some(why),
            }
        }
        changed
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
    /// False while the moment shown is provisional (`view.motion {shapes:
    /// "ready"}`): an instance shown with another configuration of its own
    /// while its own is built, or the solve carried from short of `t`.
    /// [`Session::poll_motion`] says when it may have changed.
    pub exact: bool,
    /// The instances shown with another configuration of their own.
    pub stale: Vec<String>,
    /// How many configurations are being built in the background.
    pub building: usize,
    /// The frames of the last bake (`motion.bake {frames}`): a player that
    /// rounds its `t` to `k / frames` asks for nothing new.
    pub baked: Option<u64>,
    /// What `building` counts, in steps of about equal work (a run of the
    /// grid as its grid points left, a run of a bake's frames as its
    /// frames): for a progress bar. Zero exactly when `building` is.
    pub work_left: usize,
    /// The moment `t` is being made elsewhere (the page's worker,
    /// `view_elsewhere.rs`) and is not drawn yet: a player waits for it
    /// before it asks for the next.
    pub pending: bool,
    /// Why the moment asked for could not be made elsewhere: the motion
    /// was let go of, as `view.motion` fails where it is made here.
    pub error: Option<String>,
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
        if let Some(view) = self.remote_motion() {
            return view;
        }
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
        let building = open.motion_building();
        let work_left = open.motion_work_left();
        let baked = open
            .motion
            .sweep
            .as_ref()
            .and_then(|(_, _, sweep)| sweep.bake_frames());
        Some(MotionView {
            t: state.t,
            collisions,
            rebuilt: state.rebuilt.len(),
            dead_point: state.stuck.as_ref().map(|stuck| stuck.t),
            exact: state.exact,
            stale: state
                .placed
                .as_ref()
                .map(|p| p.stale.clone())
                .unwrap_or_default(),
            building,
            baked,
            work_left,
            pending: false,
            error: None,
        })
    }

    /// [`Self::motion`] while the view is made elsewhere: what the other
    /// side said of the moment shown (pending until it did), and the bake
    /// it makes. `None` when the view is made here.
    fn remote_motion(&mut self) -> Option<Option<MotionView>> {
        let facts = self.remote_facts()?;
        let open = self.documents.get_mut(self.active)?;
        let Some(t) = open.motion_t() else {
            return Some(None);
        };
        let pending = open.remote_view().is_none_or(|view| view.facts.is_none());
        let bake = open.bake_left();
        let left = bake.as_ref().map_or(0, |b| b.left());
        // Baked there: its frames, as `view.motion` there says once a
        // moment is made after it.
        let baked_there = bake.filter(|b| {
            b.answer
                .as_ref()
                .is_some_and(|a| a.get("done") == Some(&Value::Bool(true)))
        });
        let motion = facts.as_ref().and_then(|f| f.motion.clone());
        let error = facts.as_ref().and_then(|f| f.error.clone());
        let count = |key: &str| {
            motion
                .as_ref()
                .and_then(|m| m.get(key))
                .and_then(Value::as_array)
                .map_or(0, Vec::len)
        };
        let collisions = motion
            .as_ref()
            .and_then(|m| m.get("collisions"))
            .and_then(Value::as_array)
            .map(|found| {
                found
                    .iter()
                    .filter(|c| c.get("volume").is_some())
                    .filter_map(|c| {
                        Some((
                            c.get("aName")?.as_str()?.to_string(),
                            c.get("bName")?.as_str()?.to_string(),
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let stale = motion
            .as_ref()
            .and_then(|m| m.get("stale"))
            .and_then(Value::as_array)
            .map(|s| {
                s.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        Some(Some(MotionView {
            t,
            collisions,
            rebuilt: count("rebuilt"),
            dead_point: motion
                .as_ref()
                .and_then(|m| m.pointer("/deadPoint/t"))
                .and_then(Value::as_f64),
            exact: motion
                .as_ref()
                .and_then(|m| m.get("exact"))
                .and_then(Value::as_bool)
                .unwrap_or(true),
            stale,
            building: left,
            baked: baked_there.map(|b| b.frames).or_else(|| {
                motion
                    .as_ref()
                    .and_then(|m| m.get("baked"))
                    .and_then(Value::as_u64)
            }),
            work_left: left,
            pending,
            error: if pending { None } else { error },
        }))
    }

    /// Takes in what was built in the background for the motion of the
    /// active document (`view.motion {shapes: "ready"}`): true when the
    /// moment shown may have changed (it was provisional), so a viewer
    /// shows it again ([`Self::motion`], the scene). A viewer that shows a
    /// provisional moment calls this every frame or so.
    pub fn poll_motion(&mut self) -> bool {
        let Some(open) = self.documents.get_mut(self.active) else {
            return false;
        };
        // Made elsewhere: what came since (a moment, a bake's piece).
        if open.remote_wanted() {
            return open.remote_fresh();
        }
        open.poll_motion()
    }

    /// True while a trace of the active document is worked out in the
    /// background (`view.trace` in a window); a viewer polls
    /// ([`Self::poll_motion`]) until it is drawn.
    pub fn traces_pending(&self) -> bool {
        self.documents
            .get(self.active)
            .is_some_and(|open| open.motion.trace_jobs.values().any(TraceJob::pending))
    }

    /// The traces of the active document, those kept and those asked for
    /// in the background (pending, or why they failed), by name.
    pub fn trace_states(&mut self) -> Vec<TraceState> {
        let Some(open) = self.documents.get_mut(self.active) else {
            return Vec::new();
        };
        // Read only: a trace of a state the document left is said to be
        // let go of here, and let go of when the session polls
        // (`check_trace_base`).
        let current = Arc::clone(open.history.current());
        let generation = open.asm.generation;
        let mut states: BTreeMap<String, TraceState> = open
            .motion
            .trace_jobs
            .iter()
            .filter(|(_, job)| {
                job.pending() || (Arc::ptr_eq(&job.base, &current) && job.generation == generation)
            })
            .map(|(name, job)| {
                let stale = !Arc::ptr_eq(&job.base, &current) || job.generation != generation;
                let error = match &job.error {
                    None if stale => {
                        Some("let go of: the document changed before it was done".to_string())
                    }
                    other => other.clone(),
                };
                (
                    name.clone(),
                    TraceState {
                        name: name.clone(),
                        pending: error.is_none(),
                        error,
                        length: None,
                        closed: None,
                    },
                )
            })
            .collect();
        for trace in open.traces().iter() {
            states
                .entry(trace.name.clone())
                .or_insert_with(|| TraceState {
                    name: trace.name.clone(),
                    pending: false,
                    error: None,
                    length: Some(polyline_length(&trace.points)),
                    closed: Some(ends_where_it_began(
                        &trace.points,
                        trace.base.tolerance.linear,
                    )),
                });
        }
        states.into_values().collect()
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

/// The moment `view.motion` answers with: made now when the one kept is
/// not it, so its errors are the command's; provisional only when the
/// command asked for `shapes: "ready"`.
fn moment_of_command(open: &mut OpenDocument) -> Result<Option<State>, CommandError> {
    // What the background built since: kept before the moment is made.
    open.poll_motion();
    if open.motion.t.is_some() {
        let base = open.scrubbed();
        let t = open.motion.t.unwrap_or(0.0);
        let generation = open.asm.generation;
        let ready = open.motion.ready;
        let outdated = open.motion.state.as_ref().is_none_or(|s| {
            !Arc::ptr_eq(&s.base, &base)
                || s.generation != generation
                || s.t != t
                || s.collisions_asked != open.motion.collisions
                || (!s.exact && !ready)
        });
        if outdated {
            let state = open
                .make_motion_state(&base, t)
                .map_err(|e| CommandError::new(format!("view.motion: {e}")))?;
            open.motion.state = Some(state);
        }
    }
    Ok(open.motion_state())
}

fn view_motion(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "view.motion";
    let p = Params::new(ID, params, &["t", "collisions", "shapes"])?;
    let collisions = p.bool("collisions")?;
    let ready = match p.get("shapes") {
        None => false,
        Some(Value::String(s)) if s == "ready" => true,
        Some(Value::String(s)) if s == "exact" => false,
        Some(other) => {
            return Err(CommandError::new(format!(
                "{ID}: shapes is \"ready\" or \"exact\", not {}",
                short(other)
            )));
        }
    };
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
            open.reset_builder("the motion was let go of (view.motion {t: null})");
            open.motion.memo = PairMemo::default();
        }
    }
    if let Some(collisions) = collisions {
        open.motion.collisions = collisions;
    }
    // Made elsewhere (the page's worker): the moment is asked for there;
    // what is known of it now is the answer (`pending` until it came).
    if open.remote_wanted() {
        return remote_motion_answer(session, rest);
    }
    // For this command only: every other picture is of the exact moment.
    open.motion.ready = ready;
    let state = moment_of_command(open);
    open.motion.ready = false;
    let state = state?;
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
        "exact": state.exact,
        "stale": state.placed.as_ref().map_or_else(Vec::new, |p| p.stale.clone()),
        "building": open.motion_building(),
        "baked": open.motion.sweep.as_ref().and_then(|(_, _, sweep)| sweep.bake_frames()),
    }))
}

/// `view.motion`'s answer while the view is made elsewhere: the moment
/// asked for, `pending: true` until the other side made it, then what it
/// said of it (the answer `view.motion` gives there).
fn remote_motion_answer(session: &mut Session, rest: Result<f64, String>) -> Run {
    let timeline_json = match &rest {
        Ok(value) => json!({"variable": TIMELINE, "value": value}),
        Err(why) => json!({"variable": TIMELINE, "error": why}),
    };
    let Some(view) = session.motion() else {
        return Ok(json!({"t": Value::Null, "timeline": timeline_json, "collisions": Value::Null}));
    };
    let facts = session
        .remote_facts()
        .flatten()
        .filter(|_| !view.pending)
        .and_then(|f| f.motion.clone());
    let mut answer = facts.unwrap_or_else(|| {
        json!({
            "t": view.t,
            "timeline": timeline_json,
            "collisions": Value::Null,
        })
    });
    if let Some(map) = answer.as_object_mut() {
        map.insert("pending".into(), view.pending.into());
        map.insert("elsewhere".into(), true.into());
        map.insert("building".into(), view.building.into());
        if let Some(why) = &view.error {
            map.insert("error".into(), why.clone().into());
        }
    }
    Ok(answer)
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
        &[
            "point",
            "feature",
            "t",
            "samples",
            "name",
            "grow",
            "clear",
            "background",
            "wait",
        ],
    )?;
    if let Some(clear) = p.get("clear") {
        let others: Vec<&str> = [
            "point",
            "feature",
            "t",
            "samples",
            "name",
            "grow",
            "background",
            "wait",
        ]
        .into_iter()
        .filter(|k| p.has(k))
        .collect();
        if let Some(key) = others.first() {
            return fail(format!(
                "{ID}: `clear` removes traces; it does not go with `{key}`"
            ));
        }
        let open = session.document_mut()?;
        open.check_trace_base();
        // Whatever is still worked out for them is let go of when it comes.
        open.motion.trace_serial = open.motion.trace_serial.wrapping_add(1);
        let before = open
            .traces()
            .len()
            .saturating_add(open.motion.trace_jobs.len());
        match clear {
            Value::Bool(true) => {
                open.traces().clear();
                open.motion.trace_jobs.clear();
            }
            Value::Bool(false) => {}
            Value::String(name) => {
                open.traces().retain(|trace| &trace.name != name);
                open.motion.trace_jobs.remove(name);
                let after = open
                    .traces()
                    .len()
                    .saturating_add(open.motion.trace_jobs.len());
                if after == before {
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
        let after = open
            .traces()
            .len()
            .saturating_add(open.motion.trace_jobs.len());
        return Ok(json!({"removed": before.saturating_sub(after), "traces": open.traces().len()}));
    }
    let wait = p.bool("wait")?.unwrap_or(false);
    if !p.has("point") {
        // A trace asked for before, by its name: its result once it is
        // worked out (waiting for it with `wait`), or why it failed.
        let others: Vec<&str> = ["feature", "t", "samples", "grow", "background"]
            .into_iter()
            .filter(|k| p.has(k))
            .collect();
        if let Some(key) = others.first() {
            return fail(format!(
                "{ID}: `{key}` goes with `point`; without one, view.trace {{name}} reads a trace asked for before"
            ));
        }
        let Some(name) = p.str("name")?.map(str::to_string) else {
            return fail(format!(
                "{ID}: give `point` to trace one, or `name` to read a trace asked for before"
            ));
        };
        return read_trace(session, &name, wait);
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
    let background = p.bool("background")?.unwrap_or(session.bake_in_background)
        && crate::motion_build::Builder::available();
    let open = session.document_mut()?;
    open.check_trace_base();
    // The document a trace belongs to (an edit lets go of it), and the
    // state it is taken in: the one the view shows below the motion, as
    // the player's.
    let base = Arc::clone(open.history.current());
    let shown = open.scrubbed();
    timeline(&shown).map_err(|e| CommandError::new(format!("{ID}: {e}")))?;
    let point = PointRef::parse(ID, &base, &p)?;
    let generation = open.asm.generation;
    // The motion's own sweep over that state: what a bake built is used,
    // and it is the sweep the player goes on with (it shows the same
    // state), so a background trace is not let go of when the play starts
    // (a sweep of the trace's own over a state the motion's was not of
    // was replaced by the play's, which reset the builder). A moment is a
    // function of `t` alone: the points are those of any sweep.
    let sweep: &mut Sweep = open
        .motion_sweep(&shown)
        .map_err(|e| CommandError::new(format!("{ID}: {e}")))?;
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
    if !background {
        let ts = samples_of(range, samples);
        let points = trace_points(sweep, &point, in_assembly.as_deref(), &ts, &|| false)
            .map_err(CommandError::new)?;
        let name = trace_name(open, given_name);
        return add_trace(open, base, generation, name, point, points, range, grow);
    }
    // In a window: worked out by the motion's builder on a copy of the
    // sweep, so the window goes on playing (the developer's field test of
    // 2026-11-02 found a play stopped for as long as a trace of 641
    // samples built its configurations on the engine thread). What is
    // wrong with the point is said now: it is found at the first `t`.
    trace_points(sweep, &point, in_assembly.as_deref(), &[range.0], &|| false)
        .map_err(CommandError::new)?;
    let mut copy = sweep.clone();
    if let Some(asm) = copy.assembly.as_mut() {
        asm.asm.deferred.clear();
        // A trace's samples are not the play's frames: the copy keeps
        // little of what it builds, and gives none of it to the motion
        // (taken in, hundreds of configurations pushed a bake's frames
        // out of the budget and the play built them again).
        asm.asm
            .set_config_budget(TRACE_CONFIG_BUDGET.min(asm.asm.config_budget()));
    }
    let name = trace_name(open, given_name);
    let others = open
        .traces()
        .iter()
        .filter(|t| t.name != name)
        .count()
        .saturating_add(
            open.motion
                .trace_jobs
                .iter()
                .filter(|(n, job)| **n != name && job.pending())
                .count(),
        );
    if others >= MAX_TRACES {
        return fail(format!(
            "{ID}: {MAX_TRACES} traces are kept or being worked out already; remove some with clear"
        ));
    }
    open.motion.trace_serial = open.motion.trace_serial.wrapping_add(1);
    let serial = open.motion.trace_serial;
    let run = TraceRun {
        copy: Box::new(copy),
        ts: samples_of(range, samples),
        points: Vec::new(),
        point: point.clone(),
        in_assembly: in_assembly.clone(),
        name: name.clone(),
        base: Arc::clone(&base),
        generation,
        range,
        grow,
    };
    if !open
        .motion
        .builder
        .queue_trace(trace_job(run), name.clone(), serial)
    {
        // No thread after all: worked out here.
        let ts = samples_of(range, samples);
        let sweep = open
            .motion_sweep(&shown)
            .map_err(|e| CommandError::new(format!("{ID}: {e}")))?;
        let points = trace_points(sweep, &point, in_assembly.as_deref(), &ts, &|| false)
            .map_err(CommandError::new)?;
        return add_trace(open, base, generation, name, point, points, range, grow);
    }
    open.motion.trace_jobs.insert(
        name.clone(),
        TraceJob {
            serial,
            base: Arc::clone(&base),
            generation,
            error: None,
        },
    );
    if wait {
        return read_trace(session, &name, true);
    }
    Ok(json!({
        "name": name,
        "point": point.describe(),
        "t": [range.0, range.1],
        "samples": samples,
        "grow": grow,
        "pending": true,
        "unit": length_unit(&base),
    }))
}

/// A trace worked out in the background: the copy of the sweep it works
/// on, the `t` of its samples and the points found so far, and what it
/// is delivered as.
struct TraceRun {
    copy: Box<Sweep>,
    ts: Vec<f64>,
    points: Vec<Vec3>,
    point: PointRef,
    in_assembly: Option<String>,
    name: String,
    base: Arc<Document>,
    generation: u64,
    range: (f64, f64),
    grow: bool,
}

/// Samples a background trace works out before it lets other jobs of the
/// builder run (it goes on behind them): with one worker (a machine of
/// four to seven threads), a trace of 641 samples held every other
/// background build up for minutes.
const TRACE_CHUNK: usize = 32;

/// The job that works out the next [`TRACE_CHUNK`] samples of `run`, and
/// hands on the job for the rest, or the trace when it is done.
fn trace_job(mut run: TraceRun) -> crate::motion_build::GridJob {
    Box::new(move |let_go: crate::motion_build::Cancel| {
        if let Some(asm) = run.copy.assembly.as_mut() {
            asm.asm.cancel = Some(Arc::clone(&let_go));
        }
        let from = run.points.len();
        let to = from.saturating_add(TRACE_CHUNK).min(run.ts.len());
        let ts: Vec<f64> = run
            .ts
            .get(from..to)
            .map(<[f64]>::to_vec)
            .unwrap_or_default();
        let found = trace_points(
            &mut run.copy,
            &run.point,
            run.in_assembly.as_deref(),
            &ts,
            &|| let_go(),
        );
        let found = match found {
            Ok(found) => found,
            Err(why) => {
                return crate::motion_build::Worked {
                    trace: Some(Err(why)),
                    ..crate::motion_build::Worked::default()
                };
            }
        };
        run.points.extend(found);
        if run.points.len() < run.ts.len() {
            return crate::motion_build::Worked {
                next: Some(trace_job(run)),
                ..crate::motion_build::Worked::default()
            };
        }
        crate::motion_build::Worked {
            trace: Some(Ok(Trace {
                base: run.base,
                generation: run.generation,
                name: run.name,
                point: run.point.describe(),
                points: run.points,
                t: run.range,
                grow: run.grow,
            })),
            ..crate::motion_build::Worked::default()
        }
    })
}

/// A trace asked for before, by its name: its result once it is worked
/// out (`wait`: waiting for it, at most [`TRACE_WAIT`]), or why it failed.
fn read_trace(session: &mut Session, name: &str, wait: bool) -> Run {
    const ID: &str = "view.trace";
    // How long to wait: in a window (a session that bakes in the
    // background) briefly, as assertions do, since the engine thread
    // waiting holds the play and every other client up; elsewhere longer.
    // In a browser there is no clock and nothing is worked out in the
    // background, so nothing is waited for.
    #[cfg(not(target_arch = "wasm32"))]
    let deadline = wait.then(|| {
        let patience = if session.bake_in_background {
            TRACE_WAIT_WINDOW
        } else {
            TRACE_WAIT
        };
        // native clock: cfg(not(target_arch = "wasm32")).
        std::time::Instant::now() + patience
    });
    #[cfg(target_arch = "wasm32")]
    let _ = wait;
    loop {
        let open = session.document_mut()?;
        open.poll_motion();
        open.check_trace_base();
        if let Some(job) = open.motion.trace_jobs.get(name) {
            if let Some(why) = &job.error {
                return fail(format!("{ID}: trace `{name}`: {why}"));
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                // native clock: cfg(not(target_arch = "wasm32")).
                if deadline.is_some_and(|d| std::time::Instant::now() < d) {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                    continue;
                }
            }
            return Ok(json!({"name": name, "pending": true}));
        }
        let Some(trace) = open.traces().iter().find(|t| t.name == name).cloned() else {
            return fail(format!("{ID}: there is no trace `{name}`"));
        };
        return Ok(trace_json(&trace));
    }
}

/// How long `view.trace {wait: true}` waits for a trace worked out in the
/// background, in a script and in a window (where an agent asks again,
/// or follows `ui.get` › `traces`).
#[cfg(not(target_arch = "wasm32"))]
const TRACE_WAIT: std::time::Duration = std::time::Duration::from_secs(600);
#[cfg(not(target_arch = "wasm32"))]
const TRACE_WAIT_WINDOW: std::time::Duration = std::time::Duration::from_secs(2);

/// The configurations a trace worked out in the background keeps while it
/// works (its own copy of the motion's state): each sample asks for its
/// own, used once.
const TRACE_CONFIG_BUDGET: usize = 256 << 20;

/// The name a new trace gets: the one given, else `Trace <n>` free (of
/// the traces kept and those being worked out).
fn trace_name(open: &mut OpenDocument, given: Option<String>) -> String {
    let jobs: Vec<String> = open
        .motion
        .trace_jobs
        .iter()
        .filter(|(_, job)| job.pending())
        .map(|(name, _)| name.clone())
        .collect();
    let traces = open.traces();
    given.unwrap_or_else(|| {
        (1..)
            .map(|n| format!("Trace {n}"))
            .find(|n| traces.iter().all(|t| &t.name != n) && !jobs.contains(n))
            .unwrap_or_else(|| "Trace".to_string())
    })
}

/// The place of `point` at each of `ts`, each moment made as
/// `view.motion` makes it, in `sweep`; stops (an error) when `let_go`
/// says so.
fn trace_points(
    sweep: &mut Sweep,
    point: &PointRef,
    in_assembly: Option<&str>,
    ts: &[f64],
    let_go: &dyn Fn() -> bool,
) -> Result<Vec<Vec3>, String> {
    const ID: &str = "view.trace";
    let mut points = Vec::new();
    for &t in ts {
        if let_go() {
            return Err("let go of: the motion changed before it was done".to_string());
        }
        let err = |e: String| format!("{ID}: `{}` at t = {t}: {e}", point.describe());
        let at = if let Some(name) = in_assembly {
            let (placed, _) = sweep
                .placed_at(t)
                .map_err(|e| format!("{ID}: at t = {t}: {e}"))?;
            crate::asm::point_in(&placed, name).map_err(err)?
        } else {
            let (doc, regen) = sweep.at(t).map_err(|e| format!("{ID}: at t = {t}: {e}"))?;
            point.at(&doc, &regen).map_err(err)?
        };
        points.push(at);
    }
    Ok(points)
}

/// Keeps a trace worked out here and answers `view.trace` with it.
#[allow(clippy::too_many_arguments)]
fn add_trace(
    open: &mut OpenDocument,
    base: Arc<Document>,
    generation: u64,
    name: String,
    point: PointRef,
    points: Vec<Vec3>,
    range: (f64, f64),
    grow: bool,
) -> Run {
    const ID: &str = "view.trace";
    open.motion.trace_jobs.remove(&name);
    let pending = open
        .motion
        .trace_jobs
        .values()
        .filter(|j| j.pending())
        .count();
    let traces = open.traces();
    traces.retain(|trace| trace.name != name);
    if traces.len().saturating_add(pending) >= MAX_TRACES {
        return fail(format!(
            "{ID}: {MAX_TRACES} traces are kept already; remove some with clear"
        ));
    }
    let trace = Trace {
        base,
        generation,
        name,
        point: point.describe(),
        points,
        t: range,
        grow,
    };
    let answer = trace_json(&trace);
    traces.push(trace);
    Ok(answer)
}

/// What `view.trace` answers about a trace: its points and what is
/// measured of them.
fn trace_json(trace: &Trace) -> Value {
    let points = &trace.points;
    let tolerance = trace.base.tolerance.linear;
    let closed = ends_where_it_began(points, tolerance);
    let bbox = craft3d_math::Bbox3::from_points(points.iter().copied());
    let plane = plane_of(points, tolerance).map_or(
        Value::Null,
        |(at, n)| json!({"point": vec3(at), "normal": vec3(n)}),
    );
    json!({
        "name": trace.name,
        "point": trace.point,
        "t": [trace.t.0, trace.t.1],
        "samples": points.len(),
        "grow": trace.grow,
        "points": points.iter().map(|p| vec3(*p)).collect::<Vec<_>>(),
        "length": polyline_length(points),
        "closed": closed,
        "box": bbox.corners().map(|(lo, hi)| json!([vec3(lo), vec3(hi)])),
        "plane": plane,
        "unit": length_unit(&trace.base),
    })
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
    open.poll_motion();
    if let Some(job) = open.motion.trace_jobs.get(name) {
        return fail(match &job.error {
            None => format!(
                "{command}: the trace `{name}` is still being worked out in the background; wait for it with view.trace {{name, wait: true}}"
            ),
            Some(why) => format!("{command}: the trace `{name}` could not be made: {why}"),
        });
    }
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

    /// The configurations a provisional placement leaves to the builder
    /// are set for that placement only: an error out of it (here the
    /// document has no timeline) leaves none set, as a panic does, so no
    /// grid point is solved later with them skipped.
    #[test]
    fn deferred_configurations_are_let_go_of_after_an_error_or_a_panic() {
        let doc = Document::default();
        let mut sweep = Sweep::new(&doc, RegenCache::default());
        sweep.assembly = Some(AsmSweep {
            path: None,
            asm: AsmState::default(),
            grid: BTreeMap::new(),
            solves: BTreeMap::new(),
            bake_ts: BTreeSet::new(),
            seen: BTreeMap::new(),
            grid_stopped: false,
            frame_keys: BTreeSet::new(),
            grid_target: None,
            bake_queue: std::collections::VecDeque::new(),
            last_carried: None,
        });
        let deferred = |sweep: &Sweep| sweep.assembly.as_ref().map(|a| a.asm.deferred.len());
        let keys: BTreeSet<String> = ["a|1", "b|2"].iter().map(|k| k.to_string()).collect();
        let mut builder = crate::motion_build::Builder::default();
        let mut seen_inside = None;
        let out = sweep.with_deferred(keys.clone(), |sweep| {
            seen_inside = deferred(sweep);
            sweep.placed_ready_deferred(0.5, 64, 0, &doc, &mut builder)
        });
        assert!(out.is_err());
        assert_eq!(seen_inside, Some(2), "set while it works");
        assert_eq!(deferred(&sweep), Some(0));
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            sweep.with_deferred(keys, |_| -> Result<(), String> { panic!("inside") })
        }));
        assert!(panicked.is_err());
        assert_eq!(deferred(&sweep), Some(0));
    }

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
