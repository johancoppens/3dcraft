//! A joint of an assembly held by hand for the view (`view.drive`,
//! docs/design/assemblies.md › *Driving a joint by hand*).
//!
//! A mate whose joint the document drives (a formula of `t`), or that
//! follows another through a coupling or a closed loop, cannot be set
//! without contradicting what the document says. `view.drive` shows the
//! assembly with that joint held at a value anyway, as view state (as
//! `view.motion` and `view.scrub` are): the document, its undo history
//! and its dirty state do not change. The solve holds the joint at the
//! value and lets go of the drivers that would contradict it: none when
//! the joint is driven by its own variable (its value takes the driver's
//! place) or free in the solve; else the first other driver (in the
//! assembly's order) whose release frees the joint, or all of them. The
//! others follow, through their couplings and closures. The solve is
//! carried from the document's placement along the branch, the held
//! joint moved from its value there to the one asked for; where the
//! mechanism cannot go on (a dead point, a limit), the view stops at the
//! last value it reaches and says why.

use crate::asm::{Carried, Held, Hold, Placed};
use crate::commands::{Run, fail, length_unit, spec};
use crate::motion::Collision;

/// The collisions found in a state, or why they could not be.
type Found = Result<Vec<Collision>, String>;
use crate::params::Params;
use crate::view_elsewhere::DriveAsk;
use crate::{CommandError, CommandSpec, OpenDocument, Session};
use craft3d_doc::Document;
use craft3d_expr::Dimension;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::sync::Arc;

pub(crate) static SPECS: &[CommandSpec] = &[spec!(
    "view.drive",
    "Drive Joint (View)",
    r#"{"mate": string or null, "value": number or expr, "which": "angle|distance|x|y" = the mate's main free motion, "collisions": boolean = as before (view.motion's switch), "later": boolean = false}"#,
    "Shows the active assembly with one mate's joint held by hand at a value, without changing the document: not its variables, its undo history or its dirty state (view state, as view.motion and view.scrub). Any mate with a free motion can be held: one whose joint a formula drives (a driver of t), one that follows another through a coupling or a closed loop, or one that is free. value is a number (degrees for an angle, the document's length unit for a distance) or an expression (\"30 deg\", \"2.5 mm\"). The solve holds the joint at the value and lets go of the drivers that would contradict it: none when the mate's own variable drives it (the value takes its place) or the joint is free; else the first other driver, in the assembly's order, whose release frees the joint, or all of them (listed in released). Everything else follows: couplings (a planetary's carrier turned by hand turns the sun by the ratio) and closures (a four-bar's rocker turned by hand turns the crank). The solve is carried from the document's placement along the branch; where the mechanism cannot go on (a dead point, a limit) the view stops at the last value reached and stopped says why. mate null lets go: the document's pose is shown again. A view.motion with a t, an edit or another document lets go too. The viewport, view.render, view.pick, screenshots and the assembly's outline show the held state; measures, checks and exports still see the document. Without parameters it changes nothing and says what is held. collisions (one switch with view.motion's and the player's): the held pose is checked as a moment of the motion is, the bodies that overlap drawn red and listed; later: true holds the pose without looking for them yet (a slider being dragged: checkLater says so), and the next view.drive without it (view.drive {} when the drag ends) looks for them. Returns mate (null when nothing is held), motion, unit (deg or the length unit), value (asked), reached, start (the joint's value in the document), released (the mates whose drivers were let go of), stopped (why it stopped short, or null), collisions (as view.motion's: null when not looked for, else [{a, aName, b, bName, volume}] or unknown pairs with a reason) and volumeUnit.",
    doc: true,
    view_drive
)];

/// The joint held by hand, when one is: view state of an open document.
#[derive(Clone, Default)]
pub(crate) struct Drive {
    state: Option<Driven>,
    /// Where the view is made elsewhere (the page's worker,
    /// `view_elsewhere.rs`): the joint asked to be held there, for the
    /// document state and the generation of the files it was asked on.
    remote: Option<(Arc<Document>, u64, u64, DriveAsk)>,
    /// Where the view is made for another session (the worker): why the
    /// joint asked to be held could not be, for the facts.
    pub(crate) refused: Option<(DriveAsk, String)>,
    /// Where the view is made elsewhere: why the worker could not hold
    /// the joint last asked, said once by `view.drive {}`.
    refused_there: Option<(u64, String)>,
    /// The collisions of the held pose are looked for later: the last
    /// `view.drive` asked for the pose alone (a slider being dragged);
    /// the next without `later` looks for them.
    later: bool,
    /// The same, for the person (a notice of the window), taken once.
    refused_notice: Option<String>,
}

