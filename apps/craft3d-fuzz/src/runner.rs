//! Runs the groups of a design in a session and holds every result to the
//! oracles a machine can check (docs/testing.md › Random multi-step
//! designs):
//!
//! - **crash**: a command that panicked (the engine catches it and says
//!   "internal error (a bug in 3DCraft …): it panicked"); a process that
//!   died, hung or ran out of memory is seen by the runner outside
//!   (`cargo xtask fuzz`);
//! - **bug**: a command or a feature that refused with a bug the code
//!   caught itself (its reason says "internal error (a bug in 3DCraft",
//!   `craft3d_math::BUG`, without a panic): no crash, but a bug all the
//!   same, an error or a feature's `failed` status alike;
//! - **wrong**: a group that was accepted and left a body that
//!   `document.check` (deep: topology, self-intersection, the boolean's own
//!   check) finds fault with, a boolean whose volumes break
//!   `V(A ∪ B) + V(A ∩ B) = V(A) + V(B)` or `V(A − B) + V(A ∩ B) = V(A)`,
//!   a plane cut whose body has more volume than its target had, or one by
//!   a single plane whose two sides do not add up to the target, a
//!   regeneration edit by edit that differs from a full one, or a saved
//!   document that opens to a different state;
//! - **refused**: a group a command said no to (an error, or a feature
//!   that failed with a reason). Not a bug; counted by reason. The group is
//!   undone and the design goes on without it.

use crate::steps::{Cmd, Design, Group, Setup, map_strings, resolved_text};
use craft3d_engine::{Session, Workspace};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

/// Saves made by the determinism check in this process: each gets a file
/// of its own, also when designs run on several threads at once.
static SAVES: AtomicU64 = AtomicU64::new(0);

/// The mark of a bug in a message (`craft3d_math::BUG`): a panic the
/// engine caught, or a bug the code caught itself.
const INTERNAL: &str = "internal error (a bug in 3DCraft";

/// The verdict a message carries: `crash` for a caught panic, `bug` for a
/// bug the code caught itself, `None` for an ordinary refusal.
pub(crate) fn bug_kind(message: &str) -> Option<&'static str> {
    if !message.contains(INTERNAL) {
        None
    } else if message.contains(craft3d_engine::PANICKED) {
        Some("crash")
    } else {
        Some("bug")
    }
}

/// Longest reason or detail kept, in characters.
const KEPT: usize = 600;

/// The command of the plane cut step, whose groups get checks of their own
/// ([`Runner::cut_checks`]).
const PLANE_CUT: &str = "feature.planeCut";

/// When the deep check (self-intersection on a fine mesh, the boolean's
/// full check) runs; the shallow one runs after every group.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Deep {
    EveryGroup,
    AtEnd,
}

#[derive(Clone, Debug)]
pub struct Options {
    pub deep: Deep,
    /// Run the sibling booleans of every boolean for the volume identities.
    pub identities: bool,
    /// Save, open and compare at the end.
    pub determinism: bool,
    /// Where the saved copy goes for the determinism check.
    pub scratch: PathBuf,
}

/// Something found: a crash or a wrong result, at a group.
#[derive(Clone, Debug, PartialEq)]
pub struct Finding {
    /// `crash`, `bug` or `wrong`.
    pub kind: &'static str,
    /// The oracle that saw it: `panic`, `bug`, `topology`, `self-intersection`,
    /// `boolean-check`, `unmeasurable`, `identity`, `regen`, `determinism`,
    /// `check-failed`.
    pub oracle: String,
    /// The group it was found at (the number of groups for one found at
    /// the end, by the determinism check).
    pub group: usize,
    pub command: String,
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Refusal {
    pub group: usize,
    pub command: String,
    pub reason: String,
}

impl Refusal {
    /// The reason with its numbers, names and parenthesised details taken
    /// out, so that refusals of one kind count together.
    pub fn key(&self) -> String {
        let mut out = String::new();
        let mut depth = 0_u32;
        let mut quoted = false;
        for c in self.reason.chars() {
            match c {
                '(' | '[' => depth = depth.saturating_add(1),
                ')' | ']' => depth = depth.saturating_sub(1),
                '`' => quoted = !quoted,
                _ if depth > 0 || quoted => {}
                c if c.is_ascii_digit() => {
                    if !out.ends_with('#') {
                        out.push('#');
                    }
                }
                c => out.push(c),
            }
        }
        let words: Vec<&str> = out.split_whitespace().collect();
        let short: String = words.join(" ").chars().take(90).collect();
        format!("{}: {short}", self.command)
    }
}

/// How often a check ran, and how often it could not (a measure or a
/// sibling that was refused): counted, never skipped silently.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tally {
    pub checked: usize,
    pub skipped: usize,
}

