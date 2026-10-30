//! Regenerating elsewhere: the active document's regeneration and meshing
//! done by another instance of the engine, such as the page's Web Worker,
//! so that the host's own thread never regenerates a feature or meshes a
//! body to show the document (docs/design/engine.md › *Regeneration in a
//! Web Worker*).
//!
//! A host that says so ([`Session::set_regen_elsewhere`], with
//! [`Session::set_stepwise`] and [`Session::set_checks_elsewhere`]) takes
//! the job of the scene it shows ([`Session::scene_job`], a number) when
//! that scene is not there, has it written out
//! ([`Session::scene_request`]: the shown document state as the bytes of
//! its `.3dcraft` file, the quality as the bits of its numbers) and hands
//! that to the other side, which answers part by part with
//! [`scene_begin`] and [`scene_next`]: the quick first picture, then the
//! scene at the quality asked for, each the scene's bytes
//! (`scene_wire.rs`) or the error that building it gave. Each part goes
//! back with [`Session::scene_answered`] and is kept as a scene built
//! here would be; the host shows it at once. The end of the answer goes
//! back with [`Session::scene_done`].
//!
//! The assertions and outputs of such a document are measured elsewhere
//! too, a state at a time, since the host has no regeneration to key
//! them by: the jobs of checks ([`Session::checks_job`]) are then of the
//! whole state, written out the same way and answered by
//! [`crate::measure_checks`] (`checks_elsewhere.rs`).
//!
//! The scenes are the ones this session would have built, bit for bit:
//! regeneration and meshing are functions of the document alone (and a
//! saved document regenerates to the bodies it was saved from), and the
//! scene crosses in a form that reads back as the same numbers.
//!
//! While the host has no regeneration of the state it shows, what needs
//! one waits for it or makes it: a command that needs the bodies
//! (`select.set`, `measure.*`, `file.export`, `assert.list`) regenerates
//! here when it runs, as a command always did; the assertions and outputs
//! shown are those measured elsewhere (pending until they are there); and
//! the selection is checked against the document once it is regenerated
//! here.

use crate::assert_cmds::Checked;
use crate::output_cmds::Resolved;
use crate::{CommandError, OpenDocument, SceneQuality, Session, Workspace};
use craft3d_doc::Document;
use serde_json::{Value, json};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::Arc;

/// A job of scenes, written out to be built elsewhere.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SceneRequest {
    /// What its parts go back with ([`Session::scene_answered`]).
    pub token: u64,
    /// The document state to show, as the bytes of its `.3dcraft` file
    /// (with the files it holds: an imported STEP file).
    pub document: Vec<u8>,
    /// What to build: `{quality, budget}`.
    pub work: String,
}

/// The job of scenes handed out last.
#[derive(Clone, Debug)]
pub(crate) struct SceneJob {
    pub token: u64,
    /// The document state it builds the scenes of.
    pub doc: Arc<Document>,
    pub quality: SceneQuality,
}

/// The assertions and outputs of one state, measured elsewhere whole.
#[derive(Clone, Default)]
pub(crate) struct StateChecks {
    /// The job handed out, by number, and the state it measures.
    pub job: Option<(u64, Arc<Document>)>,
    /// The last answer, for the state it was measured for: the checks in
    /// the order of its assertions, and its outputs.
    pub answer: Option<(Arc<Document>, Vec<Checked>, Arc<Resolved>)>,
}

/// What a part of an answer is of.
const PREVIEW: u8 = 0;
const ASKED: u8 = 1;
/// What a part holds.
const SCENE: u8 = b'S';
const ERROR: u8 = b'E';

impl Session {
    /// Has the active document regenerated and meshed elsewhere from now
    /// on (by a host that does the long work a piece at a time and has
    /// the checks measured elsewhere, [`Session::set_stepwise`] and
    /// [`Session::set_checks_elsewhere`]): [`Session::prepare_some`] no
    /// longer regenerates or meshes, and the host hands the jobs of the
    /// scenes over ([`Session::scene_job`]). Off again, what is not there
    /// is built here, a piece at a time.
    pub fn set_regen_elsewhere(&mut self, elsewhere: bool) {
        self.regen_elsewhere = elsewhere;
        if !elsewhere {
            self.scene_job = None;
        }
    }

