//! The view made elsewhere: an assembly (placed from its sources) and the
//! moments of a motion (`view.motion`) of the active document, made by
//! the instance of the engine that regenerates it (the page's Web Worker,
//! `regen_elsewhere.rs`), so that the host's own thread never solves,
//! places or regenerates to show them (docs/design/engine.md › *Motion
//! and assemblies in the worker*).
//!
//! The host keeps the view state (the `t` shown, collisions asked for, a
//! bake wanted) and, for what it shows, a stand-in: a document state of
//! its own that the scenes answered from elsewhere are kept under, as a
//! state regenerated elsewhere is ([`Remote`]). The job of scenes of such
//! a view carries the state below it (the document, or the state the view
//! is scrubbed to; an assembly with the source files this session read,
//! [`state_bytes`]) and the view (`t`, collisions, the bake). The other
//! side loads them into its session, makes the moment or the placement
//! there as a session that shows them itself does, and answers first with
//! the view's facts (what `view.motion` says of the moment, an assembly's
//! outline and the joints a drag moves, the placed part's features), then
//! with the scenes; a bake wanted goes on there a piece at a time, each
//! piece answered with its facts. So the host draws a moment, lists an
//! assembly and plays its motion without placing or regenerating
//! anything, and what it draws is what a session that shows the view
//! itself draws, bit for bit (`crates/engine/tests/view_elsewhere.rs`).

use crate::asm_outline::{AssemblyOutline, JointHandle, OutlineInstance, OutlineMate};
use crate::{OpenDocument, Session};
use craft3d_doc::{Blob, Document};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::sync::Arc;

/// What a stand-in's scenes wait for, and what came.
#[derive(Clone)]
pub(crate) struct RemoteView {
    /// The state below the view (the document, or the state it is
    /// scrubbed to), the generation of the assembly's files, and the
    /// moment (`t` and whether collisions are found) when a motion is
    /// shown.
    pub base: Arc<Document>,
    pub generation: u64,
    pub moment: Option<(f64, bool)>,
    /// The stand-in: the scenes answered for the view are kept under it.
    pub doc: Arc<Document>,
    /// What the other side said of the view, once it did.
    pub facts: Option<Arc<ViewFacts>>,
}

/// A bake asked for while the view is made elsewhere (`motion.bake`):
/// the other side bakes, a piece at a time, after each moment it is
/// asked for.
#[derive(Clone)]
pub(crate) struct BakeWanted {
    pub base: Arc<Document>,
    pub generation: u64,
    pub frames: u64,
    pub collisions: bool,
    /// The answer of the last piece made there (`motion.bake`'s).
    pub answer: Option<Value>,
}

impl BakeWanted {
    /// Nothing more to bake: the last piece said it is done, or failed.
    pub(crate) fn finished(&self) -> bool {
        self.answer
            .as_ref()
            .is_some_and(|a| a.get("done") == Some(&Value::Bool(true)) || a.get("error").is_some())
    }

    /// What is left, for a progress bar: what the last piece said (one
    /// before it said anything).
    pub(crate) fn left(&self) -> usize {
        if self.finished() {
            return 0;
        }
        self.answer
            .as_ref()
            .and_then(|a| a.get("remaining"))
            .and_then(Value::as_u64)
            .map_or(1, |n| usize::try_from(n).unwrap_or(usize::MAX).max(1))
    }
}

/// The view of an open document as made elsewhere: view state, not saved
/// and not undone.
#[derive(Clone, Default)]
pub(crate) struct Remote {
    view: Option<RemoteView>,
    /// The last facts answered for a state below the view: what the host
    /// lists while the next moment of it is made (the tree does not
    /// flicker as a motion plays).
    last: Option<(Arc<Document>, u64, Arc<ViewFacts>)>,
    pub(crate) bake: Option<BakeWanted>,
    /// Facts came since the host last polled ([`Session::poll_motion`]).
    fresh: bool,
    /// Whether the state below the view has a timeline, for that state:
    /// asked many times a frame, worked out once per state.
    timeline_of: Option<(Arc<Document>, bool)>,
}