impl Tally {
    fn to_json(self) -> Value {
        json!({"checked": self.checked, "skipped": self.skipped})
    }
}

/// The groups of one kind of step (`Group::step`) and how many of them
/// were accepted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Steps {
    pub groups: usize,
    pub accepted: usize,
}

/// Everything a design's run produced.
#[derive(Clone, Debug, PartialEq)]
pub struct Report {
    pub setup: Setup,
    pub groups: usize,
    pub accepted: usize,
    pub refused: Vec<Refusal>,
    pub booleans: usize,
    pub identities_checked: usize,
    pub identities_skipped: usize,
    /// Per kind of step, its groups and how many were accepted.
    pub steps: BTreeMap<String, Steps>,
    /// Plane cuts held to "no more volume than the target had".
    pub cut_volumes: Tally,
    /// Plane cuts by one plane held to "the two sides add up".
    pub cut_sides: Tally,
    pub finding: Option<Finding>,
}

impl Report {
    pub fn verdict(&self) -> &'static str {
        self.finding.as_ref().map_or("ok", |f| f.kind)
    }

    pub fn to_json(&self) -> Value {
        let steps: serde_json::Map<String, Value> = self
            .steps
            .iter()
            .map(|(step, n)| {
                let counts = json!({"groups": n.groups, "accepted": n.accepted});
                (step.clone(), counts)
            })
            .collect();
        let mut value = json!({
            "seed": self.setup.seed,
            "scale": self.setup.scale,
            "verdict": self.verdict(),
            "groups": self.groups,
            "accepted": self.accepted,
            "booleans": self.booleans,
            "identities": {"checked": self.identities_checked, "skipped": self.identities_skipped},
            "steps": steps,
            "cuts": {"volumes": self.cut_volumes.to_json(), "sides": self.cut_sides.to_json()},
            "refused": self.refused.iter().map(|r| json!({
                "group": r.group, "command": r.command, "reason": r.reason, "key": r.key(),
            })).collect::<Vec<_>>(),
        });
        if let (Some(f), Some(map)) = (&self.finding, value.as_object_mut()) {
            map.insert(
                "finding".into(),
                json!({
                    "kind": f.kind, "oracle": f.oracle, "group": f.group,
                    "command": f.command, "detail": f.detail,
                }),
            );
        }
        value
    }
}

/// What applying a group came to.
#[derive(Clone, Debug, PartialEq)]
pub enum Applied {
    Accepted,
    Refused,
    Found,
}

fn cut(text: &str) -> String {
    text.chars().take(KEPT).collect()
}

pub struct Runner {
    session: Session,
    pub setup: Setup,
    options: Options,
    /// Feature name to id, of the features the document has now.
    ids: BTreeMap<String, String>,
    /// Feature id to name.
    names: BTreeMap<String, String>,
    /// The commands of the accepted groups, with ids filled in: a script
    /// `craft3d-cli run --script` replays.
    pub script: Vec<Value>,
    pub report: Report,
}

/// Why a group stopped before its end.
enum Stop {
    Refused(String, String),
    /// A crash or a bug (`bug_kind`), the command and the message.
    Found(&'static str, String, String),
}

/// The oracle that saw a crash (`panic`) or a bug (`bug`).
fn oracle_of_bug(kind: &str) -> &'static str {
    if kind == "crash" { "panic" } else { "bug" }
}

/// With `CRAFT3D_FUZZ_TRACE` set, how long a command took, on stderr
/// (for finding which command of a design is slow).
fn trace(command: &str, started: std::time::Instant) {
    if std::env::var_os("CRAFT3D_FUZZ_TRACE").is_some() {
        eprintln!("{command}: {:.3} s", started.elapsed().as_secs_f64());
    }
}