    /// True when the active document is regenerated elsewhere: asked for
    /// ([`Session::set_regen_elsewhere`]), with the long work done a
    /// piece at a time and the checks measured elsewhere (without those,
    /// the host has to regenerate to measure them: it regenerates
    /// here). Never for an assembly: what it shows is placed from the
    /// files read for it in this session, which a document sent elsewhere
    /// does not carry, so it is built here a piece at a time.
    pub fn regen_elsewhere(&self) -> bool {
        self.regen_elsewhere
            && self.stepwise
            && self.checks_elsewhere
            && !self
                .documents
                .get(self.active)
                .is_some_and(|open| open.history.current().is_assembly())
    }

    /// True when the active document's current state is regenerated in
    /// this session (a host that regenerates elsewhere has it only once a
    /// command needed the bodies).
    pub fn regenerated_here(&self) -> bool {
        self.documents.get(self.active).is_some_and(|open| {
            open.regen
                .as_ref()
                .is_some_and(|(at, _)| *at == open.history.revision())
        })
    }

    /// The job of scenes of the active document that waits to be built
    /// elsewhere, by its number: the scene of the state shown at
    /// `quality` and its quick first picture, while one of them is not
    /// there. `None` when both are (or their errors), when there is no
    /// document, or when it is not regenerated elsewhere. The same job
    /// keeps its number until it is answered or replaced (an edit, an
    /// undo, another document, another quality): a host whose number
    /// changed drops the work it started for the old one.
    pub fn scene_job(&mut self, quality: SceneQuality) -> Option<u64> {
        if !self.regen_elsewhere() {
            return None;
        }
        self.scene_revision();
        let open = self.documents.get_mut(self.active)?;
        let shown = open.shown();
        let current = Arc::clone(open.history.current());
        // Scrubbed, with no scene of the document as it is: its first
        // picture first, which carries the datum planes a section lies on
        // while the view is scrubbed (`section.rs`), so that the page
        // never regenerates to find them.
        let (doc, quality) =
            if !Arc::ptr_eq(&shown, &current) && self.kept_scene_of(&current).is_none() {
                (current, SceneQuality::Preview)
            } else {
                (shown, quality)
            };
        let wanted =
            !self.has_scene_of(&doc, SceneQuality::Preview) || !self.has_scene_of(&doc, quality);
        if !wanted {
            if self
                .scene_job
                .as_ref()
                .is_some_and(|job| Arc::ptr_eq(&job.doc, &doc))
            {
                self.scene_job = None;
            }
            return None;
        }
        if let Some(job) = &self.scene_job
            && Arc::ptr_eq(&job.doc, &doc)
            && same_quality(job.quality, quality)
        {
            return Some(job.token);
        }
        self.scene_token = self.scene_token.wrapping_add(1).max(1);
        self.scene_job = Some(SceneJob {
            token: self.scene_token,
            doc,
            quality,
        });
        Some(self.scene_token)
    }

    /// The job numbered `token` ([`Session::scene_job`]), written out for
    /// [`scene_begin`]. An error when it does not wait any more, or its
    /// document cannot be written.
    pub fn scene_request(&self, token: u64) -> Result<SceneRequest, String> {
        let job = self
            .job_of_scenes(token)
            .ok_or_else(|| format!("no job of scenes {token} waits"))?;
        let document = craft3d_format::to_bytes(&job.doc)
            .map_err(|e| format!("the document cannot be written out: {e}"))?;
        let work = json!({
            "quality": quality_json(job.quality),
            "budget": self.scene_budget(),
        });
        Ok(SceneRequest {
            token,
            document,
            work: work.to_string(),
        })
    }