/// What the other side said of a view.
#[derive(Clone, Debug, Default)]
pub(crate) struct ViewFacts {
    /// `view.motion`'s answer there for the moment (`None` at rest).
    pub motion: Option<Value>,
    /// Why the moment could not be made there: the motion is let go of,
    /// as a session that makes it itself lets go of it.
    pub error: Option<String>,
    /// An assembly's outline there ([`Session::assembly_outline`]).
    pub outline: Option<AssemblyOutline>,
    /// The joints a drag on each placed feature moves
    /// ([`Session::assembly_joint_of`]).
    pub joints: BTreeMap<String, JointHandle>,
    /// An assembly's placed part as shown there: what its scenes are read
    /// against (its features' kinds).
    pub shown: Option<Arc<Document>>,
}

/// Most joints and outline rows a facts part may carry.
const MAX_ROWS: usize = 4_096;

/// What the bytes of a state with its files start with.
const STATE_MAGIC: &[u8; 8] = b"C3DSTATE";
/// The form's version.
const STATE_VERSION: u8 = 1;

/// An assembly's sources as this session read them: what a state handed
/// elsewhere carries so that the other side places it from the same
/// bytes ([`state_bytes`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Sources {
    /// The assembly's file (its sources are relative to its folder).
    pub path: Option<String>,
    /// For one without a file: the folder its sources were read from.
    pub base: String,
    /// The files read, by their path from the read root.
    pub files: BTreeMap<String, Result<Blob, String>>,
}

impl OpenDocument {
    /// The sources an assembly state handed elsewhere needs: `None` for a
    /// part.
    pub(crate) fn sources(&self, doc: &Document) -> Option<Sources> {
        doc.is_assembly().then(|| Sources {
            path: self.path.clone(),
            base: self.asm.unsaved_base.clone(),
            files: self.asm.files.clone(),
        })
    }

    /// The motion shown (`t` and collisions), when there is one.
    fn shown_moment(&self) -> Option<(f64, bool)> {
        self.motion_t().map(|t| (t, self.motion_collisions()))
    }

    /// True when what the view shows is made elsewhere: the session shows
    /// its views from elsewhere, and this is an assembly or its motion is
    /// shown (on a state with a timeline: one without lets go of it here).
    pub(crate) fn remote_wanted(&mut self) -> bool {
        if !self.work.elsewhere() {
            return false;
        }
        if self.history.current().is_assembly() {
            return true;
        }
        if self.motion_t().is_none() {
            return false;
        }
        let base = self.scrubbed();
        if let Some((of, ok)) = &self.remote.timeline_of
            && Arc::ptr_eq(of, &base)
        {
            return *ok;
        }
        let ok = crate::motion::timeline(&base).is_ok();
        self.remote.timeline_of = Some((base, ok));
        ok
    }

    /// The stand-in the view's scenes are kept under, made when the view
    /// changed (another `t`, an edit, a reload); `None` when the view is
    /// not made elsewhere.
    pub(crate) fn remote_shown(&mut self) -> Option<Arc<Document>> {
        if !self.remote_wanted() {
            return None;
        }
        let base = if self.history.current().is_assembly() {
            Arc::clone(self.history.current())
        } else {
            self.scrubbed()
        };
        let generation = self.asm.generation;
        let moment = self.shown_moment();
        if let Some(view) = &self.remote.view
            && Arc::ptr_eq(&view.base, &base)
            && view.generation == generation
            && same_moment(view.moment, moment)
        {
            return Some(Arc::clone(&view.doc));
        }
        // A copy of the state below, with the timeline set: a document
        // state of its own (the scenes are kept by state), with the
        // features of the moment (its scenes read against them).
        let mut doc = (*base).clone();
        if let Some((t, _)) = moment {
            // A timeline that cannot be set leaves the copy as it is: it
            // is still a state of its own, and the other side says why
            // the moment cannot be made (`ViewFacts::error`).
            let _ = doc.set_variable(crate::motion::TIMELINE, &crate::motion::literal(t));
        }
        let doc = Arc::new(doc);
        // Facts of another state below are of no use any more.
        if self
            .remote
            .last
            .as_ref()
            .is_some_and(|(b, g, _)| !Arc::ptr_eq(b, &base) || *g != generation)
        {
            self.remote.last = None;
        }
        if self
            .remote
            .bake
            .as_ref()
            .is_some_and(|b| !Arc::ptr_eq(&b.base, &base) || b.generation != generation)
        {
            self.remote.bake = None;
        }
        self.remote.view = Some(RemoteView {
            base,
            generation,
            moment,
            doc: Arc::clone(&doc),
            facts: None,
        });
        Some(doc)
    }