/// Runs `cmds` in order; the results, or why it stopped.
fn run_commands(
    session: &mut Session,
    cmds: &[Cmd],
    ids: &BTreeMap<String, String>,
) -> Result<Vec<(String, Value)>, Stop> {
    let mut done = Vec::with_capacity(cmds.len());
    for cmd in cmds {
        let params = map_strings(&cmd.params, &|s| resolved_text(s, ids), 0);
        let started = std::time::Instant::now();
        let outcome = session.execute(&cmd.command, &params);
        trace(&cmd.command, started);
        match outcome {
            Ok(result) => {
                if result.get("status").and_then(Value::as_str) == Some("failed") {
                    let reason = result
                        .get("error")
                        .and_then(Value::as_str)
                        .unwrap_or("failed without a reason");
                    return Err(match bug_kind(reason) {
                        Some(kind) => Stop::Found(kind, cmd.command.clone(), cut(reason)),
                        None => Stop::Refused(cmd.command.clone(), cut(reason)),
                    });
                }
                done.push((cmd.command.clone(), params));
            }
            Err(error) => {
                let message = error.message();
                return Err(match bug_kind(message) {
                    Some(kind) => Stop::Found(kind, cmd.command.clone(), cut(message)),
                    None => Stop::Refused(cmd.command.clone(), cut(message)),
                });
            }
        }
    }
    Ok(done)
}

/// The name of the other side of a plane cut, made on a copy of the
/// document ([`Runner::cut_checks`]).
const OTHER_SIDE: &str = "__other";

/// What a plane cut group is held to after it: its params with ids, its
/// target, and the target's volume and area before it (`None` when they
/// could not be measured: the checks are then skipped, counted).
struct CutBefore {
    params: Value,
    target: String,
    measured: Option<(f64, f64)>,
}

/// The params of the cut `params` by its one plane with the other side
/// kept, as a kept copy named [`OTHER_SIDE`]; `None` when it cuts by more
/// than one plane.
fn other_side(params: &Value) -> Option<Value> {
    let plane = match params.get("planes")? {
        Value::Array(items) => match items.as_slice() {
            [one] => one,
            _ => return None,
        },
        one => one,
    };
    let flipped = match plane {
        Value::Object(map) => {
            let mut map = map.clone();
            let above = map.get("side").and_then(Value::as_str) == Some("above");
            map.insert("side".into(), if above { "below" } else { "above" }.into());
            Value::Object(map)
        }
        other => json!({"plane": other, "side": "above"}),
    };
    let mut out = params.as_object()?.clone();
    out.insert("planes".into(), json!([flipped]));
    out.insert("keep".into(), true.into());
    out.insert("name".into(), OTHER_SIDE.into());
    Some(Value::Object(out))
}

/// The volume and area of a feature's body.
fn measure(session: &mut Session, feature: &str) -> Option<(f64, f64)> {
    let m = session
        .execute("document.measure", &json!({ "feature": feature }))
        .ok()?;
    Some((m.get("volume")?.as_f64()?, m.get("area")?.as_f64()?))
}

/// The part of `document.inspect` that must come out the same however the
/// document got there: everything but where it is kept and its undo
/// history.
fn canonical(inspect: &Value) -> Value {
    let mut value = inspect.clone();
    if let Some(map) = value.as_object_mut() {
        for key in ["path", "dirty", "history", "document"] {
            map.remove(key);
        }
    }
    value
}

/// The oracle a problem of `document.check` belongs to.
fn oracle_of(problem: &str) -> &'static str {
    if problem.starts_with("invalid body") {
        "topology"
    } else if problem.contains("passes through or touches itself") {
        "self-intersection"
    } else if problem.starts_with("not the ") || problem.contains("boolean check cannot run") {
        "boolean-check"
    } else if problem.starts_with("cannot be measured") {
        "unmeasurable"
    } else if problem.contains("cannot be searched") {
        "self-check-error"
    } else {
        "check"
    }
}

impl Runner {
    pub fn new(setup: Setup, options: Options) -> Result<Runner, String> {
        let mut session = Session::new(Workspace::unrestricted());
        session
            .execute(
                "file.new",
                &json!({"units": "mm", "tolerance": setup.tolerance()}),
            )
            .map_err(|e| e.message().to_string())?;
        Ok(Runner {
            session,
            setup,
            options,
            ids: BTreeMap::new(),
            names: BTreeMap::new(),
            script: Vec::new(),
            report: Report {
                setup,
                groups: 0,
                accepted: 0,
                refused: Vec::new(),
                booleans: 0,
                identities_checked: 0,
                identities_skipped: 0,
                steps: BTreeMap::new(),
                cut_volumes: Tally::default(),
                cut_sides: Tally::default(),
                finding: None,
            },
        })
    }