    /// A part of the answer to job `token`: what [`scene_next`] returned,
    /// or why the other side stopped while building it. A scene is kept
    /// for the job's document state; a stop makes every scene of the job
    /// not there yet an error that says why, and ends the job. False when
    /// the job does not wait any more (an edit replaced it): the part is
    /// dropped.
    pub fn scene_answered(&mut self, token: u64, part: Result<&[u8], &str>) -> bool {
        let Some(job) = self.job_of_scenes(token).cloned() else {
            return false;
        };
        let bytes = match part {
            Ok(bytes) => bytes,
            Err(why) => {
                self.end_scene_job(
                    &job,
                    &format!(
                        "internal error (a bug in 3DCraft, please report it): could not be regenerated off the page's thread: the worker building it stopped: {why}"
                    ),
                );
                return true;
            }
        };
        let (of, built) = match read_part(bytes) {
            Ok((of, SCENE, payload)) => {
                let revision = self.scene_revision_now();
                let scene = crate::scene_wire::decode(payload, &job.doc, revision)
                    .map(Arc::new)
                    .map_err(|why| {
                        CommandError::new(format!(
                            "internal error (a bug in 3DCraft, please report it): the scene built off the page's thread cannot be read: {why}"
                        ))
                    });
                (of, scene)
            }
            Ok((of, _, payload)) => (
                of,
                Err(CommandError::new(
                    String::from_utf8_lossy(payload).into_owned(),
                )),
            ),
            Err(why) => {
                self.end_scene_job(
                    &job,
                    &format!(
                        "internal error (a bug in 3DCraft, please report it): could not be regenerated off the page's thread: {why}"
                    ),
                );
                return true;
            }
        };
        let quality = if of == PREVIEW {
            SceneQuality::Preview
        } else {
            job.quality
        };
        self.keep_scene_of(&job.doc, quality, built);
        true
    }

    /// The answer to job `token` is complete: a scene of the job that did
    /// not come is an error (a bug), and the job ends. False when it does
    /// not wait any more.
    pub fn scene_done(&mut self, token: u64) -> bool {
        let Some(job) = self.job_of_scenes(token).cloned() else {
            return false;
        };
        self.end_scene_job(
            &job,
            "internal error (a bug in 3DCraft, please report it): the worker that regenerates off the page's thread answered without this scene",
        );
        true
    }

    /// Job `token` could not be built, for a reason that is no bug (it took
    /// too long, the document cannot be written out): every scene of the
    /// job not there yet is an error that says `why` as it is. False when
    /// it does not wait any more.
    pub fn scene_failed(&mut self, token: u64, why: &str) -> bool {
        let Some(job) = self.job_of_scenes(token).cloned() else {
            return false;
        };
        self.end_scene_job(&job, &format!("not regenerated: {why}"));
        true
    }

    fn job_of_scenes(&self, token: u64) -> Option<&SceneJob> {
        self.scene_job
            .as_ref()
            .filter(|job| token != 0 && job.token == token)
    }

    /// Ends `job`: what it did not bring is `why`.
    fn end_scene_job(&mut self, job: &SceneJob, why: &str) {
        for quality in [SceneQuality::Preview, job.quality] {
            if !self.has_scene_of(&job.doc, quality) {
                self.keep_scene_of(&job.doc, quality, Err(CommandError::new(why)));
            }
        }
        self.scene_job = None;
    }

    /// The job of checks of the whole current state of the active
    /// document (`checks_elsewhere.rs` hands it out while the document is
    /// regenerated elsewhere): `None` when the state has no assertions or
    /// outputs, or is answered.
    pub(crate) fn state_checks_job(&mut self) -> Option<u64> {
        let open = self.documents.get(self.active)?;
        let doc = open.history.current();
        if doc.assertions.is_empty() && doc.outputs.is_empty() {
            return None;
        }
        let checks = &open.state_checks;
        if checks
            .answer
            .as_ref()
            .is_some_and(|(d, ..)| Arc::ptr_eq(d, doc))
        {
            return None;
        }
        if let Some((token, d)) = &checks.job
            && Arc::ptr_eq(d, doc)
        {
            return Some(*token);
        }
        let doc = Arc::clone(doc);
        self.checks_token = self.checks_token.wrapping_add(1).max(1);
        let token = self.checks_token;
        self.documents.get_mut(self.active)?.state_checks.job = Some((token, doc));
        Some(token)
    }