    /// The view made elsewhere as it stands (made by
    /// [`Self::remote_shown`] first).
    pub(crate) fn remote_view(&mut self) -> Option<&RemoteView> {
        self.remote_shown()?;
        self.remote.view.as_ref()
    }

    /// The facts of the view shown, or while they are not there, the last
    /// ones of the same state below it.
    pub(crate) fn remote_facts(&mut self) -> Option<Arc<ViewFacts>> {
        self.remote_shown()?;
        if let Some(facts) = self.remote.view.as_ref().and_then(|v| v.facts.clone()) {
            return Some(facts);
        }
        self.remote.last.as_ref().map(|(_, _, f)| Arc::clone(f))
    }

    /// Takes the note that facts came since the last call.
    pub(crate) fn remote_fresh(&mut self) -> bool {
        std::mem::take(&mut self.remote.fresh)
    }

    /// Asks for a bake made elsewhere (`motion.bake` while the view is
    /// made elsewhere); `None` frames cancels it. The state below is the
    /// one shown now.
    pub(crate) fn want_bake(&mut self, frames: Option<u64>, collisions: bool) -> bool {
        let Some(view) = self.remote_view().cloned() else {
            return false;
        };
        self.remote.bake = frames.map(|frames| BakeWanted {
            base: view.base,
            generation: view.generation,
            frames,
            collisions,
            answer: None,
        });
        true
    }

    /// The bake still to be made elsewhere for the view shown.
    pub(crate) fn bake_left(&mut self) -> Option<BakeWanted> {
        let view = self.remote_view()?.clone();
        self.remote
            .bake
            .clone()
            .filter(|b| Arc::ptr_eq(&b.base, &view.base) && b.generation == view.generation)
    }

    /// Keeps the facts answered for the stand-in `doc` (`facts`, or only
    /// a bake piece's answer); false when that is not the view any more.
    fn take_facts(
        &mut self,
        doc: &Arc<Document>,
        facts: Option<ViewFacts>,
        bake: Option<Value>,
    ) -> bool {
        let Some(view) = self.remote.view.as_mut() else {
            return false;
        };
        if !Arc::ptr_eq(&view.doc, doc) {
            return false;
        }
        if let Some(facts) = facts {
            let facts = Arc::new(facts);
            view.facts = Some(Arc::clone(&facts));
            self.remote.last = Some((Arc::clone(&view.base), view.generation, facts));
        }
        if let (Some(answer), Some(wanted)) = (bake, self.remote.bake.as_mut())
            && Arc::ptr_eq(&wanted.base, &view.base)
            && wanted.generation == view.generation
        {
            wanted.answer = Some(answer);
        }
        self.remote.fresh = true;
        true
    }
}

fn same_moment(a: Option<(f64, bool)>, b: Option<(f64, bool)>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some((t, c)), Some((u, d))) => t.to_bits() == u.to_bits() && c == d,
        _ => false,
    }
}

/// The view a job of scenes carries when what it builds is made
/// elsewhere ([`OpenDocument::remote_view`]).
#[derive(Clone, Debug)]
pub(crate) struct ViewJob {
    pub base: Arc<Document>,
    pub moment: Option<(f64, bool)>,
    pub sources: Option<Sources>,
    pub bake: Option<(u64, bool)>,
}

impl ViewJob {
    /// The view as the work of a job writes it, its `t` as its bits.
    pub(crate) fn json(&self) -> Value {
        json!({
            "t": self.moment.map(|(t, _)| format!("{:016x}", t.to_bits())),
            "collisions": self.moment.is_some_and(|(_, c)| c),
            "bake": self.bake.map(|(frames, collisions)| json!({"frames": frames, "collisions": collisions})),
        })
    }
}