    /// Feature id to name, of the features the document has now.
    pub fn names(&self) -> &BTreeMap<String, String> {
        &self.names
    }

    /// Runs a query command (`document.inspect`, `document.edges`): it
    /// changes nothing.
    pub fn query(&mut self, command: &str, params: &Value) -> Option<Value> {
        self.session.execute(command, params).ok()
    }

    /// The faces of `faces` that are flat: those a datum plane can be put
    /// on, tried on a copy of the document, so nothing of it reaches the
    /// design.
    pub fn flat_faces(&self, faces: &[String]) -> Vec<String> {
        let mut copy = self.session.clone();
        faces
            .iter()
            .filter(|face| {
                copy.execute("datum.plane", &json!({"from": face, "distance": 0}))
                    .is_ok_and(|r| r.get("status").and_then(Value::as_str) == Some("ok"))
            })
            .cloned()
            .collect()
    }

    fn refresh_names(&mut self) {
        self.ids.clear();
        self.names.clear();
        let Some(inspect) = self.query("document.inspect", &json!({})) else {
            return;
        };
        for f in inspect
            .get("features")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let (Some(id), Some(name)) = (
                f.get("id").and_then(Value::as_str),
                f.get("name").and_then(Value::as_str),
            ) {
                self.ids.insert(name.to_string(), id.to_string());
                self.names.insert(id.to_string(), name.to_string());
            }
        }
    }

    /// The first feature of the document that failed, with its reason.
    fn failed_feature(&mut self) -> Option<String> {
        let inspect = self.query("document.inspect", &json!({}))?;
        let failed = inspect
            .get("features")?
            .as_array()?
            .iter()
            .find(|f| f.get("status").and_then(Value::as_str) == Some("failed"))?;
        let name = failed.get("name").and_then(Value::as_str).unwrap_or("?");
        let error = failed.get("error").and_then(Value::as_str).unwrap_or("");
        Some(cut(&format!("{name} failed: {error}")))
    }

    fn found(&mut self, kind: &'static str, oracle: &str, command: &str, detail: &str) -> Applied {
        self.report.finding = Some(Finding {
            kind,
            oracle: oracle.to_string(),
            group: self.report.groups.saturating_sub(1),
            command: command.to_string(),
            detail: cut(detail),
        });
        Applied::Found
    }

    /// Applies one group and checks what it left. After [`Applied::Found`]
    /// the design is over: the report holds the finding.
    pub fn apply(&mut self, group: &Group) -> Applied {
        if self.report.finding.is_some() {
            return Applied::Found;
        }
        self.report.groups += 1;
        let index = self.report.groups - 1;
        self.report
            .steps
            .entry(group.step.clone())
            .or_default()
            .groups += 1;
        let before = self.session.clone();
        let script_len = self.script.len();
        // A plane cut's target measured before it, for its checks after.
        let cut = (group.main_command() == PLANE_CUT)
            .then(|| self.cut_before(group))
            .flatten();
        let is_boolean = group.main_command() == "feature.boolean";
        if is_boolean {
            self.report.booleans += 1;
            if self.options.identities
                && let Some(applied) = self.identities(group)
            {
                return applied;
            }
        }
        match run_commands(&mut self.session, &group.commands, &self.ids) {
            Err(Stop::Found(kind, command, detail)) => {
                return self.found(kind, oracle_of_bug(kind), &command, &detail);
            }
            Err(Stop::Refused(command, reason)) => {
                self.session = before;
                self.report.refused.push(Refusal {
                    group: index,
                    command,
                    reason,
                });
                return Applied::Refused;
            }
            Ok(done) => {
                for (command, params) in done {
                    self.script
                        .push(json!({"command": command, "params": params}));
                }
            }
        }
        // A feature that failed because of the group (an edit upstream of
        // it) is a refusal of the group as well.
        if let Some(reason) = self.failed_feature() {
            if let Some(kind) = bug_kind(&reason) {
                let command = group.main_command().to_string();
                return self.found(kind, oracle_of_bug(kind), &command, &reason);
            }
            self.session = before;
            self.script.truncate(script_len);
            self.report.refused.push(Refusal {
                group: index,
                command: group.main_command().to_string(),
                reason,
            });
            return Applied::Refused;
        }
        self.report.accepted += 1;
        self.report
            .steps
            .entry(group.step.clone())
            .or_default()
            .accepted += 1;
        self.refresh_names();
        let command = group.main_command().to_string();
        if self.session.regeneration_matches_a_full_one() == Some(false) {
            return self.found(
                "wrong",
                "regen",
                &command,
                "the regeneration edit by edit differs from a full one",
            );
        }
        let deep = self.options.deep == Deep::EveryGroup;
        let checked = self.check(deep, &command);
        match cut {
            Some(cut) if checked == Applied::Accepted => self.cut_checks(&cut, &before),
            _ => checked,
        }
    }

    /// The params of a plane cut group with ids filled in, and its target's
    /// volume and area now, before the group runs.
    fn cut_before(&mut self, group: &Group) -> Option<CutBefore> {
        let params = group
            .commands
            .last()
            .map(|c| map_strings(&c.params, &|s| resolved_text(s, &self.ids), 0))?;
        let target = params.get("target").and_then(Value::as_str)?.to_string();
        let measured = measure(&mut self.session, &target);
        Some(CutBefore {
            params,
            target,
            measured,
        })
    }

    /// The checks of an accepted plane cut, beyond those of every group:
    /// its body has no more volume than its target had, and, when it cuts
    /// by one plane, the same cut with the other side kept (on a copy of
    /// the document before the group) has the rest, so that the two add up
    /// to the target (a side that leaves nothing has none). Each within
    /// what a boolean may decide either way, `2 t (S(A) + S(result))` per
    /// result (`craft3d_kernel::boolean::verify`, rule 1: a cut is the
    /// boolean intersection of the target with a block of which only the
    /// top, the plane, meets it, and that face's piece is in the result),
    /// and the integrator's `1e-9` of the volume.
    fn cut_checks(&mut self, cut: &CutBefore, before: &Session) -> Applied {
        let t = self.setup.tolerance();
        let name = cut.params.get("name").and_then(Value::as_str).unwrap_or("");
        let made = self.ids.get(name).cloned();
        let after = made
            .as_deref()
            .and_then(|id| measure(&mut self.session, id));
        let (Some((va, sa)), Some((vc, sc))) = (cut.measured, after) else {
            self.report.cut_volumes.skipped += 1;
            return Applied::Accepted;
        };
        self.report.cut_volumes.checked += 1;
        let allowed = 2.0 * t * (sa + sc) + 1e-9 * va.abs();
        if !(vc.is_finite() && vc - va <= allowed) {
            let detail = format!(
                "the cut {name} has volume {vc:e}, its target {} had {va:e}: more by {:e}, allowed {allowed:e}",
                cut.target,
                vc - va
            );
            return self.found("wrong", "cut-volume", PLANE_CUT, &detail);
        }
        if !self.options.identities {
            return Applied::Accepted;
        }
        let Some(other) = other_side(&cut.params) else {
            return Applied::Accepted;
        };
        let mut side = before.clone();
        let rest = match side.execute(PLANE_CUT, &other) {
            Err(e) => Err(e.message().to_string()),
            Ok(result) if result.get("status").and_then(Value::as_str) == Some("failed") => {
                let error = result.get("error").and_then(Value::as_str).unwrap_or("");
                if error.contains("leaves nothing") {
                    Ok((0.0, 0.0))
                } else {
                    Err(error.to_string())
                }
            }
            Ok(_) => measure(&mut side, OTHER_SIDE).ok_or_else(String::new),
        };
        let (vo, so) = match rest {
            Ok(rest) => rest,
            Err(message) => {
                if let Some(kind) = bug_kind(&message) {
                    let detail = format!("the other side of the cut {name}: {message}");
                    return self.found(kind, oracle_of_bug(kind), PLANE_CUT, &detail);
                }
                self.report.cut_sides.skipped += 1;
                return Applied::Accepted;
            }
        };
        self.report.cut_sides.checked += 1;
        let allowed = 2.0 * t * (2.0 * sa + sc + so) + 1e-9 * va.abs();
        let gap = vc + vo - va;
        if gap.is_finite() && gap.abs() <= allowed {
            return Applied::Accepted;
        }
        let detail = format!(
            "the two sides of the cut {name} do not add up to its target {}: {vc:e} + {vo:e} − {va:e} = {gap:e}, allowed {allowed:e}",
            cut.target
        );
        self.found("wrong", "cut-sides", PLANE_CUT, &detail)
    }

    /// `document.check`; a finding for its first problem.
    fn check(&mut self, deep: bool, command: &str) -> Applied {
        let started = std::time::Instant::now();
        let result = self
            .session
            .execute("document.check", &json!({ "deep": deep }));
        trace("document.check", started);
        let result = match result {
            Ok(r) => r,
            Err(e) => {
                let message = e.message().to_string();
                let kind = bug_kind(&message).unwrap_or("wrong");
                return self.found(kind, "check-failed", "document.check", &message);
            }
        };
        let problems = result
            .get("problems")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let Some(first) = problems.first() else {
            return Applied::Accepted;
        };
        let text = first
            .get("problem")
            .and_then(Value::as_str)
            .unwrap_or("a problem without a text");
        let feature = first.get("name").and_then(Value::as_str).unwrap_or("?");
        let detail = format!(
            "{} problem(s); the first, of {feature}: {text}",
            problems.len()
        );
        let oracle = oracle_of(text);
        self.found("wrong", oracle, command, &detail)
    }

    /// The volume identities of a boolean group, on copies of the document
    /// before it: `V(A ∪ B) + V(A ∩ B) = V(A) + V(B)` and
    /// `V(A − B) + V(A ∩ B) = V(A)`, each within what the boolean may
    /// decide either way, `2 t (S(A) + S(B))` per result
    /// (`craft3d_kernel::boolean::verify`, rule 1), for the two results on
    /// a side, and the integrator's rounding. `Some` when it found
    /// something.
    fn identities(&mut self, group: &Group) -> Option<Applied> {
        let params = group
            .commands
            .last()
            .map(|c| map_strings(&c.params, &|s| resolved_text(s, &self.ids), 0))?;
        let (Some(target), Some(tool)) = (
            params.get("target").and_then(Value::as_str),
            params.get("tool").and_then(Value::as_str),
        ) else {
            // The boolean itself refuses such params; nothing to compare.
            self.report.identities_skipped += 1;
            return None;
        };
        let (target, tool) = (target.to_string(), tool.to_string());
        let mut base = self.session.clone();
        let (Some((va, sa)), Some((vb, sb))) =
            (measure(&mut base, &target), measure(&mut base, &tool))
        else {
            self.report.identities_skipped += 1;
            return None;
        };
        let mut volumes = BTreeMap::new();
        for op in ["union", "subtract", "intersect"] {
            let mut side = base.clone();
            let name = "__identity";
            let sibling = json!({"op": op, "target": target, "tool": tool, "name": name});
            match side.execute("feature.boolean", &sibling) {
                Err(e) => {
                    if let Some(kind) = bug_kind(e.message()) {
                        let detail = format!("the sibling {op} of the boolean: {}", e.message());
                        return Some(self.found(
                            kind,
                            oracle_of_bug(kind),
                            "feature.boolean",
                            &detail,
                        ));
                    }
                    self.report.identities_skipped += 1;
                    return None;
                }
                Ok(result) if result.get("status").and_then(Value::as_str) == Some("failed") => {
                    let error = result.get("error").and_then(Value::as_str).unwrap_or("");
                    if let Some(kind) = bug_kind(error) {
                        let detail = format!("the sibling {op} of the boolean: {error}");
                        return Some(self.found(
                            kind,
                            oracle_of_bug(kind),
                            "feature.boolean",
                            &detail,
                        ));
                    }
                    if error.contains("the result is empty") {
                        volumes.insert(op, 0.0);
                    } else {
                        self.report.identities_skipped += 1;
                        return None;
                    }
                }
                Ok(_) => {
                    let Some((v, _)) = measure(&mut side, name) else {
                        self.report.identities_skipped += 1;
                        return None;
                    };
                    volumes.insert(op, v);
                }
            }
        }
        self.report.identities_checked += 1;
        let get = |op: &str| volumes.get(op).copied().unwrap_or(0.0);
        let (u, s, i) = (get("union"), get("subtract"), get("intersect"));
        let operands = Operands { va, sa, vb, sb };
        let broken = identity_gaps(operands, [u, s, i], self.setup.tolerance())?;
        let detail = format!("{broken} (target {target}, tool {tool})");
        Some(self.found("wrong", "identity", "feature.boolean", &detail))
    }

    /// The checks at the end of a design: the deep check when it did not
    /// run after every group, and save, open and compare.
    pub fn finish(&mut self) {
        if self.report.finding.is_some() {
            return;
        }
        let end = self.report.groups;
        if self.options.deep == Deep::AtEnd {
            let _ = self.check(true, "document.check");
            if let Some(f) = &mut self.report.finding {
                f.group = end;
                return;
            }
        }
        if self.options.determinism {
            self.determinism(end);
        }
    }

    fn determinism(&mut self, end: usize) {
        let _ = std::fs::create_dir_all(&self.options.scratch);
        let path = self.options.scratch.join(format!(
            "design-{}-{}-{}.3dcraft",
            self.setup.seed,
            std::process::id(),
            SAVES.fetch_add(1, Ordering::Relaxed)
        ));
        let path_text = path.to_string_lossy().to_string();
        let Some(before) = self.query("document.inspect", &json!({})) else {
            return;
        };
        let saved = self
            .session
            .execute("file.save", &json!({"path": path_text, "overwrite": true}));
        let outcome = match saved {
            Err(e) => Some(format!("file.save failed: {}", e.message())),
            Ok(_) => {
                let mut other = Session::new(Workspace::unrestricted());
                match other.execute("file.open", &json!({ "path": path_text })) {
                    Err(e) => Some(format!("file.open failed: {}", e.message())),
                    Ok(_) => match other.execute("document.inspect", &json!({})) {
                        Err(e) => Some(format!("document.inspect failed: {}", e.message())),
                        Ok(after) => difference(&canonical(&before), &canonical(&after), "")
                            .map(|d| format!("the reopened document differs: {d}")),
                    },
                }
            }
        };
        let _ = std::fs::remove_file(&path);
        if let Some(detail) = outcome {
            let _ = self.found("wrong", "determinism", "file.open", &detail);
            if let Some(f) = &mut self.report.finding {
                f.group = end;
            }
        }
    }
}