/// A joint held by hand and the assembly placed with it.
#[derive(Clone)]
struct Driven {
    /// The document state and the generation of the assembly's files it
    /// was made with: an edit or a reload lets go of it.
    base: Arc<Document>,
    generation: u64,
    /// The history's revision it was made at: an undo and a redo bring
    /// back the same document state, and still let go of it.
    revision: u64,
    mate: String,
    which: &'static str,
    angle: bool,
    /// Radians or model units.
    asked: f64,
    reached: f64,
    start: f64,
    released: Vec<String>,
    stopped: Option<String>,
    placed: Arc<Placed>,
    /// The solves at the grid's values either way from `start`, kept for
    /// the next value of the same joint (a slider being dragged).
    walks: [Walk; 2],
    /// The collisions of the held pose, for the switch they were looked
    /// for with (`None`: not yet), and the state shown: the held pose
    /// with its colliding bodies red, as a moment of the motion is.
    collisions: Option<(bool, Option<Found>, Arc<Document>)>,
}

/// The solves of a held joint at the values `start ± (k + 1) · step` of
/// one way, each carried from the one before (from the document's
/// placement for the first), as far as they were needed; and the first
/// that could not be solved, with why.
#[derive(Clone, Default)]
struct Walk {
    solved: Vec<Carried>,
    dead: Option<(usize, String)>,
    /// Where the mechanism stops on this way (a dead point, a limit),
    /// once found: a later value past it is shown there without a solve.
    limit: Option<Limit>,
}

/// The last value a held joint reaches on one way, its placement and
/// why it cannot go further.
#[derive(Clone)]
struct Limit {
    reached: f64,
    placed: Arc<Placed>,
    why: String,
}

/// Steps of the search for the last value reached when the value asked
/// for cannot be: the fraction of the way is halved this many times.
const SEARCH_STEPS: usize = 30;

/// The step of the grid a held angle is carried along: 5°. In fixed steps
/// from the document's value, so a pose is a function of the value alone
/// (as a moment of the motion is of `t`), and a periodic joint is never
/// taken past a dead point onto the branch a whole step beyond it.
const ANGLE_STEP: f64 = std::f64::consts::PI / 36.0;

/// The step of the grid a held distance is carried along, as a share of
/// the assembly's size.
const DISTANCE_STEP: f64 = 1.0 / 72.0;

/// Most steps of the grid either way: 10 turns of an angle (a solve
/// without bodies a step, so a value at the far end costs at most this
/// many on the engine's thread, about a second on a gearbox).
const MAX_STEPS: usize = 720;

impl OpenDocument {
    /// The assembly placed with its joint held by hand, while it is held
    /// for the document as it is.
    pub(crate) fn driven_placed(&mut self) -> Option<Arc<Placed>> {
        let state = self.drive.state.as_ref()?;
        if Arc::ptr_eq(&state.base, self.history.current())
            && state.generation == self.asm.generation
            && state.revision == self.history.revision()
        {
            return Some(Arc::clone(&state.placed));
        }
        self.drive.state = None;
        None
    }

    /// The state the view shows of a joint held by hand: its pose, with
    /// the bodies that collide there red when collisions are looked for
    /// (the switch `view.motion` and the player use), found as a moment's
    /// are (`motion::collisions`) once per pose and switch, after the
    /// pose was solved.
    pub(crate) fn driven_shown(&mut self) -> Option<Arc<Document>> {
        let placed = self.driven_placed()?;
        let want = self.collisions_asked();
        let fresh = self
            .drive
            .state
            .as_ref()
            .and_then(|s| s.collisions.as_ref())
            .is_some_and(|(asked, _, _)| *asked == want);
        // The pose at once, the check when the drag ends.
        if !fresh && want && self.drive.later {
            return Some(Arc::clone(&placed.doc));
        }
        if !fresh {
            let found = want.then(|| self.collisions_here(&placed.doc, &placed.regen));
            let shown = match &found {
                Some(Ok(pairs)) if pairs.iter().any(|c| c.volume.is_some()) => {
                    Arc::new(crate::motion::marked(&placed.doc, pairs))
                }
                _ => Arc::clone(&placed.doc),
            };
            if let Some(state) = self.drive.state.as_mut() {
                state.collisions = Some((want, found, shown));
            }
        }
        self.drive
            .state
            .as_ref()
            .and_then(|s| s.collisions.as_ref())
            .map(|(_, _, shown)| Arc::clone(shown))
    }