/// The view a job's work asks for: the moment (`t` read back from its
/// bits) and the bake. `None` for a job without a view.
pub(crate) fn view_of(work: &Value) -> Option<(Option<(f64, bool)>, Option<(u64, bool)>)> {
    let view = work.get("view")?.as_object()?;
    let t = view
        .get("t")
        .and_then(Value::as_str)
        .and_then(|bits| u64::from_str_radix(bits, 16).ok())
        .map(f64::from_bits)
        .filter(|t| t.is_finite());
    let collisions = view.get("collisions") == Some(&Value::Bool(true));
    let bake = view.get("bake").and_then(|b| {
        let frames = b.get("frames")?.as_u64()?.clamp(1, 1000);
        Some((frames, b.get("collisions") == Some(&Value::Bool(true))))
    });
    Some((t.map(|t| (t, collisions)), bake))
}

impl Session {
    /// The view of the active document's job of scenes, when it is made
    /// elsewhere and `doc` is its stand-in.
    pub(crate) fn view_job(&mut self, doc: &Arc<Document>) -> Option<ViewJob> {
        let open = self.documents.get_mut(self.active)?;
        let view = open.remote_view()?.clone();
        if !Arc::ptr_eq(&view.doc, doc) {
            return None;
        }
        let bake = open
            .bake_left()
            .filter(|b| !b.finished())
            .map(|b| (b.frames, b.collisions));
        let sources = open.sources(&view.base);
        Some(ViewJob {
            base: view.base,
            moment: view.moment,
            sources,
            bake,
        })
    }

    /// Takes the facts part of the answer for the stand-in `doc`.
    /// Returns the document the scenes that follow are read against (an
    /// assembly's placed part as shown there), or why it cannot be read.
    pub(crate) fn view_answered(
        &mut self,
        doc: &Arc<Document>,
        bake_only: bool,
        payload: &[u8],
    ) -> Result<(), String> {
        let (facts, bake) = decode_facts(payload)?;
        let facts = (!bake_only).then_some(facts);
        for open in &mut self.documents {
            if open.take_facts(doc, facts.clone(), bake.clone()) {
                break;
            }
        }
        Ok(())
    }

    /// The document the scenes of the stand-in `doc` are read against:
    /// the placed part the other side showed (an assembly), else `doc`.
    pub(crate) fn scenes_read_against(&self, doc: &Arc<Document>) -> Arc<Document> {
        self.documents
            .iter()
            .find_map(|open| {
                let view = open.remote.view.as_ref()?;
                Arc::ptr_eq(&view.doc, doc)
                    .then(|| view.facts.as_ref().and_then(|f| f.shown.clone()))
                    .flatten()
            })
            .unwrap_or_else(|| Arc::clone(doc))
    }

    /// The facts of the active document's view as made elsewhere: what
    /// the host shows of a moment or an assembly without making it.
    pub(crate) fn remote_facts(&mut self) -> Option<Option<Arc<ViewFacts>>> {
        let open = self.documents.get_mut(self.active)?;
        if !open.remote_wanted() {
            return None;
        }
        Some(open.remote_facts())
    }
}

/// The facts of the view the session (on the other side) shows, as the
/// bytes of a part: their JSON, then the shown document's file for an
/// assembly. `bake` is the answer of the bake piece just made.
pub(crate) fn facts_bytes(
    session: &mut Session,
    error: Option<String>,
    bake: Option<Value>,
) -> Vec<u8> {
    let mut facts = Map::new();
    let is_assembly = session
        .documents
        .get(session.active)
        .is_some_and(|open| open.history.current().is_assembly());
    let moment = session
        .documents
        .get(session.active)
        .and_then(OpenDocument::motion_t);
    let mut error = error;
    if moment.is_some() && error.is_none() {
        match session.execute("view.motion", &json!({})) {
            Ok(answer) if answer.get("t").is_some_and(|t| !t.is_null()) => {
                facts.insert("motion".into(), answer);
            }
            Ok(_) => error = Some("the motion could not be shown".into()),
            Err(e) => error = Some(e.message().to_string()),
        }
    }
    let mut shown = Vec::new();
    if is_assembly {
        if let Some(outline) = session.assembly_outline() {
            facts.insert("outline".into(), outline_json(&outline));
        }
        let placed = session
            .documents
            .get_mut(session.active)
            .map(OpenDocument::shown);
        if let Some(placed) = placed {
            let mut joints = Map::new();
            for feature in placed.features.iter().take(MAX_ROWS) {
                let id = feature.id.to_string();
                if let Some(joint) = session.assembly_joint_of(&id) {
                    joints.insert(id, joint_json(&joint));
                }
            }
            facts.insert("joints".into(), Value::Object(joints));
            match craft3d_format::to_bytes(&placed) {
                Ok(bytes) => shown = bytes,
                Err(e) => error = Some(format!("its placed parts cannot be written out: {e}")),
            }
        }
    }
    if let Some(why) = error {
        facts.insert("error".into(), why.into());
    }
    if let Some(bake) = bake {
        facts.insert("bake".into(), bake);
    }
    let text = Value::Object(facts).to_string();
    let mut out = Vec::with_capacity(text.len().saturating_add(shown.len()).saturating_add(4));
    out.extend(u32::try_from(text.len()).unwrap_or(u32::MAX).to_le_bytes());
    out.extend(text.as_bytes());
    out.extend(shown);
    out
}

