//! The Parameters tab beside the feature list: the form of a Jig
//! (docs/decisions.md, 2026-10-23; `docs/guide`, *Jigs: parameters*).
//!
//! What it shows is `param.list`, read again whenever the active
//! document's revision changes; what it does is `param.set`, handed to the
//! host as a [`ShellCommand`] so that the regeneration runs where every
//! command runs (the desktop's engine thread, the page's one thread after
//! a frame that says it is busy). The form clamps a value to its range
//! and snaps it to its step before it sends it, and says so under the
//! field; the command refuses what is out of range.
//!
//! A drag commits as it goes on the desktop, at most once per
//! [`DEBOUNCE`] seconds, every commit after the first folded into the same
//! undo step (`merge`); on the web, where a regeneration blocks the page,
//! only when the drag ends. A value that makes a feature fail or an
//! assertion break marks the field and keeps the last good model in the
//! viewport until the model is good again ([`Verdict`]).
//!
//! The logic is here without egui where it can be ([`settle`],
//! [`Pacer`], [`verdict`], [`ParamList::parse`]), so tests can ask for it.

use super::Shell;
use super::number_field::{self, FieldMemory, NumberField, Typing};
use crate::theme::{color, color_alpha};
use craft3d_engine::{FeatureState, Scene, Session};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::sync::Arc;

/// Least time between two commits of one drag on the desktop, in seconds.
pub const DEBOUNCE: f64 = 0.15;
/// How long the note under a clamped field stays, in seconds.
pub const NOTE_SECONDS: f64 = 4.0;
/// Most parameters the form shows; a file with more is cut there.
pub const MAX_PARAMS: usize = 512;
/// Longest text the form keeps of a label, a message or an info field.
const MAX_TEXT: usize = 2000;
/// Most decimals a value is shown with.
const MAX_DECIMALS: usize = 6;
/// The field id every parameter's number field shares: each field lives
/// under an egui id of its own (`push_id`), and the form remembers which
/// one the drag or the typing belongs to ([`ParamForm::active`]).
const FIELD: &str = "param";
/// What a document without public parameters says about the tab.
const NO_PARAMETERS: &str =
    "No parameters. Publish a variable (variable.publish) to make this document a Jig.";

/// What kind of value a parameter holds (`param.list` `kind`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ParamKind {
    #[default]
    Number,
    Integer,
    Boolean,
    Choice,
}

impl ParamKind {
    fn parse(name: &str) -> Self {
        match name {
            "integer" => Self::Integer,
            "boolean" => Self::Boolean,
            "choice" => Self::Choice,
            _ => Self::Number,
        }
    }

    /// Integer, boolean and choice hold whole numbers.
    pub fn whole(self) -> bool {
        self != Self::Number
    }
}

/// One public parameter as `param.list` gives it, in the unit it is shown
/// in.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Param {
    pub name: String,
    pub label: String,
    pub description: String,
    pub group: String,
    pub kind: ParamKind,
    pub choices: Vec<String>,
    /// Its expression (`40 mm`).
    pub expr: String,
    /// Its value in `unit`; none when it does not evaluate.
    pub value: Option<f64>,
    /// Why it does not evaluate.
    pub error: Option<String>,
    /// The unit it is shown in (`mm`, `deg`), or none for a plain number.
    pub unit: Option<String>,
    /// The units of another dimension (an area: `mm^2`), which a form
    /// cannot send as a bare number.
    pub unit_text: Option<String>,
    /// The default's expression and whether the value is it.
    pub default_expr: String,
    pub at_default: bool,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub step: Option<f64>,
    pub valid: bool,
    pub problems: Vec<String>,
}

impl Param {
    /// The unit as shown after a value: its own, or that of another
    /// dimension, or nothing.
    pub fn unit_shown(&self) -> &str {
        self.unit
            .as_deref()
            .or(self.unit_text.as_deref())
            .unwrap_or("")
    }

    /// Whether the form can send a number for it: not for a dimension
    /// without a unit of its own (an area), which `param.set` takes only
    /// as a value with its units.
    pub fn takes_numbers(&self) -> bool {
        self.unit_text.is_none()
    }

    /// The range, when it has both ends and they make one.
    pub fn range(&self) -> Option<[f64; 2]> {
        match (self.min, self.max) {
            (Some(lo), Some(hi)) if lo.is_finite() && hi.is_finite() && hi > lo => Some([lo, hi]),
            _ => None,
        }
    }

    /// The step of the arrows and of Ctrl while dragging: its own, else
    /// one for its range, at least 1 for a whole number.
    pub fn field_step(&self, range: [f64; 2]) -> f64 {
        let step = self
            .step
            .filter(|s| s.is_finite() && *s > 0.0)
            .unwrap_or_else(|| number_field::nice_step(range));
        if self.kind.whole() {
            step.round().max(1.0)
        } else {
            step
        }
    }

    /// `value` written with its unit, as few decimals as it needs.
    pub fn written(&self, value: f64) -> String {
        let decimals = if self.kind.whole() {
            0
        } else {
            decimals_of(value)
        };
        number_field::shown(value, 1.0, decimals, self.unit_shown())
    }

    /// The label of choice `value` (1 for the first).
    pub fn choice_label(&self, value: f64) -> Option<&str> {
        let index = whole_index(value)?;
        self.choices.get(index.checked_sub(1)?).map(String::as_str)
    }
}

/// `value` as a whole number from 0, when it is one.
fn whole_index(value: f64) -> Option<usize> {
    if !value.is_finite() || (value - value.round()).abs() > 1e-9 || value < 0.0 {
        return None;
    }
    let rounded = value.round();
    if rounded > 1e15 {
        return None;
    }
    Some(rounded as usize)
}

/// The fewest decimals (up to [`MAX_DECIMALS`]) that write `value` as it
/// is, within rounding.
pub fn decimals_of(value: f64) -> usize {
    if !value.is_finite() {
        return 0;
    }
    for decimals in 0..MAX_DECIMALS {
        let scale = 10f64.powi(decimals as i32);
        let scaled = value * scale;
        if (scaled - scaled.round()).abs() <= 1e-6 * scale.max(1.0) * value.abs().max(1.0) / scale {
            return decimals;
        }
    }
    MAX_DECIMALS
}

/// What a gallery shows of the document (`param.list` `info`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DocInfo {
    pub title: String,
    pub description: String,
    pub author: String,
    pub licence: String,
    pub url: String,
}

/// `param.list` as the form reads it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParamList {
    pub jig: bool,
    pub groups: Vec<String>,
    pub params: Vec<Param>,
    pub info: DocInfo,
}

fn text(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_str)
        .map(|s| s.chars().take(MAX_TEXT).collect())
        .unwrap_or_default()
}

fn number(value: Option<&Value>) -> Option<f64> {
    value.and_then(Value::as_f64).filter(|v| v.is_finite())
}

/// A bound's value: `{expr, value}` gives it, `{expr, error}` or null none.
fn bound(value: Option<&Value>) -> Option<f64> {
    number(value?.get("value"))
}