    /// Lets go of a joint held by hand: the document's pose is shown.
    pub(crate) fn let_go_of_drive(&mut self) {
        self.drive.state = None;
        self.drive.remote = None;
        self.drive.refused_there = None;
    }

    /// The name of the mate held by hand, when one is (asked to be held
    /// elsewhere, where the view is made elsewhere).
    pub(crate) fn driven_mate(&mut self) -> Option<String> {
        if self.work.elsewhere() {
            let (base, generation) = (Arc::clone(self.history.current()), self.asm.generation);
            if let Some(ask) = self.drive_asked(&base, generation) {
                return Some(ask.0);
            }
        }
        self.driven_placed()?;
        self.drive.state.as_ref().map(|s| s.mate.clone())
    }

    /// The joint asked to be held by hand where the view is made
    /// elsewhere, while it was asked for `base` with the files of
    /// `generation` (an edit or a reload lets go of it).
    pub(crate) fn drive_asked(
        &mut self,
        base: &Arc<Document>,
        generation: u64,
    ) -> Option<DriveAsk> {
        let (of, at, revision, ask) = self.drive.remote.as_ref()?;
        if !(Arc::ptr_eq(of, base) && *at == generation && *revision == self.history.revision()) {
            self.drive.remote = None;
            return None;
        }
        // The worker could not hold it: said by its facts for this view.
        // Only for this very request: the same mate asked again at another
        // value is not refused with an older answer.
        let bits = format!("{:016x}", ask.2.to_bits());
        let refused = self
            .remote_view_facts_drive()
            .filter(|d| {
                d.get("mate").and_then(Value::as_str) == Some(ask.0.as_str())
                    && d.get("which").and_then(Value::as_str) == Some(ask.1.as_str())
                    && d.get("value").and_then(Value::as_str) == Some(bits.as_str())
            })
            .and_then(|d| d.get("error").and_then(Value::as_str).map(str::to_string));
        if let Some(why) = refused {
            self.drive.remote = None;
            self.drive.refused_notice = Some(why.clone());
            self.drive.refused_there = Some((self.history.revision(), why));
            return None;
        }
        // Collisions as the switch now says (a toggle makes the view
        // again there).
        let mut ask = ask.clone();
        ask.3 = self.collisions_asked() && !self.drive.later;
        Some(ask)
    }
}

impl Session {
    /// The mate of the active document whose joint is held by hand for
    /// the view (`view.drive`), when one is.
    pub fn driven_mate(&mut self) -> Option<String> {
        self.documents.get_mut(self.active)?.driven_mate()
    }

    /// Why the joint last asked to be held by hand could not be, where the
    /// view is made elsewhere (the worker refused it): once, for a notice
    /// the person sees.
    pub fn take_drive_refusal(&mut self) -> Option<String> {
        let open = self.documents.get_mut(self.active)?;
        let base = Arc::clone(open.history.current());
        let generation = open.asm.generation;
        open.drive_asked(&base, generation);
        open.drive.refused_notice.take()
    }

    /// What `view.drive {}` says of the joint held by hand in the active
    /// document (its mate, value, reached and why it stopped), when one
    /// is held; also pending where it is held elsewhere.
    pub fn drive_state(&mut self) -> Option<Value> {
        let open = self.documents.get_mut(self.active)?;
        open.driven_mate()?;
        let doc = Arc::clone(open.history.current());
        let answer = answer(open, &doc);
        answer
            .get("mate")
            .is_some_and(Value::is_string)
            .then_some(answer)
    }
}

