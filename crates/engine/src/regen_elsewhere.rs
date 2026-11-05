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
use crate::view_elsewhere::Sources;
use crate::{CommandError, OpenDocument, SceneQuality, Session, Workspace};
use craft3d_doc::Document;
use craft3d_math::BUG;
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
    /// When `doc` is the stand-in of a view made elsewhere (an assembly,
    /// a moment of the motion: `view_elsewhere.rs`), that view.
    pub view: Option<crate::view_elsewhere::ViewJob>,
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
/// The view's facts (`view_elsewhere.rs`): the first part of the answer
/// to a job with a view.
const VIEW: u8 = 2;
/// A piece of a bake made after the scenes: its answer.
const BAKE: u8 = 3;
/// What a part holds.
const SCENE: u8 = b'S';
const ERROR: u8 = b'E';
const FACTS: u8 = b'F';

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
        self.sync_elsewhere();
    }

    /// Tells the documents whether their views are made elsewhere (an
    /// assembly, a moment of the motion: `view_elsewhere.rs`): whenever
    /// one of the flags [`Session::regen_elsewhere`] reads changes.
    pub(crate) fn sync_elsewhere(&self) {
        self.work
            .set_elsewhere(self.regen_elsewhere && self.stepwise && self.checks_elsewhere);
    }

    /// True when the active document is regenerated elsewhere: asked for
    /// ([`Session::set_regen_elsewhere`]), with the long work done a
    /// piece at a time and the checks measured elsewhere (without those,
    /// the host has to regenerate to measure them: it regenerates
    /// here). An assembly too: its state goes with the source files this
    /// session read for it (`view_elsewhere.rs`), and it is placed there.
    pub fn regen_elsewhere(&self) -> bool {
        self.regen_elsewhere && self.stepwise && self.checks_elsewhere
    }

    /// True when the active document's current state is regenerated in
    /// this session (a host that regenerates elsewhere has it only once a
    /// command needed the bodies).
    pub fn regenerated_here(&self) -> bool {
        self.documents.get(self.active).is_some_and(|open| {
            let revision = open.history.revision();
            // A moment of the motion made elsewhere is not here, whatever
            // is of the document as it is.
            if open.work.elsewhere() && open.motion_t().is_some() {
                return false;
            }
            // An assembly's bodies are its placement's.
            if open.history.current().is_assembly() {
                return open.asm.is_placed(revision);
            }
            open.regen.as_ref().is_some_and(|(at, _)| *at == revision)
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
        let remote = open.remote_wanted();
        // Scrubbed, with no scene of the document as it is: its first
        // picture first, which carries the datum planes a section lies on
        // while the view is scrubbed (`section.rs`), so that the page
        // never regenerates to find them. Not for a view made elsewhere,
        // whose stand-in is never the document as it is.
        let (doc, quality) = if !remote
            && !Arc::ptr_eq(&shown, &current)
            && self.kept_scene_of(&current).is_none()
        {
            (current, SceneQuality::Preview)
        } else {
            (shown, quality)
        };
        let view = if remote { self.view_job(&doc) } else { None };
        let bake = view.as_ref().and_then(|v| v.bake);
        let wanted = !self.has_scene_of(&doc, SceneQuality::Preview)
            || !self.has_scene_of(&doc, quality)
            || bake.is_some();
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
            && job.view.as_ref().and_then(|v| v.bake) == bake
        {
            return Some(job.token);
        }
        self.scene_token = self.scene_token.wrapping_add(1).max(1);
        self.scene_job = Some(SceneJob {
            token: self.scene_token,
            doc,
            quality,
            view,
        });
        Some(self.scene_token)
    }

    /// The job numbered `token` ([`Session::scene_job`]), written out for
    /// [`scene_begin`]. An error when it does not wait any more, or its
    /// document cannot be written.
    pub fn scene_request(&mut self, token: u64) -> Result<SceneRequest, String> {
        let job = self
            .job_of_scenes(token)
            .ok_or_else(|| format!("no job of scenes {token} waits"))?
            .clone();
        // A view made elsewhere: the state below it (with an assembly's
        // sources, in full the first time, by their digest after it) and
        // the view itself.
        let digest = job
            .view
            .as_ref()
            .and_then(|v| v.sources.as_ref())
            .map(crate::view_elsewhere::sources_digest);
        let sent = digest.is_some() && digest == self.sources_sent;
        let (document, view) = match &job.view {
            Some(view) => (
                crate::view_elsewhere::state_bytes(&view.base, view.sources.as_ref(), sent),
                view.json(),
            ),
            None => (
                craft3d_format::to_bytes(&job.doc).map_err(|e| e.to_string()),
                Value::Null,
            ),
        };
        let document = document.map_err(|e| format!("the document cannot be written out: {e}"))?;
        if digest.is_some() {
            self.sources_sent = digest;
        }
        let work = json!({
            "quality": quality_json(job.quality),
            "budget": self.scene_budget(),
            "view": view,
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
                        "{BUG}: could not be regenerated off the page's thread: the worker building it stopped: {why}"
                    ),
                );
                return true;
            }
        };
        let (of, built) = match read_part(bytes) {
            // The view's facts: kept for the stand-in; the scenes after
            // them are read against the placed parts they name.
            Ok((of @ (VIEW | BAKE), FACTS, payload)) => {
                if let Err(why) = self.view_answered(&job.doc, of == BAKE, payload) {
                    self.end_scene_job(
                        &job,
                        &format!(
                            "{BUG}: what was made off the page's thread cannot be read: {why}"
                        ),
                    );
                }
                return true;
            }
            Ok((VIEW | BAKE, _, _)) => {
                self.end_scene_job(
                    &job,
                    craft3d_math::bug_text!("a part of its answer is not the view's"),
                );
                return true;
            }
            Ok((of, SCENE, payload)) => {
                let revision = self.scene_revision_now();
                let against = self.scenes_read_against(&job.doc);
                let scene = crate::scene_wire::decode(payload, &against, revision)
                    .map(Arc::new)
                    .map_err(|why| {
                        CommandError::new(format!(
                            "{BUG}: the scene built off the page's thread cannot be read: {why}"
                        ))
                    });
                (of, scene)
            }
            // The worker had not been handed the sources this job names
            // (it was started again unseen): the job goes again, with them
            // in full; nothing of it is kept.
            Ok((_, ERROR, payload))
                if String::from_utf8_lossy(payload)
                    .contains(crate::view_elsewhere::SOURCES_MISSING)
                    && self.sources_sent.is_some() =>
            {
                self.sources_sent = None;
                self.scene_job = None;
                return true;
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
                    &format!("{BUG}: could not be regenerated off the page's thread: {why}"),
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
            craft3d_math::bug_text!(
                "the worker that regenerates off the page's thread answered without this scene"
            ),
        );
        true
    }

    /// The worker that builds the scenes was ended and is started again
    /// (or the work went home): the next job hands an assembly's sources
    /// over in full.
    pub fn scene_worker_restarted(&mut self) {
        self.sources_sent = None;
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
        // A view that never got its facts (the worker stopped, the deadline,
        // facts that do not read): its moment is not waited for any more,
        // and says why; a bake still wanted for it ends with the same error.
        if job.view.is_some() {
            self.view_failed(&job.doc, why);
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
        [of @ (VIEW | BAKE), what @ FACTS, rest @ ..] => Ok((*of, *what, rest)),
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
    selected: Option<(u64, Value, Value)>,
    /// The view of the job of scenes in hand (`view_elsewhere.rs`): the
    /// moment shown, and the bake to make after its scenes. A command run
    /// on another view puts it back.
    view: Option<View>,
    /// The joint held by hand in the view of the job of scenes in hand.
    drive: Option<crate::view_elsewhere::DriveAsk>,
    /// The quality the job of scenes in hand was asked for.
    asked: Option<SceneQuality>,
    /// Pieces of the bake in a row that left as much to do, and what the
    /// last one left.
    stalled: (u32, Option<u64>),
}

use crate::view_elsewhere::View;

/// A job of scenes as read: the state, an assembly's sources, the view,
/// the quality and the scene budget.
type ReadJob = (
    Document,
    Option<Sources>,
    (Option<View>, Option<crate::view_elsewhere::DriveAsk>),
    SceneQuality,
    Option<usize>,
);

/// Pieces of a bake in a row without progress after which the bake made
/// beside the page stops (its budget is full).
const BAKE_STALLED: u32 = 3;

/// A digest of a document's bytes: which state a selection was left on.
fn digest(bytes: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
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
        let read = (|| -> Result<ReadJob, String> {
            let (doc, sources) = crate::view_elsewhere::read_state(document)
                .map_err(|e| format!("the document cannot be read: {e}"))?;
            let work: Value =
                serde_json::from_str(work).map_err(|e| format!("the work cannot be read: {e}"))?;
            let view = (
                crate::view_elsewhere::view_of(&work),
                crate::view_elsewhere::drive_of(&work),
            );
            let quality = work
                .get("quality")
                .and_then(quality_from)
                .ok_or("the quality cannot be read")?;
            let budget = work
                .get("budget")
                .and_then(Value::as_u64)
                .and_then(|b| usize::try_from(b).ok());
            Ok((doc, sources, view, quality, budget))
        })();
        builder.left.clear();
        builder.refused = None;
        builder.scenes_of = None;
        builder.view = None;
        builder.drive = None;
        builder.stalled = (0, None);
        let mut bake = false;
        let quality = match read {
            Ok((doc, sources, (view, drive), quality, budget)) => {
                let same = builder.loaded.as_deref() == Some(document);
                let session = builder.session.get_or_insert_with(builder_session);
                if let Some(budget) = budget {
                    session.set_scene_budget(budget);
                }
                // The state it holds already (another quality of it): its
                // scenes are there.
                if !same {
                    load_state(session, doc);
                    if let (Some(sources), Some(open)) = (sources, session.documents.get_mut(0)) {
                        crate::view_elsewhere::install(open, sources);
                    }
                }
                // A job without a view shows the state as it is (a moment
                // shown for an earlier job is let go of).
                show_view(session, view.unwrap_or((None, None)), drive.as_ref());
                if let Some(view) = view {
                    builder.left.push_back((VIEW, SceneQuality::Preview));
                    bake = view.1.is_some();
                }
                if !same {
                    builder.loaded = Some(document.to_vec());
                }
                builder.view = view;
                builder.drive = drive;
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
        builder.asked = Some(quality);
        if bake {
            builder.left.push_back((BAKE, SceneQuality::Preview));
        }
    });
}

/// Shows `view` in this side's session: the moment of the motion (or the
/// document as it is), as `view.motion {t, collisions}` would.
fn show_view(session: &mut Session, view: View, drive: Option<&crate::view_elsewhere::DriveAsk>) {
    if let Some(open) = session.documents.get_mut(0) {
        let (moment, _) = view;
        open.set_motion_view(moment.map(|(t, _)| t), moment.is_some_and(|(_, c)| c));
        open.let_go_of_drive();
    }
    // A joint held by hand, as `view.drive` holds it here; one it cannot
    // hold is said by the facts (`view.drive {}` answers nothing held).
    if let Some((mate, which, value)) = drive {
        let unit = if which == "angle" { "rad" } else { "" };
        let value = if unit.is_empty() {
            json!(value)
        } else {
            json!(format!("{value} {unit}"))
        };
        let _ = session.execute(
            "view.drive",
            &json!({"mate": mate, "which": which, "value": value}),
        );
    }
}

/// A piece of the bake the view asks for: `motion.bake` a step at a
/// time, its answer (an error once it makes no progress).
fn bake_piece(
    session: &mut Session,
    (frames, collisions): (u64, bool),
    stalled: &mut (u32, Option<u64>),
) -> Value {
    let answer = match session.execute(
        "motion.bake",
        &json!({"frames": frames, "collisions": collisions, "limit": 1, "background": false}),
    ) {
        Ok(answer) => answer,
        Err(e) => {
            return json!({"error": format!("the bake beside the page failed: {}", e.message())});
        }
    };
    if answer.get("done") == Some(&Value::Bool(true)) {
        return answer;
    }
    // Progress is what is left going down: a piece that leaves as much
    // as the one before it made none.
    let remaining = answer.get("remaining").and_then(Value::as_u64);
    let (count, before) = *stalled;
    let none = match (before, remaining) {
        (Some(before), Some(now)) => now >= before,
        (_, None) => true,
        (None, Some(_)) => false,
    };
    *stalled = (if none { count.saturating_add(1) } else { 0 }, remaining);
    if stalled.0 >= BAKE_STALLED {
        return json!({
            "error": format!("the bake beside the page stopped: {BAKE_STALLED} pieces in a row left as much to make (the budget for what it keeps is likely full)"),
            "remaining": remaining,
        });
    }
    answer
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
        let asked = builder.asked.unwrap_or(quality);
        let builder = &mut *builder;
        let Some(session) = builder.session.as_mut() else {
            builder.left.pop_front();
            return SceneStep::Part(error_part(of, "no document to show"));
        };
        if of == VIEW {
            // The state below the view first, a piece at a time (the
            // moment's sweep starts from its regeneration), then the
            // moment or the placement, made in one go.
            if session.regenerate_some(stop.clone()).is_some() {
                return SceneStep::Working;
            }
            builder.left.pop_front();
            let mut part = vec![VIEW, FACTS];
            part.extend(crate::view_elsewhere::facts_bytes(session, None, None));
            return SceneStep::Part(part);
        }
        if of == BAKE {
            let Some(wanted) = builder.view.and_then(|(_, bake)| bake) else {
                builder.left.pop_front();
                return SceneStep::Done;
            };
            let answer = bake_piece(session, wanted, &mut builder.stalled);
            let finished = answer.get("done") != Some(&Value::Bool(false));
            if finished {
                builder.left.pop_front();
            }
            // The last piece goes with the view's facts made again (the
            // moment now says it is baked); the others with the bake's
            // answer alone.
            if finished {
                let mut part = vec![VIEW, FACTS];
                part.extend(crate::view_elsewhere::facts_bytes(
                    session,
                    None,
                    Some(answer),
                ));
                return SceneStep::Part(part);
            }
            let mut part = vec![BAKE, FACTS];
            part.extend(crate::view_elsewhere::bake_bytes(&answer));
            return SceneStep::Part(part);
        }
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

/// What this side's session did itself ([`Session::work_here`]): how a
/// test sees that a moment baked here is shown without meshing again.
pub fn scene_side_work() -> Option<crate::WorkCount> {
    BUILDER.with(|builder| {
        builder
            .try_borrow()
            .ok()?
            .session
            .as_ref()
            .map(Session::work_here)
    })
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
    run_on_state(document, work, |session, command, params| {
        session.execute(command, params)
    })
}

/// [`run_command`] with the running given: `run` gets this side's session
/// with the document state and the selection the request was written
/// with, and the command's name and parameters from it, and does what a
/// host's own job there does (`craft3d-web`'s export, its choices
/// resolved where the bodies are, [`Session::hand_over`]). An edit it
/// makes comes back as the undo steps it made, each with the document
/// after it, for the host to make the same ([`Session::command_answered`]).
/// Never panics on any input; a panic inside `run` is caught.
pub fn run_on_state(
    document: &[u8],
    work: &str,
    run: impl FnOnce(&mut Session, &str, &Value) -> Result<Value, CommandError>,
) -> crate::CommandAnswer {
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
        // Chained only when the host says the command follows one still
        // waiting there, on this very state and selection.
        let follows = work.get("follows") == Some(&Value::Bool(true));
        let state = digest(document);
        let wire = match &builder.selected {
            Some((on, input, output)) if follows && *on == state && *input == read => {
                output.clone()
            }
            _ => read.clone(),
        };
        let selection = crate::selection::Selection::from_wire(&wire, None).unwrap_or_default();
        let session = builder.session.get_or_insert_with(builder_session);
        if builder.loaded.as_deref() != Some(document) {
            let (doc, sources) = crate::view_elsewhere::read_state(document)
                .map_err(|e| format!("the document cannot be read: {e}"))?;
            load_state(session, doc);
            if let (Some(sources), Some(open)) = (sources, session.documents.get_mut(0)) {
                crate::view_elsewhere::install(open, sources);
            }
            builder.loaded = Some(document.to_vec());
        }
        // A read on a moment of the motion: shown here first (and the view
        // of the scenes in hand shown again after it).
        // Only a read carries a view; any other job leaves the one shown
        // (and the bake made for it) as it is.
        let shown_before = session
            .documents
            .first()
            .map(|open| (open.motion_t(), open.motion_collisions()));
        let asked = crate::view_elsewhere::view_of(&work).map(|(moment, _)| moment);
        let moved = asked.is_some_and(|moment| {
            shown_before != Some((moment.map(|(t, _)| t), moment.is_some_and(|(_, c)| c)))
        });
        if let (true, Some(moment)) = (moved, asked) {
            show_view(session, (moment, None), None);
        }
        let open = session.documents.get_mut(0).ok_or("no document")?;
        open.selection = selection;
        open.jig_in_use = work.get("jigInUse") == Some(&Value::Bool(true));
        let id = open.id;
        session.window_camera = work
            .get("camera")
            .and_then(crate::commands_elsewhere::camera_from)
            .map(|camera| (id, camera));
        let revision = open.history.revision();
        for name in session.file_names() {
            session.take_file(&name);
        }
        session.check_selection();
        // An instance's edit reads its source as a file, here as where it
        // was asked: the files read for the assembly there are files here
        // for that command (taken out again before what it wrote is).
        let inputs = if command.starts_with("instance.") {
            source_inputs(session)
        } else {
            Vec::new()
        };
        let before = session.documents.first().map(|o| o.selection.to_wire());
        session.edit_log = Some(Vec::new());
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run(session, &command, &params)
        }))
        .unwrap_or_else(|_| {
            Err(CommandError::new(format!(
                "{command}: {BUG}: {}",
                crate::PANICKED
            )))
        });
        for name in &inputs {
            session.take_file(name);
        }
        // A trace made here for the host: it goes with the answer, bit for
        // bit, and is not kept here (the host keeps its traces).
        let result = match result {
            Ok(mut value) if command == "view.trace" => {
                let name = value
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let wire = name.and_then(|name| {
                    session
                        .documents
                        .first_mut()
                        .and_then(|open| open.take_trace_wire(&name))
                });
                if let (Some(wire), Some(map)) = (wire, value.as_object_mut()) {
                    map.insert("trace".into(), wire);
                }
                Ok(value)
            }
            other => other,
        };
        let mut steps = session.edit_log.take().unwrap_or_default();
        let edited = session.documents.len() != 1
            || session
                .documents
                .first()
                .is_none_or(|open| open.history.revision() != revision);
        let after = session.documents.first().map(|o| o.selection.to_wire());
        let selection = (after != before).then_some(after).flatten();
        // The edit as the steps it made (also those that changed nothing:
        // under a key they start a run): only when they are the whole of
        // it (the one document, its last step the state it is in now), at
        // most as many as a history keeps, and each state crossing the
        // file as it is (the host would otherwise take another one).
        let whole = !steps.is_empty()
            && steps.len() <= craft3d_ops::DEFAULT_LIMIT
            && session.documents.len() == 1
            && steps.last().is_some_and(|step| {
                session
                    .documents
                    .first()
                    .is_some_and(|open| Arc::ptr_eq(&step.after, open.history.current()))
            });
        let mut documents = Vec::new();
        if whole {
            for step in &steps {
                let crossed = craft3d_format::to_bytes(&step.after).ok().filter(|bytes| {
                    craft3d_format::from_bytes(bytes).is_ok_and(|back| back == *step.after)
                });
                match crossed {
                    Some(bytes) => documents.push(bytes),
                    None => {
                        documents.clear();
                        break;
                    }
                }
            }
        }
        if documents.len() != steps.len() {
            steps.clear();
            documents.clear();
        }
        builder.selected = match &selection {
            Some(left) => Some((state, read, left.clone())),
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
        // The scenes in hand are of the state they were asked for, with
        // their view.
        if let Some(scenes_of) = builder.scenes_of.clone()
            && builder.loaded.as_deref() != Some(&scenes_of[..])
            && let Ok((doc, sources)) = crate::view_elsewhere::read_state(&scenes_of)
        {
            load_state(session, doc);
            if let (Some(sources), Some(open)) = (sources, session.documents.get_mut(0)) {
                crate::view_elsewhere::install(open, sources);
            }
            builder.loaded = Some(scenes_of);
        }
        if moved && builder.scenes_of.is_some() {
            show_view(
                session,
                builder.view.unwrap_or((None, None)),
                builder.drive.as_ref(),
            );
        }
        Ok::<_, String>(crate::CommandAnswer {
            answer: crate::commands_elsewhere::answer_json(result, edited, selection, &steps),
            files,
            documents,
        })
    });
    answered.unwrap_or_else(|why: String| crate::CommandAnswer {
        answer: crate::commands_elsewhere::answer_json(
            Err(CommandError::new(format!(
                "{BUG}: it could not be run beside the page: {why}"
            ))),
            false,
            None,
            &[],
        ),
        files: Vec::new(),
        documents: Vec::new(),
    })
}

/// The files read for the active assembly, put in this side's memory
/// under their paths so a command reads them as the host's session did;
/// the names put there.
fn source_inputs(session: &mut Session) -> Vec<String> {
    let files: Vec<(String, Vec<u8>)> = session
        .documents
        .first()
        .map(|open| {
            open.asm
                .files
                .iter()
                .filter_map(|(path, read)| {
                    Some((path.clone(), read.as_ref().ok()?.bytes().to_vec()))
                })
                .collect()
        })
        .unwrap_or_default();
    files
        .into_iter()
        .filter_map(|(path, bytes)| session.put_file(&path, bytes).ok().map(|()| path))
        .collect()
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

    /// A command that panics where the bodies are is answered as a crash:
    /// the bug's words and that it panicked.
    #[test]
    fn a_command_that_panics_there_is_answered_as_a_crash() {
        let document = craft3d_format::to_bytes(&craft3d_doc::Document::default()).unwrap();
        let work = json!({"command": "file.new", "params": {}}).to_string();
        let answer = run_on_state(&document, &work, |_, _, _| panic!("fell over on purpose"));
        assert!(
            answer.answer.contains(crate::BUG) && answer.answer.contains(crate::PANICKED),
            "{}",
            answer.answer
        );
        scene_forget();
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