impl ParamList {
    /// Reads the result of `param.list`. Never panics: what is missing or
    /// of the wrong type is left empty.
    pub fn parse(value: &Value) -> Self {
        let params: Vec<Param> = value
            .get("parameters")
            .and_then(Value::as_array)
            .map(|all| {
                all.iter()
                    .take(MAX_PARAMS)
                    .map(|p| {
                        let get = |key: &str| p.get(key);
                        Param {
                            name: text(get("name")),
                            label: text(get("label")),
                            description: text(get("description")),
                            group: text(get("group")),
                            kind: ParamKind::parse(&text(get("kind"))),
                            choices: get("choices")
                                .and_then(Value::as_array)
                                .map(|c| c.iter().take(MAX_PARAMS).map(|c| text(Some(c))).collect())
                                .unwrap_or_default(),
                            expr: text(get("expr")),
                            value: number(get("value")),
                            error: get("error").map(|e| text(Some(e))),
                            unit: get("unit").and_then(Value::as_str).map(str::to_string),
                            unit_text: get("unitText").and_then(Value::as_str).map(str::to_string),
                            default_expr: text(get("default").and_then(|d| d.get("expr"))),
                            at_default: get("atDefault").and_then(Value::as_bool).unwrap_or(true),
                            min: bound(get("min")),
                            max: bound(get("max")),
                            step: bound(get("step")).filter(|s| *s > 0.0),
                            valid: get("valid").and_then(Value::as_bool).unwrap_or(true),
                            problems: get("problems")
                                .and_then(Value::as_array)
                                .map(|all| all.iter().take(16).map(|p| text(Some(p))).collect())
                                .unwrap_or_default(),
                        }
                    })
                    .filter(|p| !p.name.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        let mut groups: Vec<String> = Vec::new();
        for p in &params {
            if !groups.contains(&p.group) {
                groups.push(p.group.clone());
            }
        }
        let info = value.get("info");
        let field = |key: &str| text(info.and_then(|i| i.get(key)));
        Self {
            jig: !params.is_empty(),
            groups,
            params,
            info: DocInfo {
                title: field("title"),
                description: field("description"),
                author: field("author"),
                licence: field("licence"),
                url: field("url"),
            },
        }
    }

    pub fn param(&self, name: &str) -> Option<&Param> {
        self.params.iter().find(|p| p.name == name)
    }
}

// ------------------------------------------------------------- clamping

/// A value as the form sends it, and what it says about it.
#[derive(Clone, Debug, PartialEq)]
pub struct Settled {
    pub value: f64,
    /// `clamped to 120 mm`, `rounded to 12.5 mm`: shown under the field
    /// for [`NOTE_SECONDS`]; none when the value is as asked.
    pub note: Option<String>,
}

/// `wanted` made a value `p` takes: a whole number for a whole kind (0 or
/// 1 for a boolean, 1 to the number of choices for a choice), on its step
/// (counted from its minimum), within its range. None for a value that is
/// not a number.
pub fn settle(p: &Param, wanted: f64) -> Option<Settled> {
    if !wanted.is_finite() {
        return None;
    }
    let mut value = wanted;
    let mut rounded = false;
    if let Some(step) = p.step.filter(|s| s.is_finite() && *s > 0.0) {
        let base = p.min.unwrap_or(0.0);
        let snapped = base + ((value - base) / step).round() * step;
        if snapped.is_finite() {
            // Keep the decimals the step has: 0.1 * 3 is 0.30000000000000004.
            let decimals = decimals_of(step).max(decimals_of(base));
            let scale = 10f64.powi(decimals as i32);
            let tidy = (snapped * scale).round() / scale;
            value = if tidy.is_finite() { tidy } else { snapped };
        }
    }
    if p.kind.whole() {
        value = value.round();
    }
    if !same(value, wanted) {
        rounded = true;
    }
    let (mut lo, mut hi) = (p.min, p.max);
    match p.kind {
        ParamKind::Boolean => {
            lo = Some(lo.map_or(0.0, |l| l.max(0.0)));
            hi = Some(hi.map_or(1.0, |h| h.min(1.0)));
        }
        ParamKind::Choice => {
            lo = Some(lo.map_or(1.0, |l| l.max(1.0)));
            let count = p.choices.len().max(1) as f64;
            hi = Some(hi.map_or(count, |h| h.min(count)));
        }
        ParamKind::Number | ParamKind::Integer => {}
    }
    let before = value;
    if let Some(lo) = lo.filter(|l| l.is_finite())
        && value < lo
    {
        value = if p.kind.whole() { lo.ceil() } else { lo };
    }
    if let Some(hi) = hi.filter(|h| h.is_finite())
        && value > hi
    {
        value = if p.kind.whole() { hi.floor() } else { hi };
    }
    let clamped = !same(value, before);
    let note = if clamped {
        Some(format!("clamped to {}", p.written(value)))
    } else if rounded {
        Some(format!("rounded to {}", p.written(value)))
    } else {
        None
    };
    Some(Settled { value, note })
}

fn same(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0)
}

/// The JSON value `param.set` takes for `value` of `p`: a whole number for
/// a whole kind, else a number in its unit.
pub fn sent_value(p: &Param, value: f64) -> Value {
    if p.kind.whole() && value.abs() < 1e15 {
        json!(value.round() as i64)
    } else {
        json!(value)
    }
}

// ---------------------------------------------------------------- pacing

/// A field being changed: the value it shows, and how its commits go.
#[derive(Clone, Debug, PartialEq)]
pub struct Live {
    pub name: String,
    pub value: f64,
    /// The range the drag spans: the parameter's, or one around the value
    /// where the drag began when it has none (fixed for the whole drag).
    pub range: [f64; 2],
    /// The drag is under way.
    pub dragging: bool,
    /// A value that was not committed yet.
    pub dirty: bool,
    /// The drag has committed already: the next commit folds into its
    /// undo step.
    pub committed: bool,
    /// When the last commit was sent (or the drag began), in seconds of
    /// `egui::InputState::time`.
    pub last_sent: f64,
}

/// When the value of a [`Live`] field is committed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pacer {
    /// The host has one thread (the web): a drag commits when it ends.
    pub single_thread: bool,
}

impl Pacer {
    /// Whether `live` is committed now: not while a commit is under way
    /// (`busy`); at once once the drag ended or for a change that was no
    /// drag; during a drag at most once per [`DEBOUNCE`] on the desktop,
    /// never on the web.
    pub fn due(self, live: &Live, now: f64, busy: bool) -> bool {
        if !live.dirty || busy {
            return false;
        }
        if !live.dragging {
            return true;
        }
        !self.single_thread && now - live.last_sent >= DEBOUNCE
    }
}

// --------------------------------------------------------------- results

/// A command the shell asks its host to run on the session, where the
/// host runs commands (the desktop's engine thread), and to report back
/// with [`Shell::command_done`].
#[derive(Clone, Debug, PartialEq)]
pub struct ShellCommand {
    pub command: &'static str,
    pub params: Value,
}

/// How many features failed and assertions were broken in a state the
/// form took as good: a commit that ends with more is a failure.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Baseline {
    pub features: usize,
    pub assertions: usize,
}

/// What a commit did to the model.
#[derive(Clone, Debug, PartialEq)]
pub enum Verdict {
    /// The model is as good as before.
    Good,
    /// `param.set` refused the value: nothing changed.
    Refused(String),
    /// The value was set and broke the model: a feature fails or an
    /// assertion is broken that was not before.
    Broke(String),
}