/// The bytes of a bake piece's answer, as a part of facts that holds
/// only it.
pub(crate) fn bake_bytes(answer: &Value) -> Vec<u8> {
    let text = json!({ "bake": answer }).to_string();
    let mut out = Vec::with_capacity(text.len().saturating_add(4));
    out.extend(u32::try_from(text.len()).unwrap_or(u32::MAX).to_le_bytes());
    out.extend(text.as_bytes());
    out
}

/// The facts a part holds ([`facts_bytes`]), and the bake piece's answer.
pub(crate) fn decode_facts(bytes: &[u8]) -> Result<(ViewFacts, Option<Value>), String> {
    let (len, rest) = bytes
        .split_first_chunk::<4>()
        .ok_or("the view's facts are cut short")?;
    let len = usize::try_from(u32::from_le_bytes(*len)).map_err(|_| "too long")?;
    let text = rest.get(..len).ok_or("the view's facts are cut short")?;
    let shown = rest.get(len..).unwrap_or_default();
    let value: Value = serde_json::from_slice(text)
        .map_err(|e| format!("the view's facts cannot be read: {e}"))?;
    if !value.is_object() {
        return Err("the view's facts are not an object".into());
    }
    let mut facts = ViewFacts {
        motion: value.get("motion").filter(|m| m.is_object()).cloned(),
        error: value
            .get("error")
            .and_then(Value::as_str)
            .map(|s| s.chars().take(4_096).collect()),
        ..ViewFacts::default()
    };
    if let Some(outline) = value.get("outline") {
        facts.outline = Some(outline_from(outline).ok_or("the assembly's outline cannot be read")?);
    }
    if let Some(joints) = value.get("joints").and_then(Value::as_object) {
        for (feature, joint) in joints.iter().take(MAX_ROWS) {
            let joint = joint_from(joint).ok_or("a joint of the assembly cannot be read")?;
            facts.joints.insert(feature.clone(), joint);
        }
    }
    if !shown.is_empty() {
        let doc = craft3d_format::from_bytes(shown)
            .map_err(|e| format!("the placed parts cannot be read: {e}"))?;
        facts.shown = Some(Arc::new(doc));
    }
    Ok((facts, value.get("bake").cloned()))
}

/// A number as its bits, so that it reads back as the same `f64`.
fn bits(v: f64) -> String {
    format!("{:016x}", v.to_bits())
}

fn from_bits(value: &Value) -> Option<f64> {
    value
        .as_str()
        .and_then(|b| u64::from_str_radix(b, 16).ok())
        .map(f64::from_bits)
}

fn outline_json(o: &AssemblyOutline) -> Value {
    json!({
        "instances": o.instances.iter().take(MAX_ROWS).map(|i| json!({
            "name": i.name,
            "label": i.label,
            "depth": i.depth,
            "source": i.source,
            "subassembly": i.subassembly,
            "error": i.error,
            "visible": i.visible,
            "own": i.own,
            "features": i.features,
        })).collect::<Vec<_>>(),
        "mates": o.mates.iter().take(MAX_ROWS).map(|m| json!({
            "name": m.name,
            "kind": m.kind,
            "status": m.status,
            "error": m.error,
            "value": m.value.map(|(v, angle)| json!([bits(v), angle])),
            "driven": m.driven,
            "variables": m.variables,
            "settable": m.settable,
            "limits": m.limits.map(|(a, b)| json!([bits(a), bits(b)])),
        })).collect::<Vec<_>>(),
        "dof": o.dof,
        "free": o.free,
        "solveError": o.solve_error,
        "unit": o.unit,
    })
}

