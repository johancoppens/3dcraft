//! Making a motion ahead (docs/design/motion.md › *Configurations over a
//! motion*): `motion.bake` builds every moment a play at a number of
//! frames will ask for, side by side, so that playing afterwards only
//! places poses (an assembly) or looks the moments up (a part). View
//! state only: the document does not change, and nothing shown changes
//! either, only when it is made.

use crate::commands::{Run, spec};
use crate::motion::PairMemo;
use crate::params::Params;
use crate::{CommandError, CommandSpec, Session};
use serde_json::{Value, json};

pub(crate) static SPECS: &[CommandSpec] = &[spec!(
    "motion.bake",
    "Bake Motion",
    r#"{"frames": integer from 1 to 1000 = 120, "collisions": boolean = false, "limit": integer = no limit, "background": boolean = false (true in the window), "cancel": boolean = false}"#,
    "Makes the motion of the active document ahead for a play at frames frames: the moments t = k / frames for k from 0 to frames (both ends included), so that showing them afterwards with view.motion costs only placing the parts (an assembly) or looking the moment up (a part). For an assembly it first solves the fixed steps of t the moments are carried through (building the configurations they ask for, side by side), then builds the configurations of the moments themselves: an instance whose parameters are formulas of t has one per moment, each built from the regeneration of the last configuration of its source, so only the features its changed parameters reach are rebuilt. Configurations are kept within the session's budget (1 GiB, 128 MiB in a browser; beyond it those only solved go first, then the least recently used). For a part it regenerates the moments side by side and keeps them within the same budget. collisions: true also finds each moment's collisions (as view.motion {collisions: true} would), once nothing is left to build. limit builds at most that many configurations (an assembly) or moments (a part) in this call, for a host that bakes a piece at a time between its frames: call again until done is true. View state only: the document, its undo history and what the view shows do not change; an edit or assembly.reload lets go of what was made. The document needs a timeline (view.motion). Returns frames, done (nothing left), remaining (steps of t and moments left for another call), configurations ({wanted: those the moments ask for, built: built now, failed, remaining}, an assembly) or moments (made now, a part), overBudget (what the moments ask for that the budget could not keep: built again when shown), collisions (moments whose collisions were found now), kept and megabytes (configurations or moments kept and about how much memory they take) and ms (the time it took; null where there is no clock). background: true (an assembly, natively) only hands the bake to the motion's background builder and returns at once with queued (the frames) and building (what the background has left: configurations queued or being built, frames not asked for yet, a run of the grid, traces being worked out): the grid is solved by runs in the background that build what it asks for, the frames' configurations are asked for a few at a time whenever the session polls (view.motion, a viewer), behind what view.motion {shapes: \"ready\"} asks for at once; view.motion and a viewer's poll take them in as they come. In a browser, or for a part, background is ignored: the bake is made here. In the window background is true unless given (its thread never waits for a bake). cancel: true drops what is queued in the background (what is being built now is let go of when it comes) and returns cancelled with how many were dropped; view.motion {t: null} and an edit do the same. Every bake remembers frames, which view.motion returns as baked: a player rounds its t to the frames.",
    doc: true,
    motion_bake
)];

/// The moments of a play at `frames` frames: `k / frames` for `k` from 0
/// to `frames`.
pub(crate) fn frames_of(frames: u64) -> Vec<f64> {
    let n = frames.max(1) as f64;
    (0..=frames).map(|k| k as f64 / n).collect()
}

fn motion_bake(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "motion.bake";
    let p = Params::new(
        ID,
        params,
        &["frames", "collisions", "limit", "background", "cancel"],
    )?;
    let frames = p.integer("frames", 1, 1000)?.unwrap_or(120);
    let collisions = p.bool("collisions")?.unwrap_or(false);
    let background = p.bool("background")?.unwrap_or(session.bake_in_background);
    let cancel = p.bool("cancel")?.unwrap_or(false);
    let limit = p
        .integer("limit", 1, 1_000_000)?
        .map_or(usize::MAX, |n| usize::try_from(n).unwrap_or(usize::MAX));
    let clock = Clock::start();
    let open = session.document_mut()?;
    let err = |e: String| CommandError::new(format!("{ID}: {e}"));
    crate::motion::timeline(open.history.current()).map_err(err)?;
    let base = open.scrubbed();
    let ts = frames_of(frames);
    let budget = open.asm.config_budget();
    // Made elsewhere (the page's worker): baked there, a piece at a time
    // after each moment it is asked for; what is left is followed as a
    // bake in the background is (`view.motion`'s `building`).
    if open.remote_wanted() {
        let left = open.bake_left().map_or(0, |b| b.left());
        if cancel {
            open.want_bake(None, collisions);
            return Ok(json!({ "cancelled": left }));
        }
        open.want_bake(Some(frames), collisions);
        let building = open.bake_left().map_or(0, |b| b.left());
        return Ok(json!({
            "frames": frames,
            "background": true,
            "elsewhere": true,
            "queued": frames.saturating_add(1),
            "building": building,
            "ms": Value::Null,
        }));
    }
    if cancel {
        let dropped = open.motion_cancel_background();
        return Ok(json!({ "cancelled": dropped }));
    }
    open.poll_motion();
    let (sweep, builder) = open.motion_sweep_and_builder(&base).map_err(err)?;
    sweep.set_bake_frames(frames);
    if background && sweep.is_assembly() && crate::motion_build::Builder::available() {
        let queued = sweep.bake_background(&ts, builder).map_err(err)?;
        return Ok(json!({
            "frames": frames,
            "background": true,
            "queued": queued,
            "building": open.motion_building(),
            "ms": clock.map_or(Value::Null, |c| json!(c.ms().round())),
        }));
    }
    let mut memo = PairMemo::default();
    let baked = sweep.bake(&ts, limit, collisions, &mut memo, budget);
    // The memo's answers are about bodies of this base: kept for the view.
    *open.motion_memo() = memo;
    let baked = baked.map_err(err)?;
    let megabytes = baked.bytes as f64 / f64::from(1u32 << 20);
    Ok(json!({
        "frames": frames,
        "done": baked.remaining == 0,
        "remaining": baked.remaining,
        "configurations": baked.configs.map_or(Value::Null, |c| json!({
            "wanted": c.wanted,
            "built": c.built,
            "failed": c.failed,
            "remaining": c.remaining,
        })),
        "moments": baked.moments,
        "overBudget": baked.over_budget,
        "collisions": baked.collisions,
        "kept": baked.kept,
        "megabytes": (megabytes * 10.0).round() / 10.0,
        "ms": clock.map_or(Value::Null, |c| json!(c.ms().round())),
    }))
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