    /// Where the open document whose job of checks of a whole state is
    /// `token` is, and that state.
    pub(crate) fn state_job(&self, token: u64) -> Option<(usize, Arc<Document>)> {
        if token == 0 {
            return None;
        }
        self.documents
            .iter()
            .enumerate()
            .find_map(|(at, open)| match &open.state_checks.job {
                Some((t, doc)) if *t == token => Some((at, Arc::clone(doc))),
                _ => None,
            })
    }

    /// The answer of the job of checks of a whole state at document
    /// `at`, for `doc`: the job ends.
    pub(crate) fn state_answered(
        &mut self,
        at: usize,
        doc: Arc<Document>,
        checked: Vec<Checked>,
        outputs: Resolved,
    ) {
        if let Some(open) = self.documents.get_mut(at) {
            open.state_checks.job = None;
            open.state_checks.answer = Some((doc, checked, Arc::new(outputs)));
        }
    }
}

fn same_quality(a: SceneQuality, b: SceneQuality) -> bool {
    quality_json(a) == quality_json(b)
}

/// A quality written out, its numbers as their bits.
fn quality_json(quality: SceneQuality) -> Value {
    let bits = |v: f64| format!("{:016x}", v.to_bits());
    match quality {
        SceneQuality::Automatic => json!({"kind": "automatic"}),
        SceneQuality::Preview => json!({"kind": "preview"}),
        SceneQuality::Tolerance(t) => json!({"kind": "tolerance", "tolerance": bits(t)}),
        SceneQuality::Picture(t) => json!({"kind": "picture", "tolerance": bits(t)}),
        SceneQuality::Display {
            relative,
            max_triangles,
            max_angle,
        } => json!({
            "kind": "display",
            "relative": bits(relative),
            "maxTriangles": max_triangles,
            "maxAngle": max_angle.map(bits),
        }),
    }
}

fn quality_from(value: &Value) -> Option<SceneQuality> {
    let bits = |key: &str| {
        value
            .get(key)
            .and_then(Value::as_str)
            .and_then(|b| u64::from_str_radix(b, 16).ok())
            .map(f64::from_bits)
    };
    Some(match value.get("kind")?.as_str()? {
        "automatic" => SceneQuality::Automatic,
        "preview" => SceneQuality::Preview,
        "tolerance" => SceneQuality::Tolerance(bits("tolerance")?),
        "picture" => SceneQuality::Picture(bits("tolerance")?),
        "display" => SceneQuality::Display {
            relative: bits("relative")?,
            max_triangles: usize::try_from(value.get("maxTriangles")?.as_u64()?).ok()?,
            max_angle: match value.get("maxAngle")? {
                Value::Null => None,
                _ => Some(bits("maxAngle")?),
            },
        },
        _ => return None,
    })
}

/// A part: what it is of, what it holds, and the rest.
fn read_part(bytes: &[u8]) -> Result<(u8, u8, &[u8]), String> {
    match bytes {
        [of @ (PREVIEW | ASKED), what @ (SCENE | ERROR), rest @ ..] => Ok((*of, *what, rest)),
        _ => Err("a part of its answer is not a scene".into()),
    }
}

fn part(of: u8, built: Result<Option<Arc<crate::Scene>>, CommandError>) -> Vec<u8> {
    match built {
        Ok(Some(scene)) => {
            let mut bytes = vec![of, SCENE];
            bytes.extend(crate::scene_wire::encode(&scene));
            bytes
        }
        Ok(None) => error_part(of, "no document to show"),
        Err(e) => error_part(of, e.message()),
    }
}

fn error_part(of: u8, why: &str) -> Vec<u8> {
    let mut bytes = vec![of, ERROR];
    bytes.extend(why.as_bytes());
    bytes
}