fn text(value: &Value, key: &str) -> Option<String> {
    value.get(key)?.as_str().map(str::to_string)
}

fn opt_text(value: &Value, key: &str) -> Option<Option<String>> {
    match value.get(key)? {
        Value::Null => Some(None),
        Value::String(s) => Some(Some(s.clone())),
        _ => None,
    }
}

fn texts(value: &Value, key: &str) -> Option<Vec<String>> {
    value
        .get(key)?
        .as_array()?
        .iter()
        .take(MAX_ROWS)
        .map(|v| v.as_str().map(str::to_string))
        .collect()
}

fn pair(value: &Value) -> Option<(f64, f64)> {
    match value {
        Value::Array(items) => match items.as_slice() {
            [a, b] => Some((from_bits(a)?, from_bits(b)?)),
            _ => None,
        },
        _ => None,
    }
}

/// A mate kind's name, as the outline holds it (`&'static str`).
fn kind_name(name: &str) -> Option<&'static str> {
    use craft3d_doc::MateKind;
    [
        MateKind::Fixed,
        MateKind::Revolute,
        MateKind::Slider,
        MateKind::Cylindrical,
        MateKind::Planar,
        MateKind::Ball,
    ]
    .into_iter()
    .map(MateKind::name)
    .find(|k| *k == name)
}

fn status_name(name: &str) -> Option<&'static str> {
    ["ok", "failed", "unsolved"]
        .into_iter()
        .find(|s| *s == name)
}

fn outline_from(value: &Value) -> Option<AssemblyOutline> {
    let mut out = AssemblyOutline {
        dof: match value.get("dof")? {
            Value::Null => None,
            v => Some(usize::try_from(v.as_u64()?).ok()?),
        },
        free: texts(value, "free")?,
        solve_error: opt_text(value, "solveError")?,
        unit: text(value, "unit")?,
        ..AssemblyOutline::default()
    };
    for i in value.get("instances")?.as_array()?.iter().take(MAX_ROWS) {
        out.instances.push(OutlineInstance {
            name: text(i, "name")?,
            label: text(i, "label")?,
            depth: usize::try_from(i.get("depth")?.as_u64()?).ok()?,
            source: text(i, "source")?,
            subassembly: i.get("subassembly")?.as_bool()?,
            error: opt_text(i, "error")?,
            visible: i.get("visible")?.as_bool()?,
            own: i.get("own")?.as_bool()?,
            features: texts(i, "features")?,
        });
    }
    for m in value.get("mates")?.as_array()?.iter().take(MAX_ROWS) {
        out.mates.push(OutlineMate {
            name: text(m, "name")?,
            kind: kind_name(m.get("kind")?.as_str()?)?,
            status: status_name(m.get("status")?.as_str()?)?,
            error: opt_text(m, "error")?,
            value: match m.get("value")? {
                Value::Null => None,
                Value::Array(items) => match items.as_slice() {
                    [v, angle] => Some((from_bits(v)?, angle.as_bool()?)),
                    _ => return None,
                },
                _ => return None,
            },
            driven: m.get("driven")?.as_bool()?,
            variables: texts(m, "variables")?,
            settable: m.get("settable")?.as_bool()?,
            limits: match m.get("limits")? {
                Value::Null => None,
                v => Some(pair(v)?),
            },
        });
    }
    Some(out)
}

fn joint_json(j: &JointHandle) -> Value {
    json!({
        "mate": j.mate,
        "angle": j.angle,
        "origin": j.origin.map(bits),
        "axis": j.axis.map(bits),
        "value": bits(j.value),
        "sign": bits(j.sign),
        "limits": j.limits.map(|(a, b)| json!([bits(a), bits(b)])),
        "unit": j.unit,
        "size": bits(j.size),
    })
}

fn three(value: &Value) -> Option<[f64; 3]> {
    match value.as_array()?.as_slice() {
        [a, b, c] => Some([from_bits(a)?, from_bits(b)?, from_bits(c)?]),
        _ => None,
    }
}