/// The volumes and areas of a boolean's target and tool.
#[derive(Clone, Copy, Debug)]
pub struct Operands {
    pub va: f64,
    pub sa: f64,
    pub vb: f64,
    pub sb: f64,
}

/// What breaks `V(A ∪ B) + V(A ∩ B) = V(A) + V(B)` or
/// `V(A − B) + V(A ∩ B) = V(A)`, given the volumes of the union, the
/// difference and the intersection; `None` when both hold. The allowance:
/// each of the two results on a side may differ from the exact set by
/// what lies within the tolerance `t` of either operand's boundary,
/// `2 t (S(A) + S(B))` (`craft3d_kernel::boolean::verify`, rule 1), and
/// the integrator's sums by far less than `1e-9` of the volumes (they
/// converge to `1e-12` of what they sum, `craft3d_topo::mass_bound`).
pub fn identity_gaps(o: Operands, [u, s, i]: [f64; 3], t: f64) -> Option<String> {
    let Operands { va, sa, vb, sb } = o;
    let allowance = 2.0 * (2.0 * t * (sa + sb)) + 1e-9 * (va.abs() + vb.abs());
    let union_gap = (u + i) - (va + vb);
    let subtract_gap = (s + i) - va;
    let holds = |gap: f64| gap.is_finite() && gap.abs() <= allowance;
    if holds(union_gap) && holds(subtract_gap) {
        return None;
    }
    Some(format!(
        "V(A) {va:e}, V(B) {vb:e}, union {u:e}, subtract {s:e}, intersect {i:e}: \
         U + I − A − B = {union_gap:e}, S + I − A = {subtract_gap:e}, allowed {allowance:e}"
    ))
}