/// The bytes a scene crosses as (`scene_wire.rs`), without its revision:
/// two scenes with the same bytes are the same scene bit for bit.
pub fn scene_bytes(scene: &crate::Scene) -> Vec<u8> {
    crate::scene_wire::encode(scene)
}

/// The other side: a session of its own with the one document whose
/// states it is handed, so that each state rebuilds only what changed
/// since the last (its regeneration cache and its store of meshes stay
/// between requests), and the parts of the answer still to build.
#[derive(Default)]
struct Builder {
    session: Option<Session>,
    /// What is left to build: the part, of what, at which quality.
    left: VecDeque<(u8, SceneQuality)>,
    /// Why the request cannot be built: answered as the error of each
    /// part.
    refused: Option<String>,
    /// The bytes of the document state the session holds, and of the one
    /// whose scenes it builds: a command run on another state
    /// ([`run_command`]) puts the latter back.
    loaded: Option<Vec<u8>>,
    scenes_of: Option<Vec<u8>>,
    /// The selection the last command run here read, as written out, and
    /// the one it left: a command that comes with the same selection was
    /// handed over before that answer reached the host, so it runs on
    /// what the last one left, as it would have there.
    selected: Option<(Value, Value)>,
}

thread_local! {
    static BUILDER: RefCell<Builder> = RefCell::new(Builder::default());
}

/// Puts `doc` in `session` as the state of its one document: a new state
/// of the same document (the caches kept), or the first.
pub(crate) fn load_state(session: &mut Session, doc: Document) {
    match session.documents.get_mut(0) {
        Some(open) => {
            let _ = open.history.apply("state", |current| {
                *current = doc;
                Ok::<(), ()>(())
            });
        }
        None => {
            let mut open = OpenDocument::new(doc, None);
            open.history = open.history.clone().with_limit(1);
            session.documents.push(open);
        }
    }
    session.active = 0;
}

/// Starts the answer to a job of scenes written out by
/// [`Session::scene_request`]: the document is read and made this side's
/// state; [`scene_next`] then builds the parts one by one. Never panics on
/// any input: a request that cannot be read is answered with an error
/// part for each scene.
pub fn scene_begin(document: &[u8], work: &str) {
    BUILDER.with(|builder| {
        let Ok(mut builder) = builder.try_borrow_mut() else {
            return;
        };
        let read = (|| -> Result<(Document, SceneQuality, Option<usize>), String> {
            let doc = craft3d_format::from_bytes(document)
                .map_err(|e| format!("the document cannot be read: {e}"))?;
            let work: Value =
                serde_json::from_str(work).map_err(|e| format!("the work cannot be read: {e}"))?;
            let quality = work
                .get("quality")
                .and_then(quality_from)
                .ok_or("the quality cannot be read")?;
            let budget = work
                .get("budget")
                .and_then(Value::as_u64)
                .and_then(|b| usize::try_from(b).ok());
            Ok((doc, quality, budget))
        })();
        builder.left.clear();
        builder.refused = None;
        builder.scenes_of = None;
        let quality = match read {
            Ok((doc, quality, budget)) => {
                let same = builder.loaded.as_deref() == Some(document);
                let session = builder.session.get_or_insert_with(builder_session);
                if let Some(budget) = budget {
                    session.set_scene_budget(budget);
                }
                // The state it holds already (another quality of it): its
                // scenes are there.
                if !same {
                    load_state(session, doc);
                    builder.loaded = Some(document.to_vec());
                }
                builder.scenes_of = Some(document.to_vec());
                quality
            }
            Err(why) => {
                builder.refused = Some(why);
                SceneQuality::display()
            }
        };
        builder.left.push_back((PREVIEW, SceneQuality::Preview));
        if !same_quality(quality, SceneQuality::Preview) {
            builder.left.push_back((ASKED, quality));
        }
    });
}