fn joint_from(value: &Value) -> Option<JointHandle> {
    Some(JointHandle {
        mate: text(value, "mate")?,
        angle: value.get("angle")?.as_bool()?,
        origin: three(value.get("origin")?)?,
        axis: three(value.get("axis")?)?,
        value: from_bits(value.get("value")?)?,
        sign: from_bits(value.get("sign")?)?,
        limits: match value.get("limits")? {
            Value::Null => None,
            v => Some(pair(v)?),
        },
        unit: text(value, "unit")?,
        size: from_bits(value.get("size")?)?,
    })
}

/// The bytes a document state crosses as: its `.3dcraft` file, or for an
/// assembly that file with the sources this session read for it (its
/// path and folder, each file read or why it could not be), so the other
/// side places it from the same bytes.
pub(crate) fn state_bytes(doc: &Document, sources: Option<&Sources>) -> Result<Vec<u8>, String> {
    let file = craft3d_format::to_bytes(doc).map_err(|e| e.to_string())?;
    let Some(sources) = sources else {
        return Ok(file);
    };
    let mut out = Vec::new();
    out.extend(STATE_MAGIC);
    out.push(STATE_VERSION);
    let text = |out: &mut Vec<u8>, s: &str| {
        out.extend((s.len() as u64).to_le_bytes());
        out.extend(s.as_bytes());
    };
    match &sources.path {
        Some(path) => {
            out.push(1);
            text(&mut out, path);
        }
        None => out.push(0),
    }
    text(&mut out, &sources.base);
    out.extend((sources.files.len() as u64).to_le_bytes());
    for (path, read) in &sources.files {
        text(&mut out, path);
        match read {
            Ok(blob) => {
                out.push(1);
                out.extend((blob.bytes().len() as u64).to_le_bytes());
                out.extend(blob.bytes());
            }
            Err(why) => {
                out.push(0);
                text(&mut out, why);
            }
        }
    }
    out.extend(file);
    Ok(out)
}

/// A document state as [`state_bytes`] wrote it, with the sources it
/// carries. Never panics; what it allocates is bounded by the bytes.
pub(crate) fn read_state(bytes: &[u8]) -> Result<(Document, Option<Sources>), String> {
    let Some(rest) = bytes.strip_prefix(STATE_MAGIC.as_slice()) else {
        let doc = craft3d_format::from_bytes(bytes).map_err(|e| e.to_string())?;
        return Ok((doc, None));
    };
    let mut r = Reader { bytes: rest };
    if r.u8()? != STATE_VERSION {
        return Err("the state is of another form".into());
    }
    let path = match r.u8()? {
        0 => None,
        1 => Some(r.text()?),
        _ => return Err("the state's path cannot be read".into()),
    };
    let base = r.text()?;
    let count = r.u64()?;
    if count > crate::asm::MAX_SOURCE_FILES as u64 {
        return Err("the state carries too many files".into());
    }
    let mut files = BTreeMap::new();
    for _ in 0..count {
        let name = r.text()?;
        let read = match r.u8()? {
            1 => Ok(Blob::new(r.chunk()?.to_vec())),
            0 => Err(r.text()?),
            _ => return Err("a file of the state cannot be read".into()),
        };
        files.insert(name, read);
    }
    let doc = craft3d_format::from_bytes(r.bytes).map_err(|e| e.to_string())?;
    Ok((doc, Some(Sources { path, base, files })))
}

struct Reader<'a> {
    bytes: &'a [u8],
}

impl<'a> Reader<'a> {
    fn u8(&mut self) -> Result<u8, String> {
        let (first, rest) = self.bytes.split_first().ok_or("the state is cut short")?;
        self.bytes = rest;
        Ok(*first)
    }

    fn u64(&mut self) -> Result<u64, String> {
        let (n, rest) = self
            .bytes
            .split_first_chunk::<8>()
            .ok_or("the state is cut short")?;
        self.bytes = rest;
        Ok(u64::from_le_bytes(*n))
    }