fn view_drive(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "view.drive";
    let p = Params::new(
        ID,
        params,
        &["mate", "value", "which", "collisions", "later"],
    )?;
    let collisions = p.bool("collisions")?;
    let later = p.bool("later")?.unwrap_or(false);
    let open = session.document_mut()?;
    open.drive.later = later;
    // One switch with the motion's (and the player's).
    if let Some(on) = collisions {
        open.ask_collisions(on);
    }
    let doc = Arc::clone(open.history.current());
    if !doc.is_assembly() {
        return fail(format!(
            "{ID}: the active document is not an assembly; view.drive holds a mate's joint of an assembly"
        ));
    }
    if !p.has("mate") {
        if p.has("value") || p.has("which") {
            return Err(p.missing("mate"));
        }
        return Ok(answer(open, &doc));
    }
    let Some(name) = p.str("mate")? else {
        open.let_go_of_drive();
        return Ok(answer(open, &doc));
    };
    let name = name.to_string();
    let Some(assembly) = doc.assembly.as_ref() else {
        return fail(format!("{ID}: the active document is not an assembly"));
    };
    let Some(mate) = assembly.mate(&name) else {
        return fail(format!(
            "{ID}: there is no mate `{}`",
            name.chars().take(80).collect::<String>()
        ));
    };
    let free = mate.kind.free();
    let (slot, which) = match p.str("which")? {
        Some(w) => match free.iter().position(|f| *f == w) {
            Some(k) => (k, free.get(k).copied().unwrap_or("angle")),
            None => {
                return fail(format!(
                    "{ID}: a {} mate has no free motion `{}`; it has {}",
                    mate.kind.name(),
                    w.chars().take(40).collect::<String>(),
                    if free.is_empty() {
                        "none".to_string()
                    } else {
                        free.join(", ")
                    }
                ));
            }
        },
        None => match free.first() {
            Some(w) => (0, *w),
            None => {
                return fail(format!(
                    "{ID}: a {} mate has no free motion to hold",
                    mate.kind.name()
                ));
            }
        },
    };
    let angle = which == "angle";
    let value = match p.get("value") {
        None if !p.has("value") => return Err(p.missing("value")),
        None => {
            return fail(format!(
                "{ID}: value is a number or an expression; to let go, give mate null"
            ));
        }
        Some(v) => {
            joint_value(&doc, v, angle).map_err(|e| CommandError::new(format!("{ID}: {e}")))?
        }
    };
    let generation = open.asm.generation;
    let revision = open.history.revision();
    // Made elsewhere (the page's worker): the joint is held there; what
    // it reached comes with the view's facts.
    if open.remote_wanted() {
        open.drive.state = None;
        open.drive.refused_there = None;
        open.drive.refused_notice = None;
        open.drive.remote = Some((
            Arc::clone(&doc),
            generation,
            revision,
            (name.clone(), which.to_string(), value, false),
        ));
        return Ok(answer(open, &doc));
    }
    let path = open.path.clone();
    let Some(placed) = open.placed() else {
        return fail(format!("{ID}: the assembly could not be placed"));
    };
    if let Some(why) = &placed.solve_error {
        return fail(format!(
            "{ID}: the assembly is not solved, so no joint can be held: {why}"
        ));
    }
    let Some(carried) = placed.carried.clone() else {
        return fail(format!("{ID}: the assembly is not solved"));
    };
    let Some(state) = placed.mates.iter().find(|m| m.name == name) else {
        return fail(format!("{ID}: the mate `{name}` is not in the solve"));
    };
    if let Some(why) = &state.error {
        return fail(format!("{ID}: the mate `{name}` takes no part: {why}"));
    }
    let Some(&start) = state.values.get(slot) else {
        return fail(format!(
            "{ID}: the mate `{name}` has no value for its {which} in the solve"
        ));
    };
    // The drivers to let go of: kept from the last call for the same
    // joint of the same state (a slider being dragged).
    let kept = open
        .drive
        .state
        .as_mut()
        .filter(|s| {
            Arc::ptr_eq(&s.base, &doc)
                && s.generation == generation
                && s.revision == revision
                && s.mate == name
                && s.which == which
        })
        .map(|s| (s.released.clone(), std::mem::take(&mut s.walks)));
    let released = match &kept {
        Some((released, _)) => released.clone(),
        None => to_release(
            open,
            &doc,
            path.as_deref(),
            &placed,
            &carried,
            &name,
            slot,
            which,
        )
        .map_err(|e| CommandError::new(format!("{ID}: {e}")))?,
    };
    let release: BTreeSet<String> = released.iter().cloned().collect();
    let step = if angle {
        ANGLE_STEP
    } else {
        DISTANCE_STEP * crate::asm::model_size(&placed)
    };
    if !(step.is_finite() && step > 0.0) {
        return fail(format!(
            "{ID}: the assembly has no size to step a distance by"
        ));
    }
    let span = (value - start).abs() / step;
    if !span.is_finite() || span > MAX_STEPS as f64 {
        return fail(format!(
            "{ID}: {} is too far from the joint's value {}: it is held at most {MAX_STEPS} steps of {} either way",
            shown_value(&doc, value, angle),
            shown_value(&doc, start, angle),
            shown_value(&doc, step, angle),
        ));
    }
    let way = usize::from(value < start);
    let sign = if value < start { -1.0 } else { 1.0 };
    let mut walks = match kept {
        Some((_, walks)) => walks,
        None => [Walk::default(), Walk::default()],
    };
    let hold_at = |value: f64, from: f64| Hold {
        held: Some(Held {
            mate: name.clone(),
            slot,
            value,
            start: from,
        }),
        release: release.clone(),
    };
    let at = |k: usize| start + sign * step * k as f64;
    let shown = |v: f64| shown_value(&doc, v, angle);
    let stopped_at = |reached: f64, why: &str| {
        format!(
            "`{name}` stopped at {} of {} asked for: {why}",
            shown(reached),
            shown(value)
        )
    };
    // Whole steps of the grid up to the value, as far as they go.
    let whole = (span.floor() as usize).min(MAX_STEPS);
    let walk = walks
        .get_mut(way)
        .ok_or_else(|| CommandError::new(format!("{ID}: no way")))?;
    let (held, reached, stopped) = match walk.limit.clone() {
        // Past where it stops on this way, found before: shown there.
        Some(limit) if sign * (value - limit.reached) >= 0.0 => (
            limit.placed,
            limit.reached,
            Some(stopped_at(limit.reached, &limit.why)),
        ),
        _ => {
            while walk.solved.len() < whole && walk.dead.is_none() {
                let k = walk.solved.len();
                let from = walk.solved.last().unwrap_or(&carried).clone();
                let trial = open.asm.place_held(
                    &doc,
                    path.as_deref(),
                    &from,
                    &hold_at(at(k + 1), at(k)),
                    false,
                );
                match (&trial.solve_error, trial.carried) {
                    (None, Some(next)) => walk.solved.push(next),
                    (why, _) => {
                        walk.dead =
                            Some((k, why.clone().unwrap_or_else(|| "the solve failed".into())));
                    }
                }
            }
            // From the last grid value reached, to the value or as near
            // it as the mechanism goes.
            let k = walk.solved.len().min(whole);
            let from = match k.checked_sub(1) {
                Some(last) => walk.solved.get(last).unwrap_or(&carried).clone(),
                None => carried.clone(),
            };
            let base_value = at(k);
            let target = if k < whole { at(k + 1) } else { value };
            let held = open.asm.place_held(
                &doc,
                path.as_deref(),
                &from,
                &hold_at(target, base_value),
                true,
            );
            match held.solve_error.clone() {
                None => (Arc::new(held), target, None),
                Some(failed) => {
                    // Where it stops is searched over the whole grid step
                    // past the grid's last solve, whatever was asked, so
                    // the limit (and why) is the same whichever value
                    // finds it first; the step's end says why.
                    let end = at(k + 1);
                    let why = if target.to_bits() == end.to_bits() {
                        failed
                    } else {
                        open.asm
                            .place_held(
                                &doc,
                                path.as_deref(),
                                &from,
                                &hold_at(end, base_value),
                                false,
                            )
                            .solve_error
                            .unwrap_or(failed)
                    };
                    let (mut good, mut bad) = (0.0f64, 1.0f64);
                    for _ in 0..SEARCH_STEPS {
                        let mid = 0.5 * (good + bad);
                        let v = base_value + (end - base_value) * mid;
                        let trial = open.asm.place_held(
                            &doc,
                            path.as_deref(),
                            &from,
                            &hold_at(v, base_value),
                            false,
                        );
                        if trial.solve_error.is_none() {
                            good = mid;
                        } else {
                            bad = mid;
                        }
                    }
                    let mut reached = base_value + (end - base_value) * good;
                    let mut held = open.asm.place_held(
                        &doc,
                        path.as_deref(),
                        &from,
                        &hold_at(reached, base_value),
                        true,
                    );
                    if held.solve_error.is_some() {
                        // The grid's last value, where its solve is.
                        reached = base_value;
                        held = if k == 0 {
                            (*placed).clone()
                        } else {
                            open.asm.place_held(
                                &doc,
                                path.as_deref(),
                                &from,
                                &hold_at(base_value, base_value),
                                true,
                            )
                        };
                    }
                    if let Some(again) = &held.solve_error {
                        // Never the document's pose instead: the view
                        // stays as it was.
                        return fail(format!(
                            "{ID}: `{name}` cannot be shown at {}: {again}",
                            shown(reached)
                        ));
                    }
                    let held = Arc::new(held);
                    if walk.dead.is_none() {
                        walk.dead = Some((k, why.clone()));
                    }
                    walk.limit = Some(Limit {
                        reached,
                        placed: Arc::clone(&held),
                        why: why.clone(),
                    });
                    (held, reached, Some(stopped_at(reached, &why)))
                }
            }
        }
    };
    open.drive.state = Some(Driven {
        base: Arc::clone(&doc),
        generation,
        revision,
        mate: name,
        which,
        angle,
        asked: value,
        reached,
        start,
        released,
        stopped,
        placed: held,
        walks,
        collisions: None,
    });
    Ok(answer(open, &doc))
}