/// What [`scene_step`] did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SceneStep {
    /// A part of the answer, for the host's [`Session::scene_answered`].
    Part(Vec<u8>),
    /// A piece of the work was done (a feature regenerated, a body
    /// meshed); the next part is not there yet.
    Working,
    /// The answer is complete.
    Done,
}

/// Does a piece of the answer [`scene_begin`] started: regenerates and
/// meshes until `stop` says so (asked after each feature built and each
/// body meshed; at least one is done per call) or a part is there. A
/// worker that asks for a piece at a time can look at its messages
/// between pieces, and leave a job a newer one replaced without ending
/// itself: what it built stays in its caches, and the next state rebuilds
/// only what its edit changed. The parts are those [`scene_next`] builds
/// in one go, bit for bit.
pub fn scene_step(stop: impl Fn() -> bool + Clone + 'static) -> SceneStep {
    BUILDER.with(|builder| {
        let Ok(mut builder) = builder.try_borrow_mut() else {
            return SceneStep::Done;
        };
        let Some(&(of, quality)) = builder.left.front() else {
            return SceneStep::Done;
        };
        if let Some(why) = &builder.refused {
            let part = error_part(of, why);
            builder.left.pop_front();
            return SceneStep::Part(part);
        }
        let asked = builder.left.back().map_or(quality, |&(_, q)| q);
        let Some(session) = builder.session.as_mut() else {
            builder.left.pop_front();
            return SceneStep::Part(error_part(of, "no document to show"));
        };
        let there = session
            .documents
            .get_mut(session.active)
            .map(OpenDocument::shown)
            .is_some_and(|doc| session.has_scene_of(&doc, quality));
        if !there && session.prepare_some(asked, stop).is_some() {
            return SceneStep::Working;
        }
        // There, or nothing left to do a piece at a time (a quality that
        // is not built in pieces is built now, in one go).
        let part = part(of, session.scene_with(quality));
        builder.left.pop_front();
        SceneStep::Part(part)
    })
}

/// The next part of the answer [`scene_begin`] started: the quick first
/// picture, then the scene at the quality asked for, each built now;
/// `None` when the answer is complete. The bytes go back to the host's
/// [`Session::scene_answered`].
pub fn scene_next() -> Option<Vec<u8>> {
    loop {
        match scene_step(|| false) {
            SceneStep::Part(part) => return Some(part),
            SceneStep::Working => {}
            SceneStep::Done => return None,
        }
    }
}

/// Lets go of everything this side built: what a worker that was ended
/// and started again has (the next state regenerates from nothing).
pub fn scene_forget() {
    BUILDER.with(|builder| {
        if let Ok(mut builder) = builder.try_borrow_mut() {
            *builder = Builder::default();
        }
    });
}