    fn chunk(&mut self) -> Result<&'a [u8], String> {
        let len = usize::try_from(self.u64()?).map_err(|_| "the state is cut short")?;
        if len > self.bytes.len() {
            return Err("the state is cut short".into());
        }
        let (chunk, rest) = self.bytes.split_at(len);
        self.bytes = rest;
        Ok(chunk)
    }

    fn text(&mut self) -> Result<String, String> {
        String::from_utf8(self.chunk()?.to_vec())
            .map_err(|_| "a text of the state is not UTF-8".into())
    }
}

/// Puts `sources` in the assembly `open` (the other side's), as this
/// session's own reading would have: what changed makes it place again.
pub(crate) fn install(open: &mut OpenDocument, sources: Sources) {
    let Sources { path, base, files } = sources;
    let changed = open.path != path || open.asm.unsaved_base != base || open.asm.files != files;
    open.path = path;
    if changed {
        open.asm.unsaved_base = base;
        open.asm.files = files;
        open.asm.invalidate();
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;

    #[test]
    fn a_state_with_its_sources_reads_back_and_never_panics() {
        let mut doc = Document::default();
        doc.set_variable("a", "2 mm").unwrap();
        let mut files = BTreeMap::new();
        files.insert("parts/a.3dcraft".to_string(), Ok(Blob::new(vec![1, 2, 3])));
        files.insert("parts/b.3dcraft".to_string(), Err("not found".to_string()));
        let sources = Sources {
            path: Some("top.3dcraftasm".into()),
            base: String::new(),
            files,
        };
        let bytes = state_bytes(&doc, Some(&sources)).unwrap();
        let (back, read) = read_state(&bytes).unwrap();
        assert_eq!(back, doc);
        assert_eq!(read, Some(sources));
        // Every prefix is an error, never a panic.
        for end in 0..bytes.len() {
            assert!(read_state(&bytes[..end]).is_err(), "{end}");
        }
        // A count beyond what the bytes hold is refused before anything
        // is made.
        let mut huge = STATE_MAGIC.to_vec();
        huge.extend([STATE_VERSION, 0]);
        huge.extend(0u64.to_le_bytes());
        huge.extend(u64::MAX.to_le_bytes());
        assert!(read_state(&huge).is_err());
        // A plain file is a state without sources.
        let plain = state_bytes(&doc, None).unwrap();
        assert!(matches!(read_state(&plain), Ok((d, None)) if d == doc));
    }

    #[test]
    fn facts_cross_bit_for_bit_and_hostile_ones_are_errors() {
        let outline = AssemblyOutline {
            instances: vec![OutlineInstance {
                name: "Sun".into(),
                label: "Sun".into(),
                depth: 0,
                source: "packed".into(),
                subassembly: false,
                error: None,
                visible: true,
                own: true,
                features: vec!["F1".into()],
            }],
            mates: vec![OutlineMate {
                name: "Spin".into(),
                kind: "revolute",
                status: "ok",
                error: Some("x".into()),
                value: Some((0.1 + 0.2, true)),
                driven: true,
                variables: vec!["Spin_angle".into()],
                settable: false,
                limits: Some((-0.0, f64::MAX)),
            }],
            dof: Some(1),
            free: vec!["Spin turns".into()],
            solve_error: None,
            unit: "mm".into(),
        };
        let joint = JointHandle {
            mate: "Spin".into(),
            angle: true,
            origin: [1.0 / 3.0, -0.0, 7.0],
            axis: [0.0, 0.0, 1.0],
            value: std::f64::consts::PI,
            sign: -1.0,
            limits: None,
            unit: "mm".into(),
            size: 1e-300,
        };
        assert_eq!(outline_from(&outline_json(&outline)), Some(outline));
        assert_eq!(joint_from(&joint_json(&joint)), Some(joint));
        for bad in [
            &b""[..],
            b"\x05\x00\x00\x00{}",
            b"\xff\xff\xff\xff{}",
            b"\x02\x00\x00\x00[]",
            b"\x18\x00\x00\x00{\"outline\": {\"x\": 1}}xx",
        ] {
            assert!(decode_facts(bad).is_err(), "{bad:?}");
        }
        let (facts, bake) = decode_facts(b"\x02\x00\x00\x00{}").unwrap();
        assert!(facts.motion.is_none() && facts.shown.is_none() && bake.is_none());
    }
}