/// Where two JSON values first differ, as a path and the two values.
fn difference(a: &Value, b: &Value, at: &str) -> Option<String> {
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            for (k, v) in x {
                let here = format!("{at}.{k}");
                match y.get(k) {
                    Some(w) => {
                        if let Some(d) = difference(v, w, &here) {
                            return Some(d);
                        }
                    }
                    None => return Some(format!("{here} is gone")),
                }
            }
            y.keys()
                .find(|k| !x.contains_key(*k))
                .map(|k| format!("{at}.{k} is new"))
        }
        (Value::Array(x), Value::Array(y)) => {
            if x.len() != y.len() {
                return Some(format!("{at} has {} items, then {}", x.len(), y.len()));
            }
            x.iter()
                .zip(y)
                .enumerate()
                .find_map(|(i, (v, w))| difference(v, w, &format!("{at}[{i}]")))
        }
        _ if a == b => None,
        _ => Some(format!(
            "{at}: {} then {}",
            cut(&a.to_string()),
            cut(&b.to_string())
        )),
    }
}

/// Runs a whole design (a reproducer) and its end checks.
pub fn run_design(design: &Design, options: Options) -> Result<Runner, String> {
    let mut runner = Runner::new(design.setup, options)?;
    for group in &design.groups {
        if runner.apply(group) == Applied::Found {
            break;
        }
    }
    runner.finish();
    Ok(runner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refusal_keys_drop_numbers_and_details() {
        let r = Refusal {
            group: 0,
            command: "feature.fillet".into(),
            reason: "radius: too large for edges.0 (between F1:face(1) and F1:face(2)): with a radius of 20 mm".into(),
        };
        assert_eq!(
            r.key(),
            "feature.fillet: radius: too large for edges.# : with a radius of # mm"
        );
    }

    #[test]
    fn the_identities_catch_a_wrong_volume() {
        // Two unit cubes overlapping by half: A ∪ B 1.5, A − B 0.5, A ∩ B 0.5.
        let o = Operands {
            va: 1.0,
            sa: 6.0,
            vb: 1.0,
            sb: 6.0,
        };
        let t = 1e-6;
        assert_eq!(identity_gaps(o, [1.5, 0.5, 0.5], t), None);
        // Off by what the tolerance band allows: still right.
        assert_eq!(identity_gaps(o, [1.5 + 4e-5, 0.5, 0.5], t), None);
        // A union that lost a sliver, a difference that kept the tool, a NaN.
        assert!(identity_gaps(o, [1.49, 0.5, 0.5], t).is_some());
        assert!(identity_gaps(o, [1.5, 1.0, 0.5], t).is_some());
        assert!(identity_gaps(o, [1.5, f64::NAN, 0.5], t).is_some());
    }

    #[test]
    fn problems_go_to_their_oracle() {
        assert_eq!(oracle_of("invalid body: an edge is used once"), "topology");
        assert_eq!(
            oracle_of("the body passes through or touches itself: 2 crossings"),
            "self-intersection"
        );
        assert_eq!(
            oracle_of("not the union of its target and tool: x"),
            "boolean-check"
        );
        assert_eq!(oracle_of("cannot be measured: open"), "unmeasurable");
    }

    #[test]
    fn the_other_side_of_a_cut_by_one_plane() {
        let one = json!({"target": "F1", "planes": [{"point": [0, 0, 1], "normal": [0, 0, 1]}], "name": "N2"});
        let other = other_side(&one).unwrap();
        assert_eq!(
            other["planes"],
            json!([{"point": [0, 0, 1], "normal": [0, 0, 1], "side": "above"}])
        );
        assert_eq!(other["keep"], true);
        assert_eq!(other["name"], OTHER_SIDE);
        assert_eq!(other["target"], "F1");
        // Above turns below; a plane given alone, or as a name, becomes an
        // object in a list.
        let alone = json!({"target": "F1", "planes": {"plane": "xy", "side": "above"}});
        assert_eq!(
            other_side(&alone).unwrap()["planes"],
            json!([{"plane": "xy", "side": "below"}])
        );
        let named = json!({"target": "F1", "planes": "F1:face(1)"});
        assert_eq!(
            other_side(&named).unwrap()["planes"],
            json!([{"plane": "F1:face(1)", "side": "above"}])
        );
        // Several planes have no one other side.
        assert_eq!(
            other_side(&json!({"target": "F1", "planes": ["xy", "yz"]})),
            None
        );
        assert_eq!(other_side(&json!({"target": "F1"})), None);
    }

    #[test]
    fn differences_are_found_with_their_path() {
        let a = json!({"x": [1, {"y": 2.0}]});
        assert_eq!(difference(&a, &a, ""), None);
        let b = json!({"x": [1, {"y": 2.5}]});
        assert_eq!(
            difference(&a, &b, "").as_deref(),
            Some(".x[1].y: 2.0 then 2.5")
        );
    }
}