/// The drivers to let go of so that the joint `slot` of `mate` can be
/// held (the module's documentation): none when its own variable drives
/// it or it is free, else the first other driver whose release frees it,
/// else all of them; an error when even that leaves it held by the mates.
#[allow(clippy::too_many_arguments)]
fn to_release(
    open: &mut OpenDocument,
    doc: &Document,
    path: Option<&str>,
    placed: &Placed,
    carried: &Carried,
    mate: &str,
    slot: usize,
    which: &str,
) -> Result<Vec<String>, String> {
    let own = crate::asm::joint_variable(mate, which);
    if doc.variables.iter().any(|v| v.name == own) {
        return Ok(Vec::new());
    }
    let free_in = |p: &Placed| {
        p.solve_error.is_none()
            && p.mates
                .iter()
                .find(|m| m.name == mate)
                .is_some_and(|m| m.main_free)
    };
    // Whether the main free motion is free says nothing of another one:
    // that one is held as it is, and the solve says whether it can be.
    if slot != 0 || free_in(placed) {
        return Ok(Vec::new());
    }
    let drivers: Vec<String> = placed
        .mates
        .iter()
        .filter(|m| m.driven && m.name != mate && m.error.is_none())
        .map(|m| m.name.clone())
        .collect();
    let trial = |open: &mut OpenDocument, release: BTreeSet<String>| {
        let hold = Hold {
            held: None,
            release,
        };
        open.asm.place_held(doc, path, carried, &hold, false)
    };
    for driver in &drivers {
        let p = trial(open, BTreeSet::from([driver.clone()]));
        if free_in(&p) {
            return Ok(vec![driver.clone()]);
        }
    }
    if drivers.len() > 1 {
        let p = trial(open, drivers.iter().cloned().collect());
        if free_in(&p) {
            return Ok(drivers);
        }
    }
    Err(format!(
        "the joint of `{mate}` is held by its mates (fixed, or a closed loop without a degree of freedom): letting go of {} does not free it",
        if drivers.is_empty() {
            "the drivers (there are none)".to_string()
        } else {
            format!("the drivers ({})", drivers.join(", "))
        }
    ))
}