/// The verdict on a commit's `result`, the features that fail after it
/// (`(name, error)`), against the `baseline` of the last good state.
pub fn verdict(
    result: &Result<Value, String>,
    failing: &[(String, String)],
    baseline: Baseline,
) -> Verdict {
    let value = match result {
        Err(error) => return Verdict::Refused(error.clone()),
        Ok(value) => value,
    };
    let new_failures: Vec<String> = value
        .get("failed")
        .and_then(Value::as_array)
        .map(|all| {
            all.iter()
                .map(|f| {
                    let name = text(f.get("feature"));
                    let error = text(f.get("error"));
                    if name.is_empty() {
                        error
                    } else {
                        format!("{name}: {error}")
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    let broken: Vec<String> = value
        .get("assertions")
        .and_then(|a| a.get("broken"))
        .and_then(Value::as_array)
        .map(|all| all.iter().map(|b| text(b.get("message"))).collect())
        .unwrap_or_default();
    if let Some(first) = new_failures.first() {
        return Verdict::Broke(first.clone());
    }
    if failing.len() > baseline.features
        && let Some((name, error)) = failing.first()
    {
        return Verdict::Broke(format!("{name}: {error}"));
    }
    if broken.len() > baseline.assertions
        && let Some(message) = broken.first()
    {
        return Verdict::Broke(message.clone());
    }
    Verdict::Good
}

/// What went wrong with the last commit, at the field it came from.
#[derive(Clone, Debug, PartialEq)]
pub struct Trouble {
    /// The parameter, or none for Reset all and Back to last good.
    pub name: Option<String>,
    pub message: String,
    /// The model broke (the viewport keeps the last good one); false when
    /// the command refused the value and nothing changed.
    pub broke: bool,
}

/// The last state the form took as good.
#[derive(Clone)]
struct LastGood {
    scene: Option<Arc<Scene>>,
    /// The parameters' expressions then: Back to last good sets them.
    values: Map<String, Value>,
}

/// A note under a field.
#[derive(Clone, Debug, PartialEq)]
pub struct Note {
    pub name: String,
    pub text: String,
    /// Until when it shows, in seconds of the input's time; none until
    /// the field was drawn with it (a note set by `ui.param`).
    pub until: Option<f64>,
}

/// A commit handed to the host, until it is taken.
#[derive(Clone, Debug)]
struct Pending {
    command: ShellCommand,
    /// The form was drawn saying it is busy since it was asked: on the
    /// web the page shows that before the regeneration blocks it.
    drawn: bool,
    field: Option<String>,
}

/// The form's memory between frames ([`Shell`] holds one).
#[derive(Default)]
pub struct ParamForm {
    /// `param.list` of the active document, and for which document and
    /// revision it was read.
    list: Option<ParamList>,
    read_for: Option<(u64, u64)>,
    /// The tab each document was last shown on (true: Parameters).
    tab_of: BTreeMap<u64, bool>,
    /// The document the form shows.
    doc: Option<u64>,
    live: Option<Live>,
    pending: Option<Pending>,
    /// A commit the host took and has not reported yet: its field.
    in_flight: Option<Option<String>>,
    /// The result of the last commit, waiting for the next sync.
    done: Option<(Option<String>, Result<Value, String>)>,
    note: Option<Note>,
    trouble: Option<Trouble>,
    last_good: Option<LastGood>,
    baseline: Baseline,
    /// The drag and focus of the number fields, and the field typed into,
    /// with which parameter they belong to.
    memory: FieldMemory,
    typing: Option<Typing>,
    active: Option<String>,
    pacer: Option<Pacer>,
    /// Where the last frame drew each parameter's widget, and Reset all.
    hits: Vec<(String, egui::Rect)>,
}

impl ParamForm {
    fn pacer(&self) -> Pacer {
        self.pacer.unwrap_or(Pacer {
            single_thread: cfg!(target_arch = "wasm32"),
        })
    }

    fn busy(&self) -> bool {
        self.pending.is_some() || self.in_flight.is_some()
    }

    /// The value field `name` shows: what is being dragged or typed, else
    /// the document's.
    fn shown_value(&self, p: &Param) -> Option<f64> {
        match &self.live {
            Some(live) if live.name == p.name => Some(live.value),
            _ => p.value,
        }
    }

    /// Asks the host for `param.set` with `params`. A commit that waits
    /// already is joined: its values and these, merged only when both
    /// were.
    fn ask(&mut self, params: Value, field: Option<String>) {
        if let (Some(pending), Some(new_values)) = (
            self.pending.as_mut(),
            params.get("values").and_then(Value::as_object),
        ) && params.get("reset").is_none()
            && pending.command.params.get("reset").is_none()
        {
            let merge = pending
                .command
                .params
                .get("merge")
                .and_then(Value::as_bool)
                .unwrap_or(false)
                && params
                    .get("merge")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
            if let Some(Value::Object(values)) = pending.command.params.get_mut("values") {
                for (k, v) in new_values {
                    values.insert(k.clone(), v.clone());
                }
            }
            if let Some(map) = pending.command.params.as_object_mut() {
                map.insert("merge".into(), merge.into());
            }
            if pending.field != field {
                pending.field = None;
            }
            return;
        }
        self.pending = Some(Pending {
            command: ShellCommand {
                command: "param.set",
                params,
            },
            drawn: false,
            field,
        });
    }

    /// Commits `value` of `p`: one `param.set`, folded into the last undo
    /// step with `merge`.
    fn commit(&mut self, p: &Param, value: f64, merge: bool) {
        let mut values = Map::new();
        values.insert(p.name.clone(), sent_value(p, value));
        self.ask(
            json!({ "values": values, "merge": merge }),
            Some(p.name.clone()),
        );
    }

    /// Says `note` under field `name`.
    fn say(&mut self, name: &str, note: Option<String>) {
        if let Some(text) = note {
            self.note = Some(Note {
                name: name.to_string(),
                text,
                until: None,
            });
        }
    }

    /// Commits the live value when it is due.
    fn pace(&mut self, now: f64) {
        let busy = self.busy();
        let pacer = self.pacer();
        let Some(live) = self.live.as_mut() else {
            return;
        };
        if !pacer.due(live, now, busy) {
            return;
        }
        let merge = live.committed;
        live.committed = true;
        live.dirty = false;
        live.last_sent = now;
        let (name, value) = (live.name.clone(), live.value);
        if let Some(p) = self.list.as_ref().and_then(|l| l.param(&name)).cloned() {
            self.commit(&p, value, merge);
        }
    }
}

/// A value typed into a parameter's field: a number in its unit, or the
/// number with that unit after it. None for anything else (an expression,
/// another unit), which needs the document's units.
pub fn typed_number(p: &Param, text: &str) -> Option<f64> {
    let text = text.trim();
    let bare = match p.unit.as_deref() {
        Some(unit) => text.strip_suffix(unit).map_or(text, str::trim_end),
        None => text,
    };
    bare.parse::<f64>().ok().filter(|v| v.is_finite())
}

impl Shell {
    /// The command the form asks the host to run, once the form was drawn
    /// saying it is busy: the host runs it where it runs commands and
    /// hands the result to [`Self::command_done`].
    pub fn take_command(&mut self) -> Option<ShellCommand> {
        if !self.params.pending.as_ref().is_some_and(|p| p.drawn) {
            return None;
        }
        let pending = self.params.pending.take()?;
        self.params.in_flight = Some(pending.field);
        Some(pending.command)
    }

    /// The result of a command from [`Self::take_command`]: judged at the
    /// next [`Self::sync`], against the model it leaves.
    pub fn command_done(&mut self, result: Result<Value, String>) {
        let field = self.params.in_flight.take().flatten();
        self.params.done = Some((field, result));
    }

    /// A commit asked for before this frame is drawn in it as busy: the
    /// host may take it after the frame ([`Self::take_command`]). Called
    /// as a frame begins.
    pub(super) fn mark_pending_drawn(&mut self, ctx: &egui::Context) {
        if let Some(pending) = self.params.pending.as_mut() {
            pending.drawn = true;
            ctx.request_repaint();
        }
    }

    /// A commit waits for the host.
    pub(super) fn command_waiting(&self) -> bool {
        self.params.pending.is_some()
    }

    /// Where the last frame drew each parameter's widget (by its name),
    /// and the buttons `Reset all` and `Back to last good` (by those
    /// names): for tests that drive the form with the mouse.
    pub fn parameter_hits(&self) -> &[(String, egui::Rect)] {
        &self.params.hits
    }

    /// Commits the drag of a parameter at most once per [`DEBOUNCE`]
    /// (false, the desktop) or only when it ends (true, the web, where the
    /// page has one thread). Chosen by the target; tests set it.
    pub fn set_parameters_single_thread(&mut self, single_thread: bool) {
        self.params.pacer = Some(Pacer { single_thread });
    }

    /// True while the form waits for a commit to be run or reported.
    pub fn parameters_busy(&self) -> bool {
        self.params.busy()
    }

    /// The scene the viewport draws instead of the active document's, while
    /// the last commit broke the model: the last good one.
    pub(super) fn held_scene(&self) -> Option<Arc<Scene>> {
        let trouble = self.params.trouble.as_ref()?;
        if !trouble.broke {
            return None;
        }
        self.params.last_good.as_ref()?.scene.clone()
    }

    /// Reads `param.list` again when the active document or its revision
    /// changed, judges a commit that came back, and keeps the last good
    /// state. Part of every sync.
    pub(super) fn sync_params(&mut self, session: &mut Session, scene: Option<&Arc<Scene>>) {
        let doc = session.document_status().map(|s| s.id);
        let form = &mut self.params;
        if doc != form.doc {
            if let Some(old) = form.doc {
                form.tab_of.insert(old, self.state.parameters.open);
            }
            form.doc = doc;
            form.list = None;
            form.read_for = None;
            form.live = None;
            form.pending = None;
            form.note = None;
            form.trouble = None;
            form.last_good = None;
            form.memory = FieldMemory::default();
            form.typing = None;
            form.active = None;
        }
        let Some(doc) = doc else {
            return;
        };
        let revision = scene.map_or(0, |s| s.revision);
        let changed = form.read_for != Some((doc, revision));
        if changed || form.list.is_none() {
            let list = session
                .execute("param.list", &json!({}))
                .map(|v| ParamList::parse(&v))
                .unwrap_or_default();
            if form.list.is_none() && !form.tab_of.contains_key(&doc) {
                // A Jig opens on its parameters.
                form.tab_of.insert(doc, list.jig);
            }
            if form.read_for.is_none_or(|(d, _)| d != doc) {
                self.state.parameters.open = form.tab_of.get(&doc).copied().unwrap_or(list.jig);
            }
            form.list = Some(list);
            form.read_for = Some((doc, revision));
            // The value the field showed during the drag is the
            // document's now.
            if form.live.as_ref().is_some_and(|l| !l.dragging && !l.dirty) && !form.busy() {
                form.live = None;
            }
        }
        let failing: Vec<(String, String)> = self
            .rows
            .iter()
            .filter(|r| r.state == FeatureState::Failed)
            .map(|r| (r.name.clone(), r.error.clone().unwrap_or_default()))
            .collect();
        let broken_now = self
            .assertion_rows
            .iter()
            .filter(|a| {
                use craft3d_engine::AssertionStatus as A;
                matches!(a.status, A::Failed | A::Error | A::Pending)
            })
            .count();
        let form = &mut self.params;
        if let Some((field, result)) = form.done.take() {
            match verdict(&result, &failing, form.baseline) {
                Verdict::Good => {
                    form.trouble = None;
                    form.baseline = Baseline {
                        features: failing.len(),
                        assertions: result
                            .as_ref()
                            .ok()
                            .and_then(|v| v.get("assertions"))
                            .and_then(|a| a.get("broken"))
                            .and_then(Value::as_array)
                            .map_or(0, Vec::len),
                    };
                }
                Verdict::Refused(message) => {
                    form.trouble = Some(Trouble {
                        name: field,
                        message,
                        broke: false,
                    });
                }
                Verdict::Broke(message) => {
                    form.trouble = Some(Trouble {
                        name: field,
                        message,
                        broke: true,
                    });
                }
            }
        } else if changed
            && !form.busy()
            && form.trouble.as_ref().is_some_and(|t| t.broke)
            && failing.len() <= form.baseline.features
        {
            // Mended elsewhere (an undo, an agent): good again.
            form.trouble = None;
        } else if changed && !form.busy() && form.trouble.as_ref().is_some_and(|t| !t.broke) {
            form.trouble = None;
        }
        if form.trouble.is_none() && !form.busy() {
            if changed {
                form.baseline = Baseline {
                    features: failing.len(),
                    assertions: broken_now,
                };
            }
            let values: Map<String, Value> = form
                .list
                .iter()
                .flat_map(|l| &l.params)
                .map(|p| (p.name.clone(), Value::String(p.expr.clone())))
                .collect();
            form.last_good = Some(LastGood {
                scene: scene.cloned(),
                values,
            });
        }
    }

    /// `ui.get`'s `parameters`: what the form shows besides the values
    /// (which are `param.list`'s).
    pub(crate) fn parameters_report(&self) -> Value {
        let form = &self.params;
        let list = form.list.as_ref();
        json!({
            "jig": list.is_some_and(|l| l.jig),
            "tab": if self.state.parameters.open && list.is_some_and(|l| l.jig) {
                "parameters"
            } else {
                "features"
            },
            "count": list.map_or(0, |l| l.params.len()),
            "busy": form.busy(),
            "live": form.live.as_ref().map(|l| json!({
                "name": l.name, "value": l.value, "dragging": l.dragging,
            })),
            "note": form.note.as_ref().map(|n| json!({"name": n.name, "text": n.text})),
            "trouble": form.trouble.as_ref().map(|t| json!({
                "name": t.name, "message": t.message, "holding": t.broke && self.held_scene().is_some(),
            })),
        })
    }

    /// `ui.param`: sets one parameter as the form does, clamped to its
    /// range and snapped to its step (`note` says so), and runs it at once
    /// on `session`. `value` is a number in its unit, true or false, or a
    /// choice's label.
    pub(crate) fn ui_param(
        &mut self,
        session: &mut Session,
        params: Option<&Value>,
    ) -> Result<Value, String> {
        let method = "ui.param";
        let map = match params {
            Some(Value::Object(map)) => map,
            _ => {
                return Err(format!(
                    "{method}: give {{\"name\": string, \"value\": ...}}"
                ));
            }
        };
        if let Some(unknown) = map.keys().find(|k| *k != "name" && *k != "value") {
            let unknown: String = unknown.chars().take(40).collect();
            return Err(format!(
                "{method}: unknown parameter `{unknown}`; its parameters are: name, value"
            ));
        }
        let Some(name) = map.get("name").and_then(Value::as_str) else {
            return Err(format!("{method}: `name` must be a parameter's name"));
        };
        self.sync(session);
        let Some(p) = self
            .params
            .list
            .as_ref()
            .and_then(|l| l.param(name))
            .cloned()
        else {
            let known: Vec<String> = self
                .params
                .list
                .iter()
                .flat_map(|l| &l.params)
                .map(|p| p.name.clone())
                .collect();
            let name: String = name.chars().take(60).collect();
            return Err(format!(
                "{method}: `{name}` is not a public parameter of the active document; the parameters are: {}",
                known.join(", ")
            ));
        };
        let wanted = match map.get("value") {
            Some(Value::Number(n)) => n.as_f64(),
            Some(Value::Bool(b)) => Some(if *b { 1.0 } else { 0.0 }),
            Some(Value::String(s)) => p
                .choices
                .iter()
                .position(|c| c == s)
                .map(|i| (i + 1) as f64)
                .or_else(|| typed_number(&p, s)),
            _ => None,
        };
        let Some(settled) = wanted.and_then(|v| settle(&p, v)) else {
            return Err(format!(
                "{method}: `value` must be a number in the parameter's unit, true or false, or a choice's label"
            ));
        };
        if !p.takes_numbers() {
            return Err(format!(
                "{method}: `{}` takes a value with its units; set it with param.set",
                p.name
            ));
        }
        self.params.say(&p.name, settled.note.clone());
        self.params.pending = None;
        self.params.commit(&p, settled.value, false);
        let pending = self.params.pending.take();
        let mut result = Value::Null;
        if let Some(pending) = pending {
            self.params.in_flight = Some(pending.field);
            let outcome = session
                .execute(pending.command.command, &pending.command.params)
                .map_err(|e| e.message().to_string());
            result = match &outcome {
                Ok(v) => v.clone(),
                Err(e) => json!({ "error": e }),
            };
            self.command_done(outcome);
            self.sync(session);
        }
        Ok(json!({
            "sent": sent_value(&p, settled.value),
            "note": settled.note,
            "result": result,
            "parameters": self.parameters_report(),
        }))
    }

    /// The header of the side panel: the Parameters and Features tabs
    /// when a document is open, then the tab that is chosen.
    pub(super) fn side_panel(&mut self, ui: &mut egui::Ui, has_document: bool) {
        let tokens = self.tokens;
        ui.add_space(tokens.inset * 0.5);
        let jig = has_document && self.params.list.as_ref().is_some_and(|l| l.jig);
        if jig {
            ui.horizontal(|ui| {
                let open = self.state.parameters.open;
                let mut label = egui::RichText::new("Parameters");
                if open {
                    label = label.strong();
                }
                if ui
                    .selectable_label(open, label)
                    .on_hover_text("The document's public parameters: set them, the model follows")
                    .clicked()
                {
                    self.state.parameters.open = true;
                }
                let mut label = egui::RichText::new("Features");
                if !open {
                    label = label.strong();
                }
                if ui
                    .selectable_label(!open, label)
                    .on_hover_text("The design history")
                    .clicked()
                {
                    self.state.parameters.open = false;
                }
            });
        } else {
            let header = ui.label(egui::RichText::new("Features").strong());
            if has_document {
                // A document without public parameters has no
                // Parameters tab: how it gets one.
                header.on_hover_text(NO_PARAMETERS);
            }
        }
        ui.separator();
        if jig && self.state.parameters.open {
            self.parameters_tab(ui);
        } else {
            self.params.hits.clear();
            self.tree(ui, has_document);
        }
    }

    /// The Parameters tab.
    fn parameters_tab(&mut self, ui: &mut egui::Ui) {
        let tokens = self.tokens;
        let dim = color(tokens.text_dim);
        let now = ui.input(|i| i.time);
        self.params.hits.clear();
        let Some(list) = self.params.list.clone() else {
            ui.colored_label(dim, "No parameters.");
            return;
        };
        if !list.jig {
            ui.add(egui::Label::new(egui::RichText::new(NO_PARAMETERS).color(dim)).wrap());
            return;
        }
        if !list.info.title.is_empty() {
            ui.label(egui::RichText::new(&list.info.title).heading());
        }
        if !list.info.description.is_empty() {
            ui.add(egui::Label::new(egui::RichText::new(&list.info.description).color(dim)).wrap());
        }
        let busy = self.params.busy();
        ui.horizontal(|ui| {
            let any_off = list.params.iter().any(|p| !p.at_default);
            let reset = ui
                .add_enabled(any_off, egui::Button::new("Reset all"))
                .on_hover_text("Every parameter back to its default (param.set reset)");
            self.params.hits.push(("Reset all".to_string(), reset.rect));
            if reset.clicked() {
                self.params.ask(json!({ "reset": true }), None);
            }
            if busy {
                ui.add(egui::Spinner::new());
                ui.colored_label(dim, "Regenerating…");
            }
        });
        // What went wrong with a commit that came from no field.
        if let Some(trouble) = self.params.trouble.clone()
            && trouble.name.is_none()
        {
            self.trouble_line(ui, &trouble);
        }
        ui.separator();
        // The note goes after its time.
        if let Some(note) = self.params.note.as_mut() {
            match note.until {
                None => note.until = Some(now + NOTE_SECONDS),
                Some(until) if now >= until => self.params.note = None,
                Some(until) => {
                    ui.ctx()
                        .request_repaint_after(std::time::Duration::from_secs_f64(
                            (until - now).clamp(0.0, NOTE_SECONDS),
                        ));
                }
            }
        }
        egui::ScrollArea::vertical()
            .id_salt(("craft3d-params", self.viewing))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for group in &list.groups {
                    let members: Vec<&Param> =
                        list.params.iter().filter(|p| &p.group == group).collect();
                    if group.is_empty() {
                        for p in members {
                            self.param_row(ui, p, now);
                        }
                        continue;
                    }
                    let collapsed = self.state.parameters.collapsed.contains(group);
                    let header = egui::CollapsingHeader::new(egui::RichText::new(group).strong())
                        .id_salt(("craft3d-param-group", group))
                        .open(Some(!collapsed))
                        .show(ui, |ui| {
                            for p in &members {
                                self.param_row(ui, p, now);
                            }
                        });
                    if header.header_response.clicked() {
                        let collapsed = &mut self.state.parameters.collapsed;
                        if let Some(at) = collapsed.iter().position(|g| g == group) {
                            collapsed.remove(at);
                        } else if collapsed.len() < MAX_PARAMS {
                            collapsed.push(group.clone());
                        }
                    }
                }
                let footer: Vec<&str> = [list.info.author.as_str(), list.info.licence.as_str()]
                    .into_iter()
                    .filter(|s| !s.is_empty())
                    .collect();
                if !footer.is_empty() {
                    ui.add_space(tokens.inset);
                    let text = if list.info.author.is_empty() {
                        footer.join(" · ")
                    } else {
                        format!("by {}", footer.join(" · "))
                    };
                    ui.add(egui::Label::new(egui::RichText::new(text).small().color(dim)).wrap());
                }
            });
        // A drag that ended: its last value goes now.
        if self.params.memory.drag.is_none()
            && let Some(live) = self.params.live.as_mut()
            && live.dragging
        {
            live.dragging = false;
        }
        self.params.pace(now);
        if self.params.pending.is_some() {
            ui.ctx().request_repaint();
        } else if self.params.live.as_ref().is_some_and(|l| l.dirty) {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs_f64(DEBOUNCE));
        }
    }

    /// The trouble of the last commit, with Back to last good when the
    /// model broke.
    fn trouble_line(&mut self, ui: &mut egui::Ui, trouble: &Trouble) {
        let tokens = self.tokens;
        ui.add(
            egui::Label::new(
                egui::RichText::new(&trouble.message)
                    .small()
                    .color(color(tokens.failed)),
            )
            .wrap(),
        );
        if trouble.broke
            && let Some(values) = self.params.last_good.as_ref().map(|g| g.values.clone())
            && !values.is_empty()
        {
            let back = ui
                .small_button("Back to last good")
                .on_hover_text("Sets the parameters back to the last values that built");
            self.params
                .hits
                .push(("Back to last good".to_string(), back.rect));
            if back.clicked() {
                self.params.ask(json!({ "values": values }), None);
            }
        }
    }

    /// One parameter: its widget, its reset arrow, and what is said under
    /// it.
    fn param_row(&mut self, ui: &mut egui::Ui, p: &Param, now: f64) {
        let tokens = self.tokens;
        ui.push_id(("craft3d-param", &p.name), |ui| {
            ui.horizontal(|ui| {
                let reset_room = if p.at_default { 0.0 } else { 34.0 };
                let width = (ui.available_width() - reset_room).max(40.0);
                match p.kind {
                    ParamKind::Number | ParamKind::Integer => {
                        self.number_param(ui, p, width, now);
                    }
                    ParamKind::Boolean => {
                        let mut on = p.value.is_some_and(|v| v >= 0.5);
                        let response = ui.checkbox(&mut on, &p.label);
                        self.params.hits.push((p.name.clone(), response.rect));
                        let response = if p.description.is_empty() {
                            response
                        } else {
                            response.on_hover_text(&p.description)
                        };
                        if response.changed() {
                            self.params.commit(p, if on { 1.0 } else { 0.0 }, false);
                        }
                    }
                    ParamKind::Choice => {
                        let label = ui.label(&p.label);
                        if !p.description.is_empty() {
                            label.on_hover_text(&p.description);
                        }
                        let current = p.value.and_then(|v| p.choice_label(v)).unwrap_or("—");
                        let mut chosen = None;
                        let combo = egui::ComboBox::from_id_salt("choice")
                            .selected_text(current)
                            .show_ui(ui, |ui| {
                                for (i, choice) in p.choices.iter().enumerate() {
                                    if ui.selectable_label(choice == current, choice).clicked() {
                                        chosen = Some((i + 1) as f64);
                                    }
                                }
                            });
                        self.params.hits.push((p.name.clone(), combo.response.rect));
                        if let Some(value) = chosen
                            && Some(value) != p.value
                        {
                            self.params.commit(p, value, false);
                        }
                    }
                }
                if !p.at_default
                    && ui
                        .small_button("↺")
                        .on_hover_text(format!("Back to its default, {}", p.default_expr))
                        .clicked()
                {
                    let mut values = Map::new();
                    values.insert(p.name.clone(), Value::String(p.default_expr.clone()));
                    self.params
                        .ask(json!({ "values": values }), Some(p.name.clone()));
                }
            });
            let small = |text: &str, tint| {
                egui::Label::new(egui::RichText::new(text).small().color(color(tint))).wrap()
            };
            if let Some(error) = &p.error {
                ui.add(small(error, tokens.failed));
            }
            for problem in &p.problems {
                ui.add(small(problem, tokens.attention));
            }
            if !p.takes_numbers() {
                ui.add(small(
                    &format!("Set with its units: param.set {{\"{}\": \"…\"}}", p.name),
                    tokens.text_dim,
                ));
            }
            if let Some(note) = self.params.note.as_ref().filter(|n| n.name == p.name) {
                ui.add(small(&note.text, tokens.text_dim));
            }
            if let Some(trouble) = self
                .params
                .trouble
                .clone()
                .filter(|t| t.name.as_deref() == Some(p.name.as_str()))
            {
                self.trouble_line(ui, &trouble);
            }
            ui.add_space(2.0);
        });
    }

    /// A number or integer parameter: the number field of the viewport's
    /// controls, its bar the slider over the range.
    fn number_param(&mut self, ui: &mut egui::Ui, p: &Param, width: f32, now: f64) {
        let tokens = self.tokens;
        let height = tokens.nav_height.max(20.0);
        let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
        self.params.hits.push((p.name.clone(), rect));
        let value = self.params.shown_value(p).unwrap_or(0.0);
        let range = match (&self.params.live, p.range()) {
            (_, Some(range)) => range,
            (Some(live), None) if live.name == p.name => live.range,
            (_, None) => stand_in_range(p, value),
        };
        let step = p.field_step(range);
        let trouble = self
            .params
            .trouble
            .as_ref()
            .is_some_and(|t| t.name.as_deref() == Some(p.name.as_str()));
        let mine = self.params.active.as_deref().is_none_or(|a| a == p.name);
        let mut spare_memory = FieldMemory::default();
        let mut spare_typing = None;
        let (memory, typing) = if mine {
            (&mut self.params.memory, &mut self.params.typing)
        } else {
            (&mut spare_memory, &mut spare_typing)
        };
        let decimals = if p.kind.whole() {
            0
        } else {
            decimals_of(value)
                .max(p.step.map_or(0, decimals_of))
                .min(MAX_DECIMALS)
        };
        // The field is made for the dark plates over the viewport: it
        // gets one of its own on the light panel.
        ui.painter().rect_filled(
            rect,
            egui::CornerRadius::same(tokens.field_radius),
            color_alpha(tokens.plate_fill, tokens.plate_opacity),
        );
        let outcome = NumberField {
            id: FIELD,
            label: &p.label,
            value,
            range,
            step,
            scale: 1.0,
            decimals,
            unit: p.unit_shown(),
            enabled: p.takes_numbers() && p.value.is_some(),
            range_known: p.range().is_some(),
        }
        .show(ui, rect, &tokens, memory, typing);
        let dragging = memory.drag.is_some();
        let engaged = dragging || memory.focus.is_some() || typing.is_some();
        if !mine && engaged {
            // Another field took the mouse: it is the active one now.
            self.params.memory = spare_memory;
            self.params.typing = spare_typing;
        }
        if engaged {
            self.params.active = Some(p.name.clone());
        } else if self.params.active.as_deref() == Some(p.name.as_str()) {
            self.params.active = None;
        }
        if trouble {
            ui.painter().rect_stroke(
                rect,
                egui::CornerRadius::same(tokens.field_radius),
                egui::Stroke::new(1.5, color(tokens.failed)),
                egui::StrokeKind::Inside,
            );
        }
        if !p.description.is_empty() {
            ui.interact(rect, ui.id().with("tip"), egui::Sense::hover())
                .on_hover_text(&p.description);
        }
        if let Some(wanted) = outcome.value {
            let Some(settled) = settle(p, wanted) else {
                return;
            };
            if dragging {
                let live = match self.params.live.take() {
                    Some(live) if live.name == p.name && live.dragging => live,
                    _ => Live {
                        name: p.name.clone(),
                        value,
                        range,
                        dragging: true,
                        dirty: false,
                        committed: false,
                        last_sent: now,
                    },
                };
                self.params.live = Some(Live {
                    value: settled.value,
                    dirty: true,
                    ..live
                });
            } else {
                // An arrow or the wheel: a step of its own.
                self.params.say(&p.name, settled.note);
                self.params.live = Some(Live {
                    name: p.name.clone(),
                    value: settled.value,
                    range,
                    dragging: false,
                    dirty: true,
                    committed: false,
                    last_sent: now,
                });
            }
        }
    }

    /// Text typed into a parameter's field, once Enter was pressed: a
    /// number in its unit (or a length in another unit, with the session),
    /// clamped and snapped, and committed; or sent as it is typed for
    /// `param.set` to read (a value with other units); or the reason it
    /// is not a value, at the field.
    pub(super) fn resolve_param_typing(&mut self, session: Option<&Session>) {
        let Some(typing) = self.params.typing.as_ref().filter(|t| t.entered) else {
            return;
        };
        let Some(name) = self.params.active.clone() else {
            self.params.typing = None;
            return;
        };
        let Some(p) = self
            .params
            .list
            .as_ref()
            .and_then(|l| l.param(&name))
            .cloned()
        else {
            self.params.typing = None;
            return;
        };
        let text = typing.text.trim().to_string();
        let mut wanted = typed_number(&p, &text);
        if wanted.is_none()
            && let Some(session) = session
            && let Some(unit) = p.unit.as_deref()
        {
            // A length in another unit (`1.5 in`): in the document's unit,
            // then in the parameter's.
            let kind = craft3d_engine::TypedKind::Length;
            if let (Ok(value), Ok(one)) = (
                session.evaluate_typed(&text, kind),
                session.evaluate_typed(&format!("1 {unit}"), kind),
            ) && one != 0.0
            {
                wanted = Some(value / one).filter(|v| v.is_finite());
            }
        }
        if let Some(settled) = wanted.and_then(|v| settle(&p, v)) {
            self.params.say(&p.name, settled.note);
            self.params.commit(&p, settled.value, false);
        } else if text.is_empty() {
            if let Some(typing) = self.params.typing.as_mut() {
                typing.entered = false;
                typing.error = Some("type a value".to_string());
                typing.focused = false;
            }
            return;
        } else {
            // Units the form does not know: param.set reads them, and
            // refuses what is out of range.
            let mut values = Map::new();
            values.insert(p.name.clone(), Value::String(text));
            self.params
                .ask(json!({ "values": values }), Some(p.name.clone()));
        }
        self.params.typing = None;
        self.params.memory.focus = None;
        self.params.active = None;
    }
}

/// The range a drag spans for a parameter without both ends: from its
/// minimum or as far below the value as the value is from zero (at least
/// ten steps), to its maximum or as far above.
pub fn stand_in_range(p: &Param, value: f64) -> [f64; 2] {
    if !value.is_finite() {
        return [0.0, 1.0];
    }
    let step = p.step.filter(|s| s.is_finite() && *s > 0.0).unwrap_or(1.0);
    let span = value.abs().max(10.0 * step).max(1.0);
    let lo = p.min.filter(|m| m.is_finite()).unwrap_or(value - span);
    let hi = p.max.filter(|m| m.is_finite()).unwrap_or(value + span);
    if hi > lo && lo.is_finite() && hi.is_finite() {
        [lo, hi]
    } else {
        [0.0, 1.0]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn width() -> Param {
        Param {
            name: "width".into(),
            label: "Width".into(),
            kind: ParamKind::Number,
            expr: "80 mm".into(),
            value: Some(80.0),
            unit: Some("mm".into()),
            default_expr: "80 mm".into(),
            at_default: true,
            min: Some(20.0),
            max: Some(120.0),
            step: Some(0.5),
            valid: true,
            ..Param::default()
        }
    }

    #[test]
    fn a_param_list_is_read_into_the_form() {
        let listed = json!({
            "jig": true,
            "groups": ["Size", "Finish"],
            "parameters": [
                {"name": "width", "label": "Width", "description": "Outside width", "group": "Size",
                 "kind": "number", "expr": "80 mm", "value": 80.0, "unit": "mm",
                 "default": {"expr": "80 mm", "value": 80.0}, "atDefault": true,
                 "min": {"expr": "20 mm", "value": 20.0}, "max": {"expr": "depth * 2", "error": "no depth"},
                 "step": null, "valid": true, "problems": []},
                {"name": "corners", "label": "Corners", "group": "Finish", "kind": "choice",
                 "choices": ["sharp", "round"], "expr": "2", "value": 2.0, "choice": "round", "unit": null,
                 "default": {"expr": "1", "value": 1.0}, "atDefault": false,
                 "min": null, "max": null, "step": null, "valid": false, "problems": ["out of range"]},
                {"name": "", "kind": "number"},
                "not an object"
            ],
            "info": {"title": "Box", "description": "A box", "author": "J", "licence": "MIT", "url": "", "tags": []}
        });
        let list = ParamList::parse(&listed);
        assert!(list.jig);
        assert_eq!(list.groups, vec!["Size".to_string(), "Finish".to_string()]);
        assert_eq!(list.params.len(), 2, "nameless entries are dropped");
        let w = list.param("width").unwrap();
        assert_eq!(w.min, Some(20.0));
        assert_eq!(w.max, None, "a bound that does not evaluate is none");
        assert_eq!(w.range(), None);
        assert_eq!(w.description, "Outside width");
        let c = list.param("corners").unwrap();
        assert_eq!(c.kind, ParamKind::Choice);
        assert_eq!(c.choice_label(2.0), Some("round"));
        assert_eq!(c.choice_label(3.0), None);
        assert!(!c.valid && !c.at_default);
        assert_eq!(c.problems, vec!["out of range".to_string()]);
        assert_eq!(list.info.title, "Box");
        // Anything else is an empty form, never a panic.
        for junk in [
            json!(null),
            json!([1, 2]),
            json!({"parameters": 3}),
            json!("x"),
        ] {
            let list = ParamList::parse(&junk);
            assert!(!list.jig && list.params.is_empty());
        }
    }

    #[test]
    fn values_are_clamped_and_snapped_before_they_are_sent() {
        let p = width();
        assert_eq!(
            settle(&p, 80.0).unwrap(),
            Settled {
                value: 80.0,
                note: None
            }
        );
        let over = settle(&p, 500.0).unwrap();
        assert_eq!(over.value, 120.0);
        assert_eq!(over.note.as_deref(), Some("clamped to 120 mm"));
        let under = settle(&p, -3.0).unwrap();
        assert_eq!(under.value, 20.0);
        assert_eq!(under.note.as_deref(), Some("clamped to 20 mm"));
        let off = settle(&p, 33.3).unwrap();
        assert_eq!(off.value, 33.5);
        assert_eq!(off.note.as_deref(), Some("rounded to 33.5 mm"));
        // A step of 0.1 keeps its decimals.
        let tenth = Param {
            step: Some(0.1),
            min: Some(0.0),
            ..width()
        };
        assert_eq!(settle(&tenth, 0.3).unwrap().value, 0.3);
        assert_eq!(settle(&tenth, 0.30000000000000004).unwrap().note, None);
        assert!(settle(&p, f64::NAN).is_none());
        assert!(settle(&p, f64::INFINITY).is_none());
        // Whole kinds: whole numbers within their own range.
        let count = Param {
            kind: ParamKind::Integer,
            min: Some(2.5),
            max: Some(9.0),
            step: None,
            unit: None,
            ..width()
        };
        assert_eq!(settle(&count, 4.4).unwrap().value, 4.0);
        assert_eq!(
            settle(&count, 1.0).unwrap().value,
            3.0,
            "the least whole number in range"
        );
        let lid = Param {
            kind: ParamKind::Boolean,
            min: None,
            max: None,
            step: None,
            unit: None,
            ..width()
        };
        assert_eq!(settle(&lid, 7.0).unwrap().value, 1.0);
        let corners = Param {
            kind: ParamKind::Choice,
            choices: vec!["sharp".into(), "round".into()],
            min: None,
            max: None,
            step: None,
            unit: None,
            ..width()
        };
        assert_eq!(settle(&corners, 0.0).unwrap().value, 1.0);
        assert_eq!(settle(&corners, 5.0).unwrap().value, 2.0);
        assert_eq!(sent_value(&corners, 2.0), json!(2));
        assert_eq!(sent_value(&p, 33.5), json!(33.5));
        // Typed: a number, or one with the parameter's own unit.
        assert_eq!(typed_number(&p, " 42.5 "), Some(42.5));
        assert_eq!(typed_number(&p, "42 mm"), Some(42.0));
        assert_eq!(typed_number(&p, "4 cm"), None);
        assert_eq!(typed_number(&p, "width / 2"), None);
    }

    #[test]
    fn a_drag_commits_once_per_debounce_and_on_release_and_only_on_release_on_the_web() {
        let mut live = Live {
            name: "width".into(),
            value: 90.0,
            range: [20.0, 120.0],
            dragging: true,
            dirty: true,
            committed: false,
            last_sent: 10.0,
        };
        let desktop = Pacer {
            single_thread: false,
        };
        let web = Pacer {
            single_thread: true,
        };
        assert!(!desktop.due(&live, 10.05, false), "within the debounce");
        assert!(desktop.due(&live, 10.15, false));
        assert!(
            !desktop.due(&live, 10.5, true),
            "never while one is under way"
        );
        assert!(
            !web.due(&live, 99.0, false),
            "the web waits for the release"
        );
        live.dragging = false;
        assert!(web.due(&live, 10.01, false));
        assert!(desktop.due(&live, 10.01, false));
        live.dirty = false;
        assert!(!desktop.due(&live, 50.0, false));
    }

    #[test]
    fn the_merge_flag_folds_a_drag_into_one_undo_step() {
        let mut form = ParamForm {
            list: Some(ParamList {
                jig: true,
                groups: vec![String::new()],
                params: vec![width()],
                info: DocInfo::default(),
            }),
            pacer: Some(Pacer {
                single_thread: false,
            }),
            ..ParamForm::default()
        };
        form.live = Some(Live {
            name: "width".into(),
            value: 90.0,
            range: [20.0, 120.0],
            dragging: true,
            dirty: true,
            committed: false,
            last_sent: 0.0,
        });
        // First commit of the drag: its own undo step.
        form.pace(1.0);
        let first = form.pending.take().unwrap();
        assert_eq!(first.command.command, "param.set");
        assert_eq!(
            first.command.params,
            json!({"values": {"width": 90.0}, "merge": false})
        );
        // The next ones fold into it.
        if let Some(live) = form.live.as_mut() {
            live.value = 95.0;
            live.dirty = true;
        }
        form.pace(1.05);
        assert!(form.pending.is_none(), "debounced");
        form.pace(1.2);
        let second = form.pending.take().unwrap();
        assert_eq!(
            second.command.params,
            json!({"values": {"width": 95.0}, "merge": true})
        );
        // Released: the last value goes at once, still merged.
        if let Some(live) = form.live.as_mut() {
            live.value = 101.0;
            live.dirty = true;
            live.dragging = false;
        }
        form.pace(1.21);
        let last = form.pending.take().unwrap();
        assert_eq!(
            last.command.params,
            json!({"values": {"width": 101.0}, "merge": true})
        );
        // A commit asked while another waits is joined to it.
        let p = width();
        form.commit(&p, 50.0, false);
        form.ask(json!({"values": {"depth": "30 mm"}}), None);
        let joined = form.pending.take().unwrap();
        assert_eq!(
            joined.command.params,
            json!({"values": {"width": 50.0, "depth": "30 mm"}, "merge": false})
        );
        assert_eq!(joined.field, None);
    }

    #[test]
    fn a_commit_that_breaks_the_model_is_marked_and_a_refusal_is_said() {
        let base = Baseline::default();
        assert_eq!(verdict(&Ok(json!({"set": []})), &[], base), Verdict::Good);
        assert_eq!(
            verdict(&Err("param.set: nothing was changed".into()), &[], base),
            Verdict::Refused("param.set: nothing was changed".into())
        );
        let broke = Ok(json!({"failed": [{"feature": "Fillet1", "error": "radius too large"}]}));
        assert_eq!(
            verdict(
                &broke,
                &[("Fillet1".into(), "radius too large".into())],
                base
            ),
            Verdict::Broke("Fillet1: radius too large".into())
        );
        // Still failing after a later commit of the same drag (no new
        // failure, but more than the last good state had).
        assert_eq!(
            verdict(
                &Ok(json!({})),
                &[("Fillet1".into(), "radius too large".into())],
                base
            ),
            Verdict::Broke("Fillet1: radius too large".into())
        );
        // A document that failed before the form touched it is its
        // baseline, not a failure of the form.
        let failing_before = Baseline {
            features: 1,
            assertions: 0,
        };
        assert_eq!(
            verdict(
                &Ok(json!({})),
                &[("Old".into(), "x".into())],
                failing_before
            ),
            Verdict::Good
        );
        let assertion = Ok(json!({"assertions": {"count": 1, "passed": 0,
            "broken": [{"name": "wall", "message": "wall is 1 mm, must be ≥ 2 mm"}],
            "unproven": [], "pending": 0}}));
        assert_eq!(
            verdict(&assertion, &[], base),
            Verdict::Broke("wall is 1 mm, must be ≥ 2 mm".into())
        );
    }

    #[test]
    fn a_range_stands_in_for_a_parameter_without_one() {
        let open = Param {
            min: None,
            max: None,
            step: None,
            ..width()
        };
        assert_eq!(stand_in_range(&open, 80.0), [0.0, 160.0]);
        let low = Param {
            max: None,
            ..width()
        };
        assert_eq!(stand_in_range(&low, 80.0), [20.0, 160.0]);
        assert_eq!(stand_in_range(&open, 0.0), [-10.0, 10.0]);
        assert_eq!(stand_in_range(&open, f64::NAN), [0.0, 1.0]);
        assert_eq!(decimals_of(12.5), 1);
        assert_eq!(decimals_of(120.0), 0);
        assert_eq!(decimals_of(0.125), 3);
        assert_eq!(width().written(33.5), "33.5 mm");
    }
}