/// Runs a command written out by [`Session::command_request`] on the
/// document state it was written with, in the session that builds the
/// scenes (whose regeneration of that state is there, or is reused from
/// the states before it), with the selection it read. The answer is for
/// [`Session::command_answered`]: its result, whether it changed the
/// document (it is then not taken, and the host runs it itself), the
/// selection it left when that changed, and the files it wrote. A job of
/// scenes in hand goes on afterwards. Never panics on any input.
pub fn run_command(document: &[u8], work: &str) -> crate::CommandAnswer {
    let answered = BUILDER.with(|builder| {
        let mut builder = builder
            .try_borrow_mut()
            .map_err(|_| "the worker is busy".to_string())?;
        let work: Value =
            serde_json::from_str(work).map_err(|e| format!("the work cannot be read: {e}"))?;
        let command = work
            .get("command")
            .and_then(Value::as_str)
            .ok_or("the work names no command")?
            .to_string();
        let params = work.get("params").cloned().unwrap_or(Value::Null);
        let builder = &mut *builder;
        let read = work.get("selection").cloned().unwrap_or(Value::Null);
        let wire = match &builder.selected {
            Some((input, output))
                if *input == read && builder.loaded.as_deref() == Some(document) =>
            {
                output.clone()
            }
            _ => read.clone(),
        };
        let selection = crate::selection::Selection::from_wire(&wire, None).unwrap_or_default();
        let session = builder.session.get_or_insert_with(builder_session);
        if builder.loaded.as_deref() != Some(document) {
            let doc = craft3d_format::from_bytes(document)
                .map_err(|e| format!("the document cannot be read: {e}"))?;
            load_state(session, doc);
            builder.loaded = Some(document.to_vec());
        }
        let open = session.documents.get_mut(0).ok_or("no document")?;
        open.selection = selection;
        let revision = open.history.revision();
        for name in session.file_names() {
            session.take_file(&name);
        }
        session.check_selection();
        let before = session.documents.first().map(|o| o.selection.to_wire());
        let result = session.execute(&command, &params);
        let edited = session.documents.len() != 1
            || session
                .documents
                .first()
                .is_none_or(|open| open.history.revision() != revision);
        let after = session.documents.first().map(|o| o.selection.to_wire());
        let selection = (!edited && after != before).then_some(after).flatten();
        builder.selected = match &selection {
            Some(left) => Some((read, left.clone())),
            None if !edited && wire != read => builder.selected.clone(),
            None => None,
        };
        let mut files = Vec::new();
        for name in session.file_names() {
            if let Some(bytes) = session.take_file(&name) {
                files.push((name, bytes));
            }
        }
        if edited {
            // Its edit is not this side's to keep: the state is loaded
            // again, from the bytes, the next time.
            builder.loaded = None;
        }
        // The scenes in hand are of the state they were asked for.
        if let Some(scenes_of) = builder.scenes_of.clone()
            && builder.loaded.as_deref() != Some(&scenes_of[..])
            && let Ok(doc) = craft3d_format::from_bytes(&scenes_of)
        {
            load_state(session, doc);
            builder.loaded = Some(scenes_of);
        }
        Ok::<_, String>(crate::CommandAnswer {
            answer: crate::commands_elsewhere::answer_json(result, edited, selection),
            files,
        })
    });
    answered.unwrap_or_else(|why: String| crate::CommandAnswer {
        answer: crate::commands_elsewhere::answer_json(
            Err(CommandError::new(format!(
                "internal error (a bug in 3DCraft, please report it): it could not be run beside the page: {why}"
            ))),
            false,
            None,
        ),
        files: Vec::new(),
    })
}

/// This side's session: the long work a piece at a time
/// ([`scene_step`]), the checks left to the checking worker.
fn builder_session() -> Session {
    // Files in memory: what a command run here writes ([`run_command`]).
    let mut s = Session::new(Workspace::memory());
    s.set_stepwise(true);
    s.set_checks_elsewhere(true);
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qualities_cross_bit_for_bit() {
        for q in [
            SceneQuality::display(),
            SceneQuality::Display {
                relative: 0.1 + 0.2,
                max_triangles: 7,
                max_angle: None,
            },
            SceneQuality::Preview,
            SceneQuality::Automatic,
            SceneQuality::Tolerance(-0.0),
            SceneQuality::Picture(1e-300),
        ] {
            let back = quality_from(&quality_json(q));
            assert!(back.is_some_and(|b| same_quality(b, q)), "{q:?}");
        }
        for bad in [
            json!(null),
            json!({"kind": "display"}),
            json!({"kind": "tolerance", "tolerance": "zz"}),
            json!({"kind": "nonsense"}),
        ] {
            assert!(quality_from(&bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn a_request_that_cannot_be_read_is_answered_with_errors() {
        for (document, work) in [
            (&b""[..], ""),
            (b"PK\x03\x04", "{}"),
            (b"{}", "{\"quality\": 3}"),
        ] {
            scene_begin(document, work);
            let mut parts = 0;
            while let Some(part) = scene_next() {
                assert!(matches!(read_part(&part), Ok((_, ERROR, _))));
                parts += 1;
            }
            assert_eq!(parts, 2);
        }
        assert!(read_part(b"").is_err());
        assert!(read_part(b"\x07S").is_err());
    }
}