/// A joint's value from a parameter, in radians or model units: a
/// number (degrees for an angle, the document's length unit for a
/// distance) or an expression of the right dimension.
fn joint_value(doc: &Document, value: &Value, angle: bool) -> Result<f64, String> {
    let v = match value {
        Value::Number(n) => {
            let Some(v) = n.as_f64() else {
                return Err(format!("value {n} is not a number it can use"));
            };
            if angle { v.to_radians() } else { v }
        }
        Value::String(text) => {
            if text.chars().count() > 1_000 {
                return Err("value is longer than 1000 characters".to_string());
            }
            let q = crate::asm::evaluate_in(doc, text).map_err(|e| {
                format!("value `{}`: {e}", text.chars().take(80).collect::<String>())
            })?;
            if angle {
                if q.dimension != Dimension::ANGLE && !q.dimension.is_dimensionless() {
                    return Err(format!(
                        "value `{}` is {}, not an angle",
                        text.chars().take(80).collect::<String>(),
                        q.dimension.described()
                    ));
                }
                q.value
            } else {
                if q.dimension != Dimension::LENGTH {
                    return Err(format!(
                        "value `{}` is {}, not a length",
                        text.chars().take(80).collect::<String>(),
                        q.dimension.described()
                    ));
                }
                crate::asm::in_units(doc, q.value)
            }
        }
        other => {
            return Err(format!(
                "value is a number or an expression, not {}",
                other.to_string().chars().take(80).collect::<String>()
            ));
        }
    };
    if !v.is_finite() || v.abs() > 1e12 {
        return Err(format!("value {v} is not a finite value it can hold"));
    }
    Ok(v)
}

