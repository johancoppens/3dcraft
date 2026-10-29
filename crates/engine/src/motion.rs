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
//!   others). Each state starts from the one shown before it, sketches
//!   included, so a linkage solved in a sketch stays on its branch from
//!   frame to frame. With `collisions` the bodies that overlap at that `t`
//!   are drawn red, in the scene itself, so every renderer and every
//!   picture shows them.
//! - `view.trace` samples the path of a named point (a vertex by its
//!   persistent name, or a sketch point) over a range of `t`; the trace
//!   is drawn in the viewport and `file.export {trace}` writes it as SVG
//!   or DXF in its plane.
//! - `motion.check` checks any assertion over a range of `t`: the first
//!   `t` it fails at, and where its measure is least and greatest (a
//!   clearance's minimum, the first collision of an `overlap`).
//!
//! What is rebuilt, honestly: a part's features that depend on `t` are
//! regenerated at every frame (only those). An assembly's instances are
//! placed by transforms only (docs/design/assemblies.md); until assemblies
//! play `t` (track C, phase 2), `t` drives the variables of a part.

use crate::commands::{Run, fail, length_unit, spec, vec3};
use crate::params::Params;
use crate::selection::{Resolver, SelectionKind, feature_named};
use crate::{CommandError, CommandSpec, OpenDocument, Session, Workspace};
use craft3d_doc::{Appearance, Document, EntityId, FeatureId};
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
        "Shows the model in the view at a moment t of its motion, without changing the document: not its variables, its undo history or its dirty state (view state, as view.scrub). The timeline is the document's variable t when it is a plain number (no unit): formulas of t drive the motion (a crank angle 360 deg * t). Only the features that depend on t are regenerated, and only their bodies are meshed again; each state starts from the one shown before it, sketches included, so a linkage stays on its branch. collisions: true draws the bodies that overlap at t red (in the scene, so view.render, the window and view.replay show it) and lists the pairs. t null shows the document as it is again. The viewport, view.render, view.pick and screenshots show the state; document.inspect, measures and exports still see the document itself. An edit keeps the motion (the state is made again from the edited document); a document whose t is gone or has a unit lets go of it. Without parameters it changes nothing. Returns t (null when not shown), the timeline (its variable and its value in the document), rebuilt (the features whose bodies differ from the document's at this t) and collisions ([{a, aName, b, bName, volume}], or unknown pairs with a reason) when asked for.",
        doc: true,
        view_motion
    ),
    spec!(
        "view.trace",
        "Trace Point",
        r#"{"point": vertex name or {"sketch": id or name, "entity": entity id}, "feature": id or name = the body that has the vertex, "t": [number, number] = [0, 1], "samples": integer = 65, "name": string = "Trace <n>", "clear": boolean or trace name}"#,
        "Traces the path of a point over the motion: the point is a vertex by its persistent name (document.edges lists them; feature says which body when several have it) or a point of a sketch ({sketch, entity}); its place is taken at samples values of t evenly from t[0] to t[1] (both included), each state regenerated as view.motion makes it, from the one before. The trace is view state of the document (not saved, not undone, let go of at the next edit): the viewport draws it as a line, and file.export {path: *.svg or *.dxf, trace: name} writes it in its plane. A trace with the name of one kept replaces it; at most 32 are kept. clear: true removes every trace, clear: name one of them (nothing is traced then). The document needs a timeline (view.motion). A point that is gone at some t fails the trace, naming that t. Returns name, point, t, samples, points ([x, y, z] in the document's unit), length (along the polyline), closed (it ends where it began, within the tolerance), box and plane ({point, normal} when the path lies in one plane within the tolerance, else null).",
        doc: true,
        view_trace
    ),
    spec!(
        "motion.check",
        "Check Over Motion",
        r#"{"assertions": [name, ...] = every assertion (or only measure's), "measure": {the parameters of assert.add without name}, "t": [number, number] = [0, 1], "samples": integer = 33, "details": boolean = false}"#,
        "Checks assertions over the motion: at samples values of t evenly from t[0] to t[1] (both included) the document is set to that t (as view.motion does, each state from the one before) and the assertions are measured as document.check measures them, on a copy: the document does not change. measure is an assertion written as for assert.add (without its name) and checked only here; assertions names the document's own (default: all of them, or none when measure is given). Any measure works: overlap with is false gives the first t of a collision, clearance with a min the least clearance and its t. Per assertion: status (failed when any sample fails; else error, notProven, passed, or inactive when it is never on), failed (how many samples), firstFailure ({t, measured, message}, null when none), min and max ({t, value} of a numeric measure), unit and expected; with details every sample ({t, status, measured}). ok is true when every assertion passed. The document needs a timeline (view.motion). At most 1000 samples.",
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

/// Most answers about pairs of bodies kept between states.
const MAX_MEMO: usize = 4_096;

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

/// A run through states of one document over `t`: each state is the one
/// before with `t` set (its sketches settled from where they were, so a
/// linkage stays on its branch), regenerated with a cache of its own that
/// starts as a copy of the document's.
#[derive(Clone)]
pub(crate) struct Sweep {
    doc: Document,
    cache: RegenCache,
}

impl Sweep {
    pub(crate) fn new(base: &Document, cache: RegenCache) -> Self {
        Self {
            doc: base.clone(),
            cache,
        }
    }

    /// The state at `t`, and its regeneration.
    pub(crate) fn at(&mut self, t: f64) -> Result<(Arc<Document>, Arc<Regen>), String> {
        self.doc
            .set_variable(TIMELINE, &literal(t))
            .map_err(|e| e.to_string())?;
        craft3d_regen::settle_sketches_cached(&mut self.doc, &mut self.cache);
        let regen = craft3d_regen::regenerate_cached(&self.doc, &mut self.cache);
        Ok((Arc::new(self.doc.clone()), Arc::new(regen)))
    }
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

/// Answers about pairs of bodies by the bodies' addresses, holding the
/// bodies so that the addresses are not taken by others.
#[derive(Clone, Default)]
pub(crate) struct PairMemo {
    answers: BTreeMap<(usize, usize), (Arc<Body>, Arc<Body>, Answer)>,
}

fn address(body: &Arc<Body>) -> usize {
    Arc::as_ptr(body) as usize
}

/// The pairs of the visible bodies of `regen` that overlap (or that the
/// kernel could not decide), in the order of the bodies. Bodies whose
/// boxes do not meet are clear at the cost of the boxes; a pair of bodies
/// asked about before (the same bodies: a pair `t` does not move) is not
/// asked again.
pub(crate) fn collisions(
    doc: &Document,
    regen: &Regen,
    memo: &mut PairMemo,
) -> Result<Vec<Collision>, String> {
    let tolerance = Tolerance::new(doc.tolerance.linear, doc.tolerance.angular)
        .ok_or("the document's tolerance is not positive and finite")?;
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
    let mut pairs = Vec::new();
    for (i, (_, a, abox)) in bodies.iter().enumerate() {
        for (j, (_, b, bbox)) in bodies.iter().enumerate().skip(i.saturating_add(1)) {
            if crate::interference_cmds::boxes_meet(abox.as_ref(), bbox.as_ref(), margin) {
                let key = (address(a), address(b));
                if !memo.answers.contains_key(&key) {
                    pairs.push((i, j));
                }
            }
        }
    }
    if pairs.len() > MAX_COLLISION_PAIRS {
        return Err(format!(
            "{} pairs of bodies have boxes that meet, more than the {MAX_COLLISION_PAIRS} one state checks",
            pairs.len()
        ));
    }
    let indices: Vec<usize> = (0..pairs.len()).collect();
    let found = crate::interference_cmds::in_order(&indices, |n| {
        let (i, j) = pairs.get(n).copied()?;
        let (a, b) = (bodies.get(i)?.1, bodies.get(j)?.1);
        Some(match interference(a, b, &tolerance) {
            Ok(Interference::Overlap(common)) => craft3d_topo::mass_properties(&common.body)
                .map(|mass| mass.volume)
                .map_err(|e| format!("the common volume could not be measured: {e}")),
            Ok(_) => Ok(0.0),
            Err(e) => Err(e.to_string()),
        })
    });
    if memo.answers.len().saturating_add(pairs.len()) > MAX_MEMO {
        memo.answers.clear();
    }
    for ((i, j), answer) in pairs.iter().zip(found) {
        let (Some(a), Some(b)) = (bodies.get(*i), bodies.get(*j)) else {
            continue;
        };
        let answer = answer.unwrap_or_else(|| Err("the query stopped unexpectedly".to_string()));
        memo.answers.insert(
            (address(a.1), address(b.1)),
            (Arc::clone(a.1), Arc::clone(b.1), answer),
        );
    }
    let mut out = Vec::new();
    for (i, (ida, a, abox)) in bodies.iter().enumerate() {
        for (idb, b, bbox) in bodies.iter().skip(i.saturating_add(1)) {
            if !crate::interference_cmds::boxes_meet(abox.as_ref(), bbox.as_ref(), margin) {
                continue;
            }
            match memo.answers.get(&(address(a), address(b))).map(|e| &e.2) {
                Some(Ok(volume)) if *volume > 0.0 => out.push(Collision {
                    a: *ida,
                    b: *idb,
                    volume: Some(*volume),
                    unknown: None,
                }),
                Some(Err(reason)) => out.push(Collision {
                    a: *ida,
                    b: *idb,
                    volume: None,
                    unknown: Some(reason.clone()),
                }),
                _ => {}
            }
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
    t: f64,
    collisions_asked: bool,
    /// What is drawn: the state with its colliding bodies marked.
    doc: Arc<Document>,
    regen: Arc<Regen>,
    rebuilt: Vec<FeatureId>,
    collisions: Option<Result<Vec<Collision>, String>>,
}

/// A trace of a point over the motion: view state of one document state.
#[derive(Clone)]
pub(crate) struct Trace {
    base: Arc<Document>,
    pub(crate) name: String,
    pub(crate) point: String,
    pub(crate) points: Vec<Vec3>,
}

/// Where the motion of an open document is: view state, not saved and
/// not undone.
#[derive(Clone, Default)]
pub(crate) struct Motion {
    /// The `t` the view shows; `None`: the document as it is.
    t: Option<f64>,
    collisions: bool,
    /// The run of states the view goes through, with the document state
    /// it runs over.
    sweep: Option<(Arc<Document>, Sweep)>,
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
        let reuse = self
            .motion
            .sweep
            .as_ref()
            .is_some_and(|(of, _)| Arc::ptr_eq(of, base));
        if !reuse {
            self.regen();
            let cache = self.cache.clone();
            self.motion.sweep = Some((Arc::clone(base), Sweep::new(base, cache)));
        }
        let (_, sweep) = self
            .motion
            .sweep
            .as_mut()
            .ok_or("the motion lost its states")?;
        let (plain, regen) = sweep.at(t)?;
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
            t,
            collisions_asked: self.motion.collisions,
            rebuilt: rebuilt(&base_regen, &regen),
            doc,
            regen,
            collisions,
        })
    }

    /// The state of the document the view shows: the document itself,
    /// the state it is scrubbed to (`view.scrub`), or that at the `t` of
    /// its motion (`view.motion`).
    pub(crate) fn shown(&mut self) -> Arc<Document> {
        match self.motion_state() {
            Some(state) => state.doc,
            None => self.scrubbed(),
        }
    }

    /// The regeneration of [`Self::shown`].
    pub(crate) fn shown_regen(&mut self) -> Arc<Regen> {
        match self.motion_state() {
            Some(state) => state.regen,
            None => self.scrubbed_regen(),
        }
    }

    /// The traces of the document as it is; lets go of those of another
    /// state of it.
    fn traces(&mut self) -> &mut Vec<Trace> {
        let current = Arc::clone(self.history.current());
        self.motion
            .traces
            .retain(|trace| Arc::ptr_eq(&trace.base, &current));
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
}

/// A trace as a viewer draws it ([`Session::traces`]).
#[derive(Clone, Debug, PartialEq)]
pub struct TraceView {
    pub name: String,
    pub points: Vec<Vec3>,
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
        }
    }
    if let Some(collisions) = collisions {
        open.motion.collisions = collisions;
    }
    if open.motion.t.is_some() {
        // Made now, so the state's errors are this command's.
        let base = open.scrubbed();
        let t = open.motion.t.unwrap_or(0.0);
        let stale = open.motion.state.as_ref().is_none_or(|s| {
            !Arc::ptr_eq(&s.base, &base) || s.t != t || s.collisions_asked != open.motion.collisions
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
    let name = |id: FeatureId| {
        doc.feature(id)
            .map_or_else(|| id.to_string(), |f| f.name.clone())
    };
    let timeline_json = match &rest {
        Ok(value) => json!({"variable": TIMELINE, "value": value}),
        Err(why) => json!({"variable": TIMELINE, "error": why}),
    };
    let Some(state) = state else {
        return Ok(json!({"t": Value::Null, "timeline": timeline_json, "collisions": Value::Null}));
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

/// The plane a polyline lies in within `tolerance`: a point of it and its
/// normal, `None` when it does not lie in one or spans none (a point, a
/// straight line).
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
        &["point", "feature", "t", "samples", "name", "clear"],
    )?;
    if let Some(clear) = p.get("clear") {
        let others: Vec<&str> = ["point", "feature", "t", "samples", "name"]
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
    open.regen();
    let mut sweep = Sweep::new(&base, open.cache.clone());
    let mut points = Vec::new();
    for t in samples_of(range, samples) {
        let (doc, regen) = sweep
            .at(t)
            .map_err(|e| CommandError::new(format!("{ID}: at t = {t}: {e}")))?;
        let at = point.at(&doc, &regen).map_err(|e| {
            CommandError::new(format!("{ID}: `{}` at t = {t}: {e}", point.describe()))
        })?;
        points.push(at);
    }
    let tolerance = base.tolerance.linear;
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
        name: name.clone(),
        point: point.describe(),
        points: points.clone(),
    });
    let closed = match (points.first(), points.last()) {
        (Some(a), Some(b)) => (*a - *b).length() <= tolerance,
        _ => false,
    };
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
    let closed = match (trace.points.first(), trace.points.last()) {
        (Some(a), Some(b)) => trace.points.len() > 2 && (*a - *b).length() <= tolerance,
        _ => false,
    };
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
    let mut copy = OpenDocument::new(base, None);
    copy.cache = cache;
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
    for t in samples_of(range, samples) {
        let text = literal(t);
        crate::commands::edit_merging(
            &mut scratch,
            ID,
            "Motion",
            Some("motion.check.t"),
            true,
            |doc| doc.set_variable(TIMELINE, &text),
        )?;
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
            let first = entry.samples.iter().find(|x| x.1 == S::Failed).map_or(
                Value::Null,
                |(t, _, value, message)| json!({"t": t, "measured": value, "message": message}),
            );
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
            out.insert("firstFailure".into(), first);
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
        "ok": all_passed,
        "assertions": results,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

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
                Vec3::new(3.0 + a.cos(), 5.0, a.sin())
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