/// A joint's value as the answer gives it: degrees for an angle, the
/// document's length unit for a distance.
fn shown_value(doc: &Document, v: f64, angle: bool) -> String {
    if angle {
        format!("{}°", round(v.to_degrees()))
    } else {
        format!("{} {}", round(v), length_unit(doc))
    }
}

/// Six significant digits, for words.
fn round(v: f64) -> f64 {
    let text = format!("{v:.6e}");
    text.parse().unwrap_or(v)
}

/// What is held, as `view.drive` answers.
fn answer(open: &mut OpenDocument, doc: &Document) -> Value {
    if open.remote_wanted() {
        let base = Arc::clone(open.history.current());
        let generation = open.asm.generation;
        let Some((mate, which, value, _)) = open.drive_asked(&base, generation) else {
            // The worker could not hold the joint last asked: said once.
            // Not after an edit (another revision).
            let revision = open.history.revision();
            return match open.drive.refused_there.take() {
                Some((at, why)) if at == revision => {
                    json!({"mate": Value::Null, "view": true, "error": why})
                }
                _ => json!({"mate": Value::Null, "view": true}),
            };
        };
        let angle = which == "angle";
        // What was answered there for this view, once it came.
        let came = open
            .remote_view()
            .and_then(|v| v.facts.as_ref())
            .and_then(|f| f.drive.clone())
            .filter(|d| d.get("mate").and_then(Value::as_str) == Some(mate.as_str()));
        if let Some(mut came) = came {
            if let Some(map) = came.as_object_mut() {
                map.insert("elsewhere".into(), true.into());
            }
            return came;
        }
        return json!({
            "mate": mate,
            "motion": which,
            "unit": if angle { "deg".to_string() } else { length_unit(doc).to_string() },
            "value": if angle { value.to_degrees() } else { value },
            "elsewhere": true,
            "pending": true,
            "view": true,
        });
    }
    let shown = open.driven_shown();
    let Some(s) = open.drive.state.as_ref() else {
        return json!({"mate": Value::Null, "view": true});
    };
    let found = s
        .collisions
        .as_ref()
        .and_then(|(_, found, _)| found.as_ref());
    let collisions = match &shown {
        Some(shown) => crate::motion::collisions_json(shown, found),
        None => Value::Null,
    };
    let check_later = found.is_none() && open.drive.later && open.collisions_asked();
    let unit = if s.angle {
        "deg".to_string()
    } else {
        length_unit(doc).to_string()
    };
    let out = |v: f64| if s.angle { v.to_degrees() } else { v };
    json!({
        "mate": s.mate,
        "motion": s.which,
        "unit": unit,
        "value": out(s.asked),
        "reached": out(s.reached),
        "start": out(s.start),
        "released": s.released,
        "stopped": s.stopped,
        "solveNote": s.placed.solve_note,
        "collisions": collisions,
        "checkLater": check_later,
        "volumeUnit": format!("{}^3", length_unit(doc)),
        "view": true,
    })
}
