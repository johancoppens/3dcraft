//! Assemblies, derived (docs/design/assemblies.md): what an open
//! assembly shows is worked out here from its document and the source
//! files read for it, and may be thrown away at any time (rule 2).
//!
//! The pipeline, per state of the document:
//!
//! 1. each instance's source: a file read earlier ([`AsmState::files`],
//!    by [`crate::Session::read_sources`], the only part that touches
//!    files), or a document packed into the assembly;
//! 2. each distinct configuration (the source's bytes and the values its
//!    public parameters are given) regenerated once and cached, in
//!    parallel, merged in the order of their keys;
//! 3. each mate's two frames, from the geometry the mate names through
//!    its instance (`Instance/Feature:name`), in that instance's own
//!    coordinates;
//! 4. solved (`asm_solve`, track B's `craft3d-assembly`);
//! 5. the bodies placed by their instance's pose, by transforms only.
//!
//! The result is a part ([`Placed::doc`], one feature per placed body,
//! named after its instance) with its regeneration ([`Placed::regen`]):
//! the scene, picking, selection, export and the assertions read it as
//! they read any part.

use crate::asm_solve as solver;
use crate::selection::{Resolver, SelectionKind};
use craft3d_doc::{
    Assembly, Blob, BodyProperties, Document, Feature, FeatureId, FeatureKind, Import, Instance,
    MateKind, MateRef, Source, WORLD,
};
use craft3d_expr::{Dimension, Quantity};
use craft3d_math::{Transform, Vec3};
use craft3d_regen::{FeatureResult, Names, Regen, Status};
use craft3d_topo::Body;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// How deep subassemblies nest, at most.
pub(crate) const MAX_DEPTH: usize = 8;
/// Most source files one assembly reads, its subassemblies' included.
pub(crate) const MAX_SOURCE_FILES: usize = 1_000;
/// Most bytes of configurations an [`AsmState`] keeps by default
/// (estimated by [`built_bytes`]): beyond it the least recently used are
/// let go of, never one the last placement used. Enough for the frames of
/// a motion through two hundred configurations of a part of a thousand
/// faces (a belt whose loop follows `t`: 4.4 MB resident each, measured;
/// the router's 120 frames ask for 170), so a motion played again builds
/// nothing. Less in the browser, whose memory is smaller and shared with
/// the page.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) const DEFAULT_CONFIG_BUDGET: usize = 1 << 30;
#[cfg(target_arch = "wasm32")]
pub(crate) const DEFAULT_CONFIG_BUDGET: usize = 128 << 20;
/// Most variable names a drive's dependencies are followed through.
const MAX_DEPENDS: usize = 10_000;

/// What an open assembly has derived, kept between commands. Not saved.
#[derive(Clone, Default)]
pub(crate) struct AsmState {
    /// The source files read, by their path as [`join`] writes it (from
    /// the read root): their bytes, or why they could not be read.
    pub files: BTreeMap<String, Result<Blob, String>>,
    /// What each source file looked like on disk when it was read (its
    /// size and modification time), so a check for changes reads again
    /// only those that look changed (`Session::follow_sources`). None for
    /// files in memory, which are read again to be compared.
    pub stamps: BTreeMap<String, crate::recovery::FileStamp>,
    /// Sources that changed but could not be read again (gone,
    /// unreadable, refused by the roots), by path: why. Their last bytes
    /// read stay in `files`, so their instances keep the last good bodies.
    pub unfollowed: BTreeMap<String, String>,
    /// Source documents, by the SHA-256 of their bytes.
    pub(crate) parsed: BTreeMap<String, Result<Arc<Document>, String>>,
    /// Configurations built, by [`config_key`].
    configs: BTreeMap<String, Config>,
    /// Per source that is given parameters (its hash and the length unit
    /// it is built in, [`Job::source`]): the regeneration cache of the
    /// last configuration built of it, so the next configuration of that
    /// source rebuilds only the features its changed parameters reach.
    regen_caches: BTreeMap<String, craft3d_regen::RegenCache>,
    /// Counts placements and builds, for which configuration was used
    /// least recently.
    tick: u64,
    /// About how many bytes the configurations kept take ([`built_bytes`]).
    config_bytes: usize,
    /// Most bytes of configurations kept; `None` for
    /// [`DEFAULT_CONFIG_BUDGET`].
    config_budget: Option<usize>,
    /// Raised whenever `files` changes, so the placement is redone.
    pub generation: u64,
    /// The last placement: for the document revision and the generation.
    placed: Option<(u64, u64, Arc<Placed>)>,
    /// How many configurations were regenerated so far (a count for
    /// `assembly.inspect` and the tests: two instances of one
    /// configuration cost one).
    pub builds: u64,
    /// The bodies the last placements placed, by the address of their
    /// source body ([`PlacedCopies`]): a body whose source and pose did
    /// not change since is the same `Arc`, not a new copy.
    placed_copies: PlacedCopies,
    /// Configurations of instances mated by their datums alone are kept
    /// slim ([`slimmed`]): set in a motion's own copy of the state, whose
    /// many configurations are shown and solved, never measured or
    /// selected in.
    pub slim: bool,
    /// Configurations a bake in pieces needs together (the batch it is
    /// building, `motion.bake {limit}`): never let go of by the budget
    /// until the batch is done, so a batch larger than the budget still
    /// completes (it is held one batch beyond the budget, then let go of).
    pinned: BTreeSet<String>,
    /// Configurations being built elsewhere (a motion's builder): a
    /// placement neither works them out nor builds them; one made at
    /// once ([`Self::place_ready`]) shows another of the instance's.
    pub deferred: BTreeSet<String>,
    /// Asked between the features of each part it builds, on a copy of a
    /// motion's state in the builder (`motion_build.rs`): true when the
    /// run was let go of, and then the build stops there and its
    /// configuration is not kept (`craft3d_regen::progress::with_stop`).
    /// `None` everywhere else.
    pub cancel: Option<crate::motion_build::Cancel>,
    /// The folder the sources of an assembly without a file were read
    /// from (`Session::read_sources`: the current folder with access to
    /// every file, the read root's top otherwise): its sources are
    /// placed from there until it is saved, as they were read.
    pub unsaved_base: String,
}

/// The bodies the last two placements placed (`build_shown`), by the
/// address of the source body each is a copy of: the source (held, so its
/// address is not reused), the pose and the copy, with the placement that
/// used it last. A body placed again by the same pose is taken from here:
/// a copy is a function of its source and its pose, so it is the same
/// body either way, and a moment of a motion that leaves most instances
/// where they were copies, meshes and describes only those that moved
/// (docs/design/motion.md › *Costs*). Bounded by what two placements
/// place.
#[derive(Clone, Default)]
struct PlacedCopies {
    by_source: BTreeMap<usize, Vec<PlacedCopy>>,
    /// Counts the placements of the top assembly.
    placement: u64,
}

#[derive(Clone)]
struct PlacedCopy {
    source: Arc<Body>,
    pose: Transform,
    placed: Arc<Body>,
    used: u64,
}

impl PlacedCopies {
    /// `body` placed by `pose`: the copy kept when there is one, else a
    /// new one, noted for the meshes (`meshes::note_placed`) and kept.
    fn place(&mut self, body: &Arc<Body>, pose: &Transform) -> Result<Arc<Body>, String> {
        let used = self.placement;
        let copies = self
            .by_source
            .entry(Arc::as_ptr(body) as usize)
            .or_default();
        // Each copy once per placement: two instances placed alike are
        // two bodies, as they would be without this.
        if let Some(copy) = copies
            .iter_mut()
            .find(|c| c.used != used && Arc::ptr_eq(&c.source, body) && c.pose == *pose)
        {
            copy.used = used;
            return Ok(Arc::clone(&copy.placed));
        }
        let placed = Arc::new(body.transformed(pose).map_err(|e| e.to_string())?);
        // Meshed as its source's mesh moved (`meshes.rs`): a moment of a
        // motion meshes nothing.
        crate::meshes::note_placed(&placed, body, pose);
        copies.push(PlacedCopy {
            source: Arc::clone(body),
            pose: *pose,
            placed: Arc::clone(&placed),
            used,
        });
        Ok(placed)
    }

    /// A placement of the top assembly begins.
    fn begin(&mut self) {
        self.placement = self.placement.wrapping_add(1);
    }

    /// A placement of the top assembly ended: what neither it nor the one
    /// before it used is let go of.
    fn end(&mut self) {
        let last = self.placement.wrapping_sub(1);
        let now = self.placement;
        for copies in self.by_source.values_mut() {
            copies.retain(|c| c.used == now || c.used == last);
        }
        self.by_source.retain(|_, copies| !copies.is_empty());
    }
}

/// A configuration kept: what was built (or why it could not be), about
/// how many bytes it holds, and when it was last used ([`AsmState::tick`]).
#[derive(Clone)]
struct Config {
    built: Result<Arc<Built>, String>,
    bytes: usize,
    used: u64,
    /// Placed with its bodies at least once (a moment shown, the assembly
    /// at rest), not only solved: a grid point of a motion needs its
    /// configurations for its solve alone, once, so those go first.
    shown: bool,
    /// Kept without the bodies that went into later features
    /// ([`slimmed`]): enough for an instance mated by its datums alone.
    slim: bool,
    /// Used by a solve without bodies (a motion's grid point).
    solved: bool,
}

/// What [`AsmState::prebuild`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Prebuilt {
    /// Distinct configurations the states ask for.
    pub wanted: usize,
    /// Of those, built now (failed ones included).
    pub built: usize,
    /// Of those, that failed.
    pub failed: usize,
    /// Not built because of the limit: asked for again, they are built.
    pub remaining: usize,
}

impl Prebuilt {
    /// Adds what another call built (`wanted` is left as it is).
    pub(crate) fn add(&mut self, other: &Prebuilt) {
        self.built = self.built.saturating_add(other.built);
        self.failed = self.failed.saturating_add(other.failed);
        self.remaining = other.remaining;
    }
}

/// About how many bytes a configuration holds: every distinct body of its
/// regeneration ([`craft3d_regen::body_bytes`]) and as much again for
/// their names and the regeneration's own records (a measured belt of 940
/// faces: see docs/design/motion.md › *Configurations over a motion*).
fn built_bytes(built: &Result<Arc<Built>, String>) -> usize {
    let Ok(built) = built else {
        return RECORDS;
    };
    regen_bytes(match &**built {
        Built::Part { regen, .. } => regen,
        Built::Assembly(placed) => &placed.regen,
    })
}

/// What a kept state is counted as beyond its bodies: its document and
/// the records around it.
const RECORDS: usize = 4096;

/// About how many bytes a regeneration holds: every distinct body
/// ([`craft3d_regen::body_bytes`]) and as much again for their names and
/// the regeneration's own records.
pub(crate) fn regen_bytes(regen: &Regen) -> usize {
    let mut seen: BTreeSet<usize> = BTreeSet::new();
    let mut bytes = 0usize;
    for feature in &regen.features {
        if let Some(body) = &feature.body
            && seen.insert(Arc::as_ptr(body) as usize)
        {
            bytes = bytes.saturating_add(craft3d_regen::body_bytes(body));
        }
    }
    bytes.saturating_mul(2).saturating_add(RECORDS)
}

/// A configuration built: a part regenerated with its parameters set, or
/// a subassembly placed.
pub(crate) enum Built {
    Part {
        doc: Arc<Document>,
        regen: Arc<Regen>,
    },
    Assembly(Arc<Placed>),
}

/// An instance as placed.
#[derive(Clone)]
pub(crate) struct InstanceState {
    pub name: String,
    /// Its source as a viewer lists it: the file's path as the assembly
    /// holds it, or `packed`.
    pub source: String,
    /// Why the instance is not placed, when it is not.
    pub error: Option<String>,
    /// Its configuration's key (source hash and parameters).
    pub config: Option<String>,
    /// The SHA-256 of the bytes the instance was built from.
    pub hash: Option<String>,
    /// The source file's bytes differ from the hash the document holds.
    pub changed: bool,
    /// Why it does not show what its source file holds now, while it
    /// still shows a version of it: pinned to another, or the file
    /// changed and could not be read again (the last version read stays).
    pub held: Option<String>,
    pub pose: Transform,
    /// The features of [`Placed::doc`] that hold its bodies.
    pub bodies: Vec<FeatureId>,
    pub built: Option<Arc<Built>>,
    /// The picked body's feature in the source, when one is picked.
    pub picked: Option<FeatureId>,
    /// A flexible subassembly: its own instances take its place in the
    /// solve (named `Sub/Inner`), with its mates; it is not solved itself.
    pub flexed: bool,
    /// For an instance of a flexible subassembly: that subassembly's
    /// instance name.
    pub flex_of: Option<String>,
}

/// A mate as solved.
#[derive(Clone)]
pub(crate) struct MateState {
    pub name: String,
    pub kind: MateKind,
    /// Why the mate takes no part, when it does not.
    pub error: Option<String>,
    /// The value of its main free motion after the solve (radians or
    /// model units), for a mate that has one.
    pub free: Option<f64>,
    /// Every free motion's value after the solve, in the order of its
    /// kind's free motions (radians or model units); empty when it was
    /// not solved.
    pub values: Vec<f64>,
    pub driven: bool,
    /// Its main free motion is free in the solve: driving it (`joint.set`)
    /// takes a degree of freedom and contradicts nothing. False when the
    /// solve failed.
    pub main_free: bool,
}

/// A coupling as solved: why it takes no part, when it does not.
#[derive(Clone)]
pub(crate) struct CouplingState {
    pub name: String,
    pub error: Option<String>,
}

/// An assembly placed.
#[derive(Clone)]
pub(crate) struct Placed {
    /// A part with one feature per placed body: what is drawn, picked,
    /// exported and measured.
    pub doc: Arc<Document>,
    pub regen: Arc<Regen>,
    pub instances: Vec<InstanceState>,
    pub mates: Vec<MateState>,
    pub couplings: Vec<CouplingState>,
    /// Why the solve failed, when it did (the instances then stay at
    /// their placements).
    pub solve_error: Option<String>,
    /// No mate reaches the world: the first instance was held where it
    /// is placed.
    pub floating: bool,
    /// When solved: the degrees of freedom left in the whole assembly, and
    /// each free motion in words.
    pub dof: Option<(usize, Vec<String>)>,
    /// Its own mates as the solver took them, each side by the name of
    /// its instance (`None`: the world): what a parent that holds it as a
    /// flexible subassembly solves with its own.
    pub specs: Vec<NamedSpec>,
    /// Its couplings as the solver took them, their mates indices into
    /// [`Placed::specs`]: a parent that holds it as a flexible
    /// subassembly solves them with its own.
    pub coupling_specs: Vec<solver::CouplingSpec>,
    /// The state the solve reached, for the next moment of a motion to
    /// be carried from (`motion.rs`); `None` when it failed.
    pub carried: Option<Carried>,
    /// A moment of a motion that could not be carried from the moment
    /// before it (the instances or the driven values taking part changed
    /// in between) and was solved from the placements instead: why. It
    /// may then lie on another branch than the motion's.
    pub solve_note: Option<String>,
    /// The instances shown with another configuration of theirs than
    /// the one their parameters ask for, which is being built elsewhere
    /// (`view.motion {shapes: "ready"}`, `motion_build.rs`); empty for an
    /// exact placement.
    pub stale: Vec<String>,
}

/// A solved state of an assembly's motion: the poses of the instances
/// that took part in the solve (in their order) and the driven values
/// they were solved at. The next moment starts from it
/// ([`AsmState::place_moment`]).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Carried {
    pub poses: Vec<Transform>,
    pub drives: Vec<f64>,
    /// Every mate's values there, per spec (angles not wrapped: the turns
    /// a joint had come, which the poses do not say).
    pub values: Vec<Vec<f64>>,
    /// The degrees of freedom left there.
    pub dof: usize,
}

/// A joint held by hand for the view (`view.drive`, `drive_cmds.rs`):
/// the drivers of the mates in `release` are let go of, and the main free
/// motion of `held`'s mate (when given) is driven at its value. The
/// solve is carried from the document's placement, the held motion moved
/// from `start` (its value there) to the value along the branch.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Hold {
    pub held: Option<Held>,
    pub release: BTreeSet<String>,
}

/// The joint a [`Hold`] drives: the mate, which of its free motions (by
/// index), the value to drive it to and its value at the start (radians
/// or model units).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Held {
    pub mate: String,
    pub slot: usize,
    pub value: f64,
    pub start: f64,
}

/// A mate as the solver takes it, its two sides by instance name (`None`
/// for the world) rather than by index.
#[derive(Clone)]
pub(crate) struct NamedSpec {
    pub a: Option<String>,
    pub b: Option<String>,
    pub spec: solver::MateSpec,
    /// Per driven free motion: the variables of the assembly that solved
    /// it which its driven value depends on (its own variable among them).
    /// A parent that holds it as a flexible subassembly keeps the drive
    /// only when it sets one of them ([`flex`]).
    pub drive_deps: [BTreeSet<String>; solver::VARIABLES],
    /// A mate of this assembly's own (listed first), not one a flexible
    /// subassembly of it brought (which keeps its own name).
    pub own: bool,
}

impl Placed {
    pub(crate) fn instance(&self, name: &str) -> Option<&InstanceState> {
        self.instances.iter().find(|i| i.name == name)
    }
}

/// The folder of a path, with `/` or `\` between its parts: empty for a
/// file name alone.
pub(crate) fn dir_of(path: &str) -> String {
    match path.rfind(['/', '\\']) {
        Some(0) => "/".to_string(),
        Some(at) => path.get(..at).unwrap_or_default().to_string(),
        None => String::new(),
    }
}

/// `relative` (parts between `/`, `..` allowed) beneath `dir`, with `..`
/// and `.` taken out as text. An error when `..` climbs out of a relative
/// `dir`'s start.
pub(crate) fn join(dir: &str, relative: &str) -> Result<String, String> {
    let absolute = dir.starts_with('/')
        || dir.starts_with('\\')
        || (dir.is_empty() && relative.starts_with('/'));
    let mut parts: Vec<&str> = dir
        .split(['/', '\\'])
        .filter(|p| !p.is_empty() && *p != ".")
        .collect();
    let mut climbs = 0usize;
    for part in relative.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    if absolute {
                        return Err(format!("`{relative}` climbs above the root"));
                    }
                    climbs = climbs.saturating_add(1);
                }
            }
            other => parts.push(other),
        }
    }
    let mut out = String::new();
    if absolute {
        out.push('/');
    }
    let mut first = true;
    for _ in 0..climbs {
        if !first {
            out.push('/');
        }
        out.push_str("..");
        first = false;
    }
    for part in parts {
        if !first {
            out.push('/');
        }
        out.push_str(part);
        first = false;
    }
    Ok(out)
}

/// The path from folder `from` to `to` (both as [`join`] writes them),
/// with `/` between its parts.
pub(crate) fn relative_to(from: &str, to: &str) -> String {
    let split = |p: &str| -> Vec<String> {
        p.split(['/', '\\'])
            .filter(|s| !s.is_empty() && *s != ".")
            .map(str::to_string)
            .collect()
    };
    let (a, b) = (split(from), split(to));
    let common = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let mut parts: Vec<String> = vec!["..".to_string(); a.len().saturating_sub(common)];
    parts.extend(b.into_iter().skip(common));
    parts.join("/")
}

/// A rigid pose from a placement: the rotation vector (degrees), then
/// the position.
pub(crate) fn placement_pose(p: &craft3d_doc::Placement) -> Transform {
    let [rx, ry, rz] = p.rotation;
    let r = Vec3::new(rx, ry, rz);
    let angle = r.length().to_radians();
    let rotation = r
        .normalized()
        .and_then(|axis| Transform::rotation(axis, angle))
        .unwrap_or(Transform::IDENTITY);
    let [x, y, z] = p.position;
    rotation.then(&Transform::translation(Vec3::new(x, y, z)))
}

/// A quantity written as an expression a source document reads: in its
/// base units (millimetres, radians).
fn source_text(q: Quantity) -> Result<String, String> {
    if !q.value.is_finite() {
        return Err("is not finite".to_string());
    }
    if q.dimension.is_dimensionless() {
        Ok(format!("{}", q.value))
    } else if q.dimension == Dimension::LENGTH {
        Ok(format!("{} mm", q.value))
    } else if q.dimension == Dimension::ANGLE {
        Ok(format!("{} rad", q.value))
    } else {
        Err("has a unit other than a length or an angle, which a parameter override cannot pass on yet".to_string())
    }
}

/// The key of a configuration: the source's hash and each parameter
/// given, by name, as the text it was set to.
pub(crate) fn config_key(hash: &str, params: &BTreeMap<String, String>) -> String {
    let mut key = hash.to_string();
    for (name, text) in params {
        key.push('|');
        key.push_str(name);
        key.push('=');
        key.push_str(text);
    }
    key
}

/// The values of the assembly's variables, in base units.
fn evaluation(doc: &Document) -> Option<craft3d_expr::Evaluation> {
    doc.variable_table().ok().map(|t| t.evaluate())
}

fn eval(values: Option<&craft3d_expr::Evaluation>, source: &str) -> Result<Quantity, String> {
    match values {
        Some(values) => values.evaluate(source).map_err(|e| e.to_string()),
        None => craft3d_expr::Expr::parse(source)
            .and_then(|e| e.eval(|_| None))
            .map_err(|e| e.to_string()),
    }
}

/// `source` evaluated with `doc`'s variables, in base units
/// (millimetres, radians).
pub(crate) fn evaluate_in(doc: &Document, source: &str) -> Result<Quantity, String> {
    eval(evaluation(doc).as_ref(), source)
}

/// A length in base units (mm) in `doc`'s length unit.
pub(crate) fn in_units(doc: &Document, millimetres: f64) -> f64 {
    let factor = doc.units.length.as_expr_unit().factor();
    if factor.is_finite() && factor > 0.0 {
        millimetres / factor
    } else {
        millimetres
    }
}

/// What a source reads as: its document, parsed once per hash.
fn parse(state: &mut AsmState, blob: &Blob) -> Result<Arc<Document>, String> {
    let hash = blob.sha256();
    if let Some(found) = state.parsed.get(&hash) {
        return found.clone();
    }
    let parsed = craft3d_format::from_bytes(blob.bytes())
        .map(Arc::new)
        .map_err(|e| e.to_string());
    state.parsed.insert(hash, parsed.clone());
    parsed
}

/// The bytes of an instance's source, whether they differ from the hash
/// the document holds, and why it does not follow its file when it does
/// not ([`InstanceState::held`]).
fn source_bytes(
    assembly: &Assembly,
    instance: &Instance,
    base: &str,
    files: &BTreeMap<String, Result<Blob, String>>,
    unfollowed: &BTreeMap<String, String>,
) -> Result<(Blob, bool, Option<String>), String> {
    let mut held = None;
    let (blob, recorded) = match &instance.source {
        Source::Packed { hash } => (
            assembly
                .packed(hash)
                .cloned()
                .ok_or_else(|| format!("the packed document {hash} is missing"))?,
            hash,
        ),
        Source::File { path, hash } => {
            let full = join(base, path)?;
            if let Some(why) = unfollowed.get(&full) {
                held = Some(format!(
                    "its source `{path}` changed and could not be read again ({why}); it shows the last version read"
                ));
            }
            let blob = match files.get(&full) {
                Some(Ok(blob)) => blob.clone(),
                Some(Err(why)) => return Err(format!("its source `{path}` cannot be read: {why}")),
                None => {
                    return Err(format!(
                        "its source `{path}` has not been read (assembly.reload reads it)"
                    ));
                }
            };
            (blob, hash)
        }
    };
    let now = blob.sha256();
    if let Some(pinned) = &instance.pinned
        && *pinned != now
    {
        // A pinned instance never follows: the packed copy of the pinned
        // version is used when the file holds one.
        if let Some(packed) = assembly.packed(pinned) {
            let note = match &instance.source {
                Source::File { path, .. } => format!(
                    "pinned to {}…: its source `{path}` changed and is not followed (instance.edit with pinned: null follows it)",
                    pinned.get(..12).unwrap_or(pinned)
                ),
                Source::Packed { .. } => format!(
                    "pinned to {}…, not to the packed version it names",
                    pinned.get(..12).unwrap_or(pinned)
                ),
            };
            return Ok((packed.clone(), false, held.or(Some(note))));
        }
        return Err(format!(
            "it is pinned to {}…, and its source is now {}… (instance.edit with pinned: null follows it)",
            pinned.get(..12).unwrap_or(pinned),
            now.get(..12).unwrap_or(&now)
        ));
    }
    Ok((blob, *recorded != now, held))
}

/// The parameter values an instance gives its source, written for it.
fn override_values(
    instance: &Instance,
    values: Option<&craft3d_expr::Evaluation>,
) -> Result<BTreeMap<String, String>, String> {
    let mut out = BTreeMap::new();
    for p in &instance.params {
        let q = eval(values, p.expr.source())
            .map_err(|e| format!("its parameter `{}` = {}: {e}", p.name, p.expr.source()))?;
        let text = source_text(q).map_err(|e| format!("its parameter `{}` {e}", p.name))?;
        out.insert(p.name.clone(), text);
    }
    Ok(out)
}

/// The source with its public parameters set, then in the length unit
/// `unit` (as large as it is: a part in another unit is converted as
/// `document.setUnit {keep: sizes}` converts it, proven). The parameters
/// are set first, in the source's own unit: a value given is written with
/// its unit, so the conversion keeps it.
fn configured(
    source: &Document,
    params: &BTreeMap<String, String>,
    unit: craft3d_doc::LengthUnit,
) -> Result<Document, String> {
    let mut doc = source.clone();
    if !params.is_empty() {
        let given: Map<String, Value> = params
            .iter()
            .map(|(k, v)| (k.clone(), Value::String(v.clone())))
            .collect();
        let (sources, _) = crate::param_cmds::prepare_set("instance", source, &given, false)
            .map_err(|e| e.message().trim_start_matches("instance: ").to_string())?;
        for (name, text) in &sources {
            doc.set_variable(name, text).map_err(|e| e.to_string())?;
        }
    }
    crate::unit_cmds::in_unit(doc, unit).map_err(|e| {
        format!(
            "its source is in {} and cannot be taken to the assembly's {}: {e}",
            source.units.length.symbol(),
            unit.symbol()
        )
    })
}

/// An instance's source document and the parameters it gives it, and
/// whether its file changed since the document took its hash.
#[allow(clippy::too_many_arguments)]
fn prepare(
    assembly: &Assembly,
    instance: &Instance,
    base: &str,
    files: &BTreeMap<String, Result<Blob, String>>,
    chain: &[String],
    state: &mut AsmState,
    doc: &Document,
    values: Option<&craft3d_expr::Evaluation>,
) -> Result<Prepared, String> {
    let (blob, changed, held) = source_bytes(assembly, instance, base, files, &state.unfollowed)?;
    let hash = blob.sha256();
    if chain.contains(&hash) {
        return Err(
            "its source contains this assembly: a subassembly cycle is refused".to_string(),
        );
    }
    let source = parse(state, &blob)?;
    if source.is_assembly() && chain.len() >= MAX_DEPTH {
        return Err(format!("subassemblies nest deeper than {MAX_DEPTH} levels"));
    }
    // A part in another length unit is regenerated in the assembly's
    // (`configured`); a subassembly's own mates and placements would have
    // to be converted too, which is owed.
    if source.units.length != doc.units.length && source.is_assembly() {
        return Err(format!(
            "its source is an assembly in {} and this one is in {}: a subassembly in another length unit is not supported yet; set its unit with document.setUnit",
            source.units.length.symbol(),
            doc.units.length.symbol()
        ));
    }
    let params = override_values(instance, values)?;
    Ok((hash, source, params, changed, held))
}

/// A body an instance shows: what to call it, the body in the instance's
/// own coordinates, its names and the source's appearance and material.
pub(crate) type ShownBody = (
    String,
    Arc<Body>,
    Option<Arc<Names>>,
    Option<BodyProperties>,
);

/// An instance's source: its hash, its document, the parameters it is
/// given, whether its file changed, and why it does not follow it.
type Prepared = (
    String,
    Arc<Document>,
    BTreeMap<String, String>,
    bool,
    Option<String>,
);

/// What one configuration needs to be built: its source document (set)
/// and, for a subassembly, the folder its own sources are relative to.
struct Job {
    key: String,
    doc: Result<Document, String>,
    base: String,
    /// For a part given parameters: its source's hash and the length unit
    /// it is built in, under which the regeneration cache of its last
    /// configuration is kept ([`AsmState::regen_caches`]).
    source: Option<String>,
    /// Kept slim ([`slimmed`]): no mate needs more of it than its datums.
    slim: bool,
}

/// `regen` without the bodies (and their names) that went into later
/// features: what is left is what an instance shows ([`bodies_of`]), its
/// datums and its sketches, which is all a placement needs of a
/// configuration whose instances are mated by their datums alone (a
/// mate on a face or an edge resolves its name through the bodies that
/// went into the body, `selection::Resolver`). A motion's configurations
/// are kept so (`AsmState::slim`): a belt whose loop follows `t` keeps
/// 0.74 MB of bodies instead of 1.97 MB (docs/design/motion.md › *Memory*).
pub(crate) fn slimmed(mut regen: Regen) -> Regen {
    for feature in &mut regen.features {
        if feature.consumed_by.is_some() {
            feature.body = None;
            feature.names = None;
        }
    }
    regen
}

/// True when every mate of `assembly` on the instance `name` holds to a
/// datum of its source `source` (no face, edge or vertex), so a slim
/// configuration of it places it as a whole one does.
fn mated_by_datums(assembly: &Assembly, name: &str, source: &Document) -> bool {
    assembly
        .mates
        .iter()
        .flat_map(|mate| [&mate.a, &mate.b])
        .filter(|side| side.instance == name)
        .all(|side| {
            crate::selection::feature_named(source, &side.name)
                .and_then(|id| source.feature(id))
                .is_some_and(|f| crate::selection::is_datum(&f.kind))
        })
}

/// The placement of `doc` (an assembly) with the files read for it.
/// `chain` holds the hashes of the assemblies above it, for the cycle
/// check; `depth` how deep it is.
pub(crate) fn place(
    doc: &Document,
    base: &str,
    state: &mut AsmState,
    chain: &mut Vec<String>,
) -> Placed {
    place_from(doc, base, state, chain, None, true, None, None)
}

/// [`place`], its solve carried from `from` when given: the instances
/// start where `from` left them and the drives move from `from`'s values
/// to the document's along the motion's branch (`Solver::advance`).
/// Without `bodies` the bodies are not placed (a grid point of a motion
/// needs the solve alone): the placed part is empty. With `ready` the
/// configurations of parts that are not kept are not built: each instance
/// that asks for one is shown with the configuration `ready` gives for it
/// by its name when that is kept (else it is left out, its configuration
/// being built), and is listed in [`Placed::stale`]
/// (`view.motion {shapes: "ready"}`). With `hold` a joint is held by
/// hand for the view ([`Hold`]).
#[allow(clippy::too_many_arguments)]
fn place_from(
    doc: &Document,
    base: &str,
    state: &mut AsmState,
    chain: &mut Vec<String>,
    from: Option<&Carried>,
    bodies: bool,
    ready: Option<&BTreeMap<String, String>>,
    hold: Option<&Hold>,
) -> Placed {
    let Some(assembly) = doc.assembly.as_ref() else {
        return empty(doc, Some("the document is not an assembly".to_string()));
    };
    let values = evaluation(doc);
    if chain.is_empty() && bodies {
        state.placed_copies.begin();
    }
    // 1. Sources and configurations.
    let mut wanted: BTreeSet<String> = BTreeSet::new();
    let (mut instances, mut jobs) = gather(doc, assembly, base, state, chain, &mut wanted);
    let mut stale = Vec::new();
    if let Some(ready) = ready {
        let mut later: BTreeSet<String> = jobs
            .iter()
            .filter(|job| job.doc.as_ref().is_ok_and(|d| !d.is_assembly()))
            .map(|job| job.key.clone())
            .collect();
        jobs.retain(|job| !later.contains(&job.key));
        // And those being built elsewhere already (`AsmState::deferred`).
        later.extend(
            instances
                .iter()
                .filter_map(|st| st.config.clone())
                .filter(|key| state.deferred.contains(key) && !state.configs.contains_key(key)),
        );
        for st in &mut instances {
            if !st.config.as_ref().is_some_and(|key| later.contains(key)) {
                continue;
            }
            match ready
                .get(&st.name)
                .filter(|key| state.configs.contains_key(*key))
            {
                Some(key) => st.config = Some(key.clone()),
                None => st.error = Some("its configuration is being built".to_string()),
            }
            stale.push(st.name.clone());
        }
    }
    // 2. Build what is not cached.
    build(state, chain, jobs);
    state.tick = state.tick.saturating_add(1);
    let tick = state.tick;
    for (st, instance) in instances.iter_mut().zip(&assembly.instances) {
        if st.error.is_some() {
            continue;
        }
        let Some(key) = &st.config else { continue };
        match state.configs.get_mut(key) {
            Some(Config {
                built: Ok(built),
                used,
                shown,
                solved,
                ..
            }) => {
                *used = tick;
                *shown |= bodies;
                *solved |= !bodies;
                st.built = Some(Arc::clone(built));
                // A subassembly whose own sources changed has changed too.
                if let Built::Assembly(sub) = &**built
                    && sub.instances.iter().any(|i| i.changed)
                {
                    st.changed = true;
                }
                if let Some(body) = &instance.body {
                    match pick(built, body) {
                        Ok(id) => st.picked = Some(id),
                        Err(why) => st.error = Some(why),
                    }
                }
            }
            Some(Config {
                built: Err(why),
                used,
                shown,
                ..
            }) => {
                *used = tick;
                *shown |= bodies;
                st.error = Some(format!("its configuration failed: {why}"));
            }
            None => st.error = Some("its configuration was not built".to_string()),
        }
    }
    // Within the budget, keeping what this placement uses; only at the
    // top, so a subassembly placed while its parent builds does not let go
    // of what the parent has just built and not yet taken.
    if chain.is_empty() {
        state.evict();
    }
    // A flexible subassembly gives its instances and mates to the solve.
    let extra = flex(doc, assembly, &mut instances);
    // 3. and 4. Frames and the solve.
    let (mates, couplings, solve_error, floating, dof, specs, carried, solve_note) = solve_mates(
        doc,
        assembly,
        values.as_ref(),
        &mut instances,
        &extra,
        from,
        hold,
    );
    // 5. Placed bodies.
    let mut placed = if bodies {
        let placed = build_shown(doc, assembly, &mut instances, &mut state.placed_copies);
        if chain.is_empty() {
            state.placed_copies.end();
        }
        placed
    } else {
        let mut placed = empty(doc, None);
        placed.instances = instances;
        placed
    };
    (placed.specs, placed.coupling_specs) = specs;
    placed.mates = mates;
    placed.couplings = couplings;
    placed.solve_error = solve_error;
    placed.floating = floating;
    placed.dof = dof;
    placed.carried = carried;
    placed.solve_note = solve_note;
    placed.stale = stale;
    placed
}

/// Step 1 of a placement: each instance as far as its source and its
/// configuration's key, and the configurations to build that are neither
/// kept nor in `wanted` already (each added to it).
fn gather(
    doc: &Document,
    assembly: &Assembly,
    base: &str,
    state: &mut AsmState,
    chain: &[String],
    wanted: &mut BTreeSet<String>,
) -> (Vec<InstanceState>, Vec<Job>) {
    let values = evaluation(doc);
    let files = state.files.clone();
    let mut instances: Vec<InstanceState> = Vec::with_capacity(assembly.instances.len());
    let mut jobs: Vec<Job> = Vec::new();
    let mut slim_keys: BTreeMap<String, bool> = BTreeMap::new();
    for instance in &assembly.instances {
        let mut st = InstanceState {
            name: instance.name.clone(),
            source: match &instance.source {
                Source::File { path, .. } => path.clone(),
                Source::Packed { .. } => "packed".to_string(),
            },
            error: None,
            config: None,
            hash: None,
            changed: false,
            held: None,
            pose: placement_pose(&instance.placement),
            bodies: Vec::new(),
            built: None,
            picked: None,
            flexed: false,
            flex_of: None,
        };
        let prepared = prepare(
            assembly,
            instance,
            base,
            &files,
            chain,
            state,
            doc,
            values.as_ref(),
        )
        .map(|(hash, source, params, changed, held)| {
            st.changed = changed;
            st.held = held;
            (hash, source, params)
        });
        match prepared {
            Ok((hash, source, params)) => {
                let sub_base = match &instance.source {
                    Source::File { path, .. } => {
                        join(base, path).map(|p| dir_of(&p)).unwrap_or_default()
                    }
                    Source::Packed { .. } => base.to_string(),
                };
                let mut key = config_key(&hash, &params);
                // Built in the assembly's length unit.
                key.push('#');
                key.push_str(doc.units.length.symbol());
                if source.is_assembly() {
                    // A subassembly's placement also depends on the folder
                    // its own sources are read from: two copies of one file
                    // in two folders are two configurations.
                    key.push('@');
                    key.push_str(&sub_base);
                }
                st.hash = Some(hash.clone());
                st.config = Some(key.clone());
                // Slim only in a motion's own state, at the top (a parent's
                // mate may reach into a subassembly's instances), for an
                // instance mated by its datums alone.
                let slim = state.slim
                    && chain.is_empty()
                    && !source.is_assembly()
                    && mated_by_datums(assembly, &instance.name, &source);
                let all_slim = slim_keys.entry(key.clone()).or_insert(true);
                *all_slim &= slim;
                let kept = state.configs.get(&key).is_some_and(|c| slim || !c.slim);
                // Being built elsewhere: not worked out again here.
                let deferred = !state.configs.contains_key(&key) && state.deferred.contains(&key);
                if !kept && !deferred && wanted.insert(key.clone()) {
                    let source_key = (!params.is_empty() && !source.is_assembly())
                        .then(|| format!("{hash}#{}", doc.units.length.symbol()));
                    jobs.push(Job {
                        key,
                        doc: configured(&source, &params, doc.units.length),
                        base: sub_base,
                        source: source_key,
                        slim,
                    });
                }
            }
            Err(why) => st.error = Some(why),
        }
        instances.push(st);
    }
    // Slim only when every instance that shares it may have it slim.
    for job in &mut jobs {
        job.slim &= slim_keys.get(&job.key).copied().unwrap_or(false);
    }
    (instances, jobs)
}

/// Step 2 of a placement: builds `jobs` and keeps them, the parts side by
/// side, subassemblies one after the other (each builds its own parts
/// side by side). A part given parameters is regenerated from the
/// regeneration cache of the last configuration built of its source, so
/// only the features its changed parameters reach are rebuilt; the result
/// is that of a regeneration from nothing, body for body
/// (`craft3d_regen::regenerate_cached_needing` with `Need::All`).
fn build(state: &mut AsmState, chain: &mut Vec<String>, jobs: Vec<Job>) {
    state.tick = state.tick.saturating_add(1);
    let tick = state.tick;
    let (subs, parts): (Vec<Job>, Vec<Job>) = jobs
        .into_iter()
        .partition(|job| job.doc.as_ref().is_ok_and(Document::is_assembly));
    let indices: Vec<usize> = (0..parts.len()).collect();
    let caches = &state.regen_caches;
    let cancel = state.cancel.clone();
    let built = crate::interference_cmds::in_order(&indices, |i| {
        let job = parts.get(i)?;
        Some(match &job.doc {
            Ok(doc) => {
                let mut cache = job
                    .source
                    .as_ref()
                    .and_then(|source| caches.get(source))
                    .cloned()
                    .unwrap_or_default();
                let mut regen = match &cancel {
                    None => craft3d_regen::regenerate_cached_needing(
                        doc,
                        &mut cache,
                        craft3d_regen::Need::All,
                    ),
                    Some(cancel) => {
                        let cancel = std::sync::Arc::clone(cancel);
                        let (regen, stopped) = craft3d_regen::progress::with_stop(
                            move || cancel(),
                            || {
                                craft3d_regen::regenerate_cached_needing(
                                    doc,
                                    &mut cache,
                                    craft3d_regen::Need::All,
                                )
                            },
                        );
                        if stopped {
                            // Cut short: neither the configuration nor its
                            // cache is kept.
                            return Some(None);
                        }
                        regen
                    }
                };
                if job.slim {
                    regen = slimmed(regen);
                }
                let built = Ok(Arc::new(Built::Part {
                    doc: Arc::new(doc.clone()),
                    regen: Arc::new(regen),
                }));
                Some((built, job.source.is_some().then_some(cache)))
            }
            Err(why) => Some((Err(why.clone()), None)),
        })
    });
    for (job, result) in parts.iter().zip(built) {
        if job.doc.is_ok() {
            state.builds = state.builds.saturating_add(1);
        }
        let (result, cache) = match result {
            Some(Some(done)) => done,
            // Let go of part way (`AsmState::cancel`): not kept.
            Some(None) => continue,
            None => (Err("its regeneration stopped".to_string()), None),
        };
        // The cache of the last configuration built of each source, in the
        // order of the jobs (which cache is kept changes only the time).
        if let (Some(source), Some(cache)) = (&job.source, cache) {
            state.regen_caches.insert(source.clone(), cache);
        }
        state.keep(job.key.clone(), result, tick, job.slim);
    }
    for job in subs {
        let result = match &job.doc {
            Ok(sub) => {
                let hash = job
                    .key
                    .split(['|', '@'])
                    .next()
                    .unwrap_or_default()
                    .to_string();
                chain.push(hash);
                let placed = place(sub, &job.base, state, chain);
                chain.pop();
                state.builds = state.builds.saturating_add(1);
                Ok(Arc::new(Built::Assembly(Arc::new(placed))))
            }
            Err(why) => Err(why.clone()),
        };
        state.keep(job.key, result, tick, false);
    }
}

/// What the flexible subassemblies give their parent's solve: their
/// mates, by the names of their instances there, and their couplings
/// (mate indices into `specs`).
#[derive(Default)]
pub(crate) struct Flexed {
    pub specs: Vec<NamedSpec>,
    pub couplings: Vec<solver::CouplingSpec>,
}

/// Each flexible subassembly's instances added to `instances` as
/// `Sub/Inner`, starting where the subassembly puts them, and its mates
/// (by those names) and couplings, its world being the subassembly's
/// placement. The subassembly itself is then not solved.
///
/// **Its drives.** A free motion the subassembly drives itself (its
/// `<mate>_angle` variable, a formula of its `t`) is free in the parent
/// unless the parent drives it: the instance sets one of the
/// subassembly's parameters that the drive depends on (`params`, the
/// drive's variable published, or the `t` it follows). So a leg that
/// plays its own motion is a part of a machine that drives it otherwise;
/// a drive kept depends, in the parent, on what the parent's values for
/// those parameters depend on, for the parent's own parent.
fn flex(doc: &Document, assembly: &Assembly, instances: &mut Vec<InstanceState>) -> Flexed {
    let mut flexed = Flexed::default();
    let mut added = Vec::new();
    for (st, instance) in instances.iter_mut().zip(&assembly.instances) {
        if st.error.is_some() || instance.mode != craft3d_doc::InstanceMode::Flexible {
            continue;
        }
        let Some(Built::Assembly(sub)) = st.built.as_deref() else {
            continue;
        };
        st.flexed = true;
        let anchor = st.pose;
        let prefixed = |inner: &str| format!("{}/{inner}", st.name);
        for inner in sub
            .instances
            .iter()
            .filter(|i| i.error.is_none() && !i.flexed)
        {
            let mut child = inner.clone();
            child.name = prefixed(&inner.name);
            child.pose = inner.pose.then(&anchor);
            child.bodies = Vec::new();
            child.flex_of = Some(st.name.clone());
            added.push(child);
        }
        let offset = flexed.specs.len();
        for named in &sub.specs {
            let mut spec = named.spec.clone();
            // The subassembly's world is where it is placed.
            if named.a.is_none() {
                spec.a.1 = spec.a.1.moved(&anchor);
            }
            if named.b.is_none() {
                spec.b.1 = spec.b.1.moved(&anchor);
            }
            let mut drive_deps = named.drive_deps.clone();
            for (slot, deps) in spec.driven.iter_mut().zip(drive_deps.iter_mut()) {
                if slot.is_none() {
                    continue;
                }
                let by: Vec<&craft3d_doc::ParamOverride> = instance
                    .params
                    .iter()
                    .filter(|p| deps.contains(&p.name))
                    .collect();
                if by.is_empty() {
                    *slot = None;
                    deps.clear();
                } else {
                    *deps = depends_on(
                        doc,
                        by.iter().flat_map(|p| {
                            craft3d_expr::names_in(p.expr.source())
                                .into_iter()
                                .map(str::to_string)
                        }),
                    );
                }
            }
            flexed.specs.push(NamedSpec {
                a: named.a.as_deref().map(prefixed),
                b: named.b.as_deref().map(prefixed),
                spec,
                drive_deps,
                own: false,
            });
        }
        for c in &sub.coupling_specs {
            let kind = match &c.kind {
                solver::CouplingKind::Linear(terms) => solver::CouplingKind::Linear(
                    terms
                        .iter()
                        .map(|&(m, v, k)| (m.saturating_add(offset), v, k))
                        .collect(),
                ),
                other => other.clone(),
            };
            flexed.couplings.push(solver::CouplingSpec {
                kind,
                mates: (
                    c.mates.0.saturating_add(offset),
                    c.mates.1.saturating_add(offset),
                ),
                value: c.value,
            });
        }
    }
    instances.extend(added);
    flexed
}

/// Per driven free motion of `mate` (as `spec` drives it): the variables
/// of `doc` its value depends on, its own variable among them.
fn drive_deps(
    doc: &Document,
    mate: &craft3d_doc::Mate,
    spec: &solver::MateSpec,
) -> [BTreeSet<String>; solver::VARIABLES] {
    let mut out: [BTreeSet<String>; solver::VARIABLES] = Default::default();
    for ((slot, which), deps) in spec
        .driven
        .iter()
        .zip(mate.kind.free().iter().copied())
        .zip(out.iter_mut())
    {
        if slot.is_some() {
            *deps = depends_on(doc, [joint_variable(&mate.name, which)]);
        }
    }
    out
}

/// The variables of `doc` named by `start` and every one they depend on,
/// through the variables' formulas (a cycle or a long chain is cut at
/// [`MAX_DEPENDS`] names).
fn depends_on(doc: &Document, start: impl IntoIterator<Item = String>) -> BTreeSet<String> {
    let mut seen = BTreeSet::new();
    let mut stack: Vec<String> = start.into_iter().take(MAX_DEPENDS).collect();
    while let Some(name) = stack.pop() {
        if seen.len() >= MAX_DEPENDS {
            break;
        }
        if !seen.insert(name.clone()) {
            continue;
        }
        if let Some(v) = doc.variables.iter().find(|v| v.name == name)
            && stack.len() < MAX_DEPENDS
        {
            stack.extend(
                craft3d_expr::names_in(v.expr.source())
                    .into_iter()
                    .map(str::to_string),
            );
        }
    }
    seen
}

/// The placement of a document that has nothing to place.
fn empty(doc: &Document, error: Option<String>) -> Placed {
    let mut shown = doc.clone();
    shown.assembly = None;
    shown.features.clear();
    Placed {
        doc: Arc::new(shown),
        regen: Arc::new(Regen::default()),
        instances: Vec::new(),
        mates: Vec::new(),
        couplings: Vec::new(),
        solve_error: error,
        floating: false,
        dof: None,
        specs: Vec::new(),
        coupling_specs: Vec::new(),
        carried: None,
        solve_note: None,
        stale: Vec::new(),
    }
}

/// The feature of a multi-body source whose body an instance shows: by
/// the feature's name or id. Never another one: a body that is gone fails
/// the instance by its name.
fn pick(built: &Built, body: &str) -> Result<FeatureId, String> {
    let Built::Part { doc, regen } = built else {
        return Err(format!(
            "it picks the body `{body}`, and its source is an assembly (pick the instance's bodies in the subassembly)"
        ));
    };
    let id = crate::selection::feature_named(doc, body)
        .ok_or_else(|| format!("its source has no feature `{body}`: the body it picks is gone"))?;
    match regen.feature(id).and_then(FeatureResult::own_body) {
        Some(_) => Ok(id),
        None => Err(format!(
            "the body `{body}` it picks is not one of its source's bodies now (it went into a later feature, or failed)"
        )),
    }
}

/// The bodies an instance shows, in its own coordinates, each with the
/// names of its entities and what to call it.
pub(crate) fn bodies_of(st: &InstanceState) -> Vec<ShownBody> {
    let Some(built) = &st.built else {
        return Vec::new();
    };
    match built.as_ref() {
        Built::Part { doc, regen } => regen
            .features
            .iter()
            .filter(|f| st.picked.is_none_or(|p| p == f.id))
            .filter(|f| !doc.is_hidden(f.id) || st.picked == Some(f.id))
            .filter_map(|f| {
                let body = f.own_body()?;
                let name = doc
                    .feature(f.id)
                    .map_or_else(|| f.id.to_string(), |feature| feature.name.clone());
                let props = doc.bodies.iter().find(|p| p.feature == f.id).cloned();
                Some((name, Arc::clone(body), f.names.clone(), props))
            })
            .collect(),
        Built::Assembly(placed) => placed
            .regen
            .features
            .iter()
            .filter(|f| !placed.doc.is_hidden(f.id))
            .filter_map(|f| {
                let body = f.own_body()?;
                let name = placed
                    .doc
                    .feature(f.id)
                    .map_or_else(|| f.id.to_string(), |feature| feature.name.clone());
                let props = placed
                    .doc
                    .bodies
                    .iter()
                    .find(|p| p.feature == f.id)
                    .cloned();
                Some((name, Arc::clone(body), f.names.clone(), props))
            })
            .collect(),
    }
}

/// The part that shows the placed bodies.
fn build_shown(
    doc: &Document,
    assembly: &Assembly,
    instances: &mut [InstanceState],
    copies: &mut PlacedCopies,
) -> Placed {
    let mut shown = doc.clone();
    shown.assembly = None;
    shown.features.clear();
    shown.bodies.clear();
    shown.faces.clear();
    shown.hidden.clear();
    shown.rollback = None;
    shown.inserted.clear();
    let mut results = Vec::new();
    let mut next = 1u64;
    for st in instances.iter_mut() {
        // A flexible subassembly is drawn through its instances.
        if st.error.is_some() || st.flexed {
            continue;
        }
        let Some(instance) = assembly.instance(st.flex_of.as_deref().unwrap_or(&st.name)) else {
            continue;
        };
        let bodies = bodies_of(st);
        let several = bodies.len() > 1;
        let file_name = match &instance.source {
            Source::File { path, .. } => path.clone(),
            Source::Packed { hash } => format!("packed {hash}"),
        };
        for (name, body, names, props) in bodies {
            let placed = match copies.place(&body, &st.pose) {
                Ok(placed) => placed,
                Err(e) => {
                    st.error = Some(format!("its body `{name}` cannot be placed: {e}"));
                    break;
                }
            };
            let id = FeatureId(next);
            next = next.saturating_add(1);
            shown.features.push(Feature {
                id,
                name: if several {
                    format!("{}/{name}", st.name)
                } else {
                    st.name.clone()
                },
                suppressed: false,
                when: None,
                kind: FeatureKind::Import(Import {
                    file_name: file_name.clone(),
                    solid: 0,
                    data: Blob::new(Vec::new()),
                }),
                made_by: None,
                last_changed_by: None,
            });
            if !instance.visible {
                shown.hidden.push(id);
            }
            // The source's appearance and material where they are fixed
            // values (an expression would read the source's variables),
            // the instance's colour over it.
            let mut appearance = props
                .as_ref()
                .and_then(|p| p.appearance.clone())
                .filter(|a| matches!(a.color, craft3d_doc::BodyColor::Fixed(_)));
            if let Some(colour) = instance.colour {
                appearance = Some(craft3d_doc::BodyAppearance {
                    color: craft3d_doc::BodyColor::Fixed(colour),
                    metallic: appearance.as_ref().and_then(|a| a.metallic),
                    roughness: appearance.as_ref().and_then(|a| a.roughness),
                    optics: appearance.as_ref().map(|a| a.optics).unwrap_or_default(),
                });
            }
            let material = props
                .and_then(|p| p.material)
                .filter(|m| m.density_expr.is_none());
            if appearance.is_some() || material.is_some() {
                shown.bodies.push(BodyProperties {
                    feature: id,
                    appearance,
                    material,
                });
            }
            st.bodies.push(id);
            results.push(FeatureResult {
                id,
                status: Status::Ok,
                body: Some(placed),
                sketch: None,
                datum: None,
                names,
                instances: None,
                sweep: None,
                thread: None,
                developed: None,
                offset: None,
                consumed_by: None,
                followed: None,
                passed: None,
            });
        }
    }
    shown.next_feature_id = next;
    // A flexible subassembly holds its instances' bodies.
    let children: Vec<(String, Vec<FeatureId>)> = instances
        .iter()
        .filter_map(|i| Some((i.flex_of.clone()?, i.bodies.clone())))
        .collect();
    for (parent, ids) in children {
        if let Some(st) = instances.iter_mut().find(|i| i.name == parent) {
            st.bodies.extend(ids);
        }
    }
    Placed {
        doc: Arc::new(shown),
        regen: Arc::new(Regen {
            // The assembly's own variables, evaluated as a part's are.
            variables: craft3d_regen::regenerate(doc).variables,
            features: results,
        }),
        instances: instances.to_vec(),
        mates: Vec::new(),
        couplings: Vec::new(),
        solve_error: None,
        floating: false,
        dof: None,
        specs: Vec::new(),
        coupling_specs: Vec::new(),
        carried: None,
        solve_note: None,
        stale: Vec::new(),
    }
}

/// A frame of the world: `xy`, `yz`, `zx` (planes), `x`, `y`, `z`
/// (axes), `origin`.
fn world_frame(name: &str) -> Result<solver::Frame, String> {
    let (z, x) = match name {
        "xy" | "z" | "origin" => (Vec3::Z, Vec3::X),
        "yz" | "x" => (Vec3::X, Vec3::Y),
        "zx" | "y" => (Vec3::Y, Vec3::Z),
        other => {
            return Err(format!(
                "the world has no `{}`: write world/xy, world/yz, world/zx, world/x, world/y, world/z or world/origin",
                other.chars().take(60).collect::<String>()
            ));
        }
    };
    Ok(solver::Frame {
        origin: Vec3::ZERO,
        x,
        y: z.cross(x),
        z,
    })
}

fn frame_of(f: &craft3d_math::Frame) -> solver::Frame {
    solver::Frame {
        origin: f.origin(),
        x: f.x(),
        y: f.y(),
        z: f.z(),
    }
}

/// A mate's limits on its main free motion, evaluated (radians, model
/// units); `None` without limits or when they do not evaluate.
pub(crate) fn mate_limits(doc: &Document, mate: &craft3d_doc::Mate) -> Option<(f64, f64)> {
    let (lo, hi) = mate.limits.as_ref()?;
    let values = evaluation(doc);
    let convert = |q: Quantity| {
        if q.dimension == Dimension::LENGTH {
            in_units(doc, q.value)
        } else {
            q.value
        }
    };
    let lo = convert(eval(values.as_ref(), lo.source()).ok()?);
    let hi = convert(eval(values.as_ref(), hi.source()).ok()?);
    (lo.is_finite() && hi.is_finite() && lo < hi).then_some((lo, hi))
}

/// The frame `name` (as a mate names it) means in an assembly placed, in
/// the world.
pub(crate) fn frame_of_ref(placed: &Arc<Placed>, name: &str) -> Result<solver::Frame, String> {
    if let Some(world) = name.strip_prefix("world/") {
        return world_frame(world);
    }
    frame_in(&Built::Assembly(Arc::clone(placed)), None, name)
}

/// Where the point `name` (`Instance/Feature:name` or `Instance/Datum`,
/// as a mate names it) is in an assembly placed: the origin of its frame.
pub(crate) fn point_in(placed: &Arc<Placed>, name: &str) -> Result<Vec3, String> {
    Ok(frame_in(&Built::Assembly(Arc::clone(placed)), None, name)?.origin)
}

/// The frame `name` means in a configuration, in its own coordinates:
/// a datum of the source, a face, an edge or a vertex of its bodies (of
/// the picked one when one is picked), or through a subassembly's
/// instance (`Inner/Feature:name`).
pub(crate) fn frame_in(
    built: &Built,
    picked: Option<FeatureId>,
    name: &str,
) -> Result<solver::Frame, String> {
    match built {
        Built::Assembly(placed) => {
            let (inner, rest) = name.split_once('/').ok_or_else(|| {
                format!("`{name}` names no instance of the subassembly: write `Inner/Feature:name`")
            })?;
            let mut st = placed
                .instance(inner)
                .ok_or_else(|| format!("the subassembly has no instance `{inner}`"))?;
            let mut rest = rest;
            // Through a flexible subassembly of it: its instance.
            if st.flexed {
                let (deeper, after) = rest.split_once('/').ok_or_else(|| {
                    format!(
                        "`{inner}` is a flexible subassembly: write `{inner}/Inner/Feature:name`"
                    )
                })?;
                let child = format!("{inner}/{deeper}");
                st = placed
                    .instance(&child)
                    .ok_or_else(|| format!("the subassembly has no instance `{child}`"))?;
                rest = after;
            }
            if let Some(why) = &st.error {
                return Err(format!(
                    "the subassembly's instance `{inner}` is not placed: {why}"
                ));
            }
            let built = st
                .built
                .as_ref()
                .ok_or_else(|| format!("the subassembly's instance `{inner}` is not built"))?;
            Ok(frame_in(built, st.picked, rest)?.moved(&st.pose))
        }
        Built::Part { doc, regen } => {
            // A datum of the source, by its name or id.
            if let Some(id) = crate::selection::feature_named(doc, name)
                && let Some(feature) = doc.feature(id)
                && crate::selection::is_datum(&feature.kind)
            {
                let datum = regen.feature(id).and_then(|f| f.datum).ok_or_else(|| {
                    match regen.feature(id).map(|f| &f.status) {
                        Some(Status::Failed(why)) => format!("the datum `{name}` failed: {why}"),
                        _ => format!("the datum `{name}` is not placed"),
                    }
                })?;
                return datum_frame(&datum, name);
            }
            let only = picked.map(|id| id.to_string());
            let mut resolver = Resolver::new(doc, regen);
            let (feature, kind, index) = resolver.locate(name, only.as_deref())?;
            // The instance shows only the source's visible bodies (or the
            // one picked): a mate on a hidden one would hold to nothing
            // drawn.
            if picked != Some(feature) && doc.is_hidden(feature) {
                let shown = doc
                    .feature(feature)
                    .map_or_else(|| feature.to_string(), |f| f.name.clone());
                return Err(format!(
                    "`{name}` is on the body of `{shown}`, which the source hides; an instance shows only the visible bodies (body: \"{shown}\" picks it)"
                ));
            }
            let body = regen
                .feature(feature)
                .and_then(FeatureResult::own_body)
                .ok_or_else(|| format!("`{name}` has no body"))?;
            entity_frame(body, kind, index, doc.tolerance.linear, name)
        }
    }
}

fn datum_frame(datum: &craft3d_regen::Datum, name: &str) -> Result<solver::Frame, String> {
    use craft3d_regen::Datum;
    match datum {
        Datum::Plane(frame) => Ok(frame_of(frame)),
        Datum::Axis { origin, direction } => craft3d_math::Frame::from_z(*origin, *direction)
            .map(|f| frame_of(&f))
            .ok_or_else(|| format!("the datum axis `{name}` has no direction")),
        Datum::Point(p) => Ok(solver::Frame {
            origin: *p,
            x: Vec3::X,
            y: Vec3::Y,
            z: Vec3::Z,
        }),
        Datum::Helix(_) => Err(format!(
            "`{name}` is a helix; a mate needs a plane, an axis or a point"
        )),
    }
}

/// The area-weighted centroid of face `index` of `body`, from a mesh of
/// it at a chordal tolerance a thousandth of the body's size.
fn face_centroid(body: &Body, index: usize, tolerance: f64) -> Option<Vec3> {
    let size = body
        .held_bbox()
        .ok()
        .and_then(|b| b.corners())
        .map_or(1.0, |(lo, hi)| lo.distance(hi));
    let params = craft3d_tess::TessParams {
        chordal_tolerance: (size * 1e-3).max(tolerance * 10.0),
        ..craft3d_tess::TessParams::default()
    };
    let mesh = craft3d_tess::tessellate_body(body, &params).ok()?;
    let wanted = u32::try_from(index).ok()?;
    let mut area = 0.0;
    let mut sum = Vec3::ZERO;
    for (t, face) in mesh.face_ids.iter().enumerate() {
        if *face != wanted {
            continue;
        }
        let [a, b, c] = mesh.corners(t)?;
        let w = (b - a).cross(c - a).length() * 0.5;
        area += w;
        sum += (a + b + c) * (w / 3.0);
    }
    (area > 0.0 && area.is_finite()).then(|| sum * (1.0 / area))
}

/// A mate frame from a face, an edge or a vertex (the plan's table): a
/// plane at the face's centroid with its outward normal; the axis of a
/// cylinder or a cone at the point nearest the face's centroid; the
/// centre and axis of a circle; a point.
fn entity_frame(
    body: &Body,
    kind: SelectionKind,
    index: usize,
    tolerance: f64,
    name: &str,
) -> Result<solver::Frame, String> {
    use craft3d_geom::{Curve3, Surface};
    match kind {
        SelectionKind::Face => {
            let face = body
                .faces
                .get(index)
                .ok_or_else(|| format!("`{name}` is not a face of the body"))?;
            let centroid = face_centroid(body, index, tolerance)
                .ok_or_else(|| format!("the face `{name}` has no area to take a centroid of"))?;
            match &face.surface {
                Surface::Plane(plane) => {
                    let f = plane.frame();
                    let z = if face.reversed { -f.z() } else { f.z() };
                    let origin = centroid - z * (centroid - f.origin()).dot(z);
                    craft3d_math::Frame::new(origin, z, f.x())
                        .map(|f| frame_of(&f))
                        .ok_or_else(|| format!("the face `{name}` has no frame"))
                }
                Surface::Cylinder(c) => axis_frame(c.frame(), centroid, name),
                Surface::Cone(c) => axis_frame(c.frame(), centroid, name),
                other => Err(format!(
                    "the face `{name}` is a {}; a mate's frame comes from a planar, cylindrical or conical face, a circular edge, a vertex or a datum",
                    other.kind()
                )),
            }
        }
        SelectionKind::Edge => {
            let edge = body
                .edges
                .get(index)
                .ok_or_else(|| format!("`{name}` is not an edge of the body"))?;
            match &edge.curve {
                Curve3::Circle(circle) => Ok(frame_of(circle.frame())),
                other => Err(format!(
                    "the edge `{name}` is a {}; a mate's frame comes from a circular edge",
                    other.kind()
                )),
            }
        }
        SelectionKind::Vertex => {
            let vertex = body
                .vertices
                .get(index)
                .ok_or_else(|| format!("`{name}` is not a vertex of the body"))?;
            Ok(solver::Frame {
                origin: vertex.point,
                x: Vec3::X,
                y: Vec3::Y,
                z: Vec3::Z,
            })
        }
        _ => Err(format!("`{name}` names no face, edge or vertex")),
    }
}

fn axis_frame(
    f: &craft3d_math::Frame,
    centroid: Vec3,
    name: &str,
) -> Result<solver::Frame, String> {
    let z = f.z();
    let origin = f.origin() + z * (centroid - f.origin()).dot(z);
    craft3d_math::Frame::new(origin, z, f.x())
        .map(|f| frame_of(&f))
        .ok_or_else(|| format!("the face `{name}` has no axis"))
}

/// The variable that drives a mate's free motion `which` (`angle`,
/// `distance`, `x`, `y`): `<mate>_<which>`.
pub(crate) fn joint_variable(mate: &str, which: &str) -> String {
    format!("{mate}_{which}")
}

fn solver_kind(kind: MateKind) -> solver::MateKind {
    match kind {
        MateKind::Fixed => solver::MateKind::Fixed,
        MateKind::Revolute => solver::MateKind::Revolute,
        MateKind::Slider => solver::MateKind::Slider,
        MateKind::Cylindrical => solver::MateKind::Cylindrical,
        MateKind::Planar => solver::MateKind::Planar,
        MateKind::Ball => solver::MateKind::Ball,
    }
}

/// One side of a mate: the instance's index among the solved ones (or
/// the world) and its frame.
fn side(
    r: &MateRef,
    instances: &[InstanceState],
    index: &BTreeMap<String, usize>,
) -> Result<(usize, solver::Frame), String> {
    if r.instance == WORLD {
        return Ok((solver::WORLD, world_frame(&r.name)?));
    }
    let mut st = instances
        .iter()
        .find(|i| i.name == r.instance)
        .ok_or_else(|| format!("there is no instance `{}`", r.instance))?;
    let mut name = r.name.as_str();
    // Through a flexible subassembly: the instance of it that is solved.
    if st.flexed {
        let (inner, rest) = name.split_once('/').ok_or_else(|| {
            format!(
                "`{}` is a flexible subassembly: write `{}/Inner/Feature:name`",
                r.instance, r.instance
            )
        })?;
        let child = format!("{}/{inner}", r.instance);
        st = instances
            .iter()
            .find(|i| i.name == child)
            .ok_or_else(|| format!("the subassembly `{}` has no instance `{inner}`", r.instance))?;
        name = rest;
    }
    if let Some(why) = &st.error {
        return Err(format!("its instance `{}` is not placed: {why}", st.name));
    }
    let built = st
        .built
        .as_ref()
        .ok_or_else(|| format!("its instance `{}` is not built", st.name))?;
    let frame = frame_in(built, st.picked, name).map_err(|e| format!("`{r}`: {e}"))?;
    let at = index
        .get(&st.name)
        .copied()
        .ok_or_else(|| format!("its instance `{}` is not placed", st.name))?;
    Ok((at, frame))
}

/// A mate as the solver takes it: its frames, offset, angle, driven
/// value and limits, in the assembly's units.
fn mate_spec(
    doc: &Document,
    mate: &craft3d_doc::Mate,
    instances: &[InstanceState],
    index: &BTreeMap<String, usize>,
    values: Option<&craft3d_expr::Evaluation>,
) -> Result<solver::MateSpec, String> {
    let length = |q: Quantity| in_units(doc, q.value);
    let a = side(&mate.a, instances, index)?;
    let b = side(&mate.b, instances, index)?;
    let number = |e: &craft3d_doc::Expr, what: &str| -> Result<Quantity, String> {
        eval(values, e.source()).map_err(|err| format!("its {what} {}: {err}", e.source()))
    };
    let offset = match &mate.offset {
        Some(e) => length(number(e, "offset")?),
        None => 0.0,
    };
    let angle = match &mate.angle {
        Some(e) => number(e, "angle")?.value,
        None => 0.0,
    };
    // Each free motion is driven when its variable exists.
    let mut driven = [None; solver::VARIABLES];
    for (slot, which) in driven.iter_mut().zip(mate.kind.free().iter().copied()) {
        let var = joint_variable(&mate.name, which);
        if let Some(v) = doc.variables.iter().find(|v| v.name == var) {
            let q =
                eval(values, v.expr.source()).map_err(|e| format!("its variable `{var}`: {e}"))?;
            *slot = Some(if which == "angle" { q.value } else { length(q) });
        }
    }
    // The limits hold the main free motion.
    let mut limits = [None; solver::VARIABLES];
    if let Some((lo, hi)) = &mate.limits {
        let lo = number(lo, "lower limit")?;
        let hi = number(hi, "upper limit")?;
        let convert = |q: Quantity| {
            if q.dimension == Dimension::LENGTH {
                length(q)
            } else {
                q.value
            }
        };
        if let Some(slot) = limits.first_mut() {
            *slot = Some((convert(lo), convert(hi)));
        }
    }
    Ok(solver::MateSpec {
        name: mate.name.clone(),
        kind: solver_kind(mate.kind),
        a,
        b,
        offset,
        angle,
        // A mate puts b's frame against a's, turned over (face to face),
        // unless `flip` keeps it as it is.
        flip: !mate.flip,
        limits,
        driven,
    })
}

/// A linear coupling's terms as the solver takes them: `(spec, variable,
/// coefficient)`.
type LinearTerms = Vec<(usize, usize, f64)>;

/// A linear coupling's terms for the solver, `(spec, variable,
/// coefficient)`, and its right-hand side: `Σ cᵢ · qᵢ = value` in base
/// units (millimetres, radians), each length motion's coefficient taken
/// to the assembly's length unit. Every term's product has the dimension
/// of the value (length, mass and time; an angle is a pure number here, as
/// radians are),
/// except that a value of zero fits any; the first term's sets it then.
fn linear_terms(
    doc: &Document,
    assembly: &Assembly,
    c: &craft3d_doc::Coupling,
    values: Option<&craft3d_expr::Evaluation>,
    at: &dyn Fn(&str) -> Result<usize, String>,
) -> Result<(LinearTerms, f64), String> {
    let value = eval(values, c.value.source())
        .map_err(|e| format!("its value {}: {e}", c.value.source()))?;
    if !value.value.is_finite() {
        return Err(format!("its value {} is not finite", c.value.source()));
    }
    // Millimetres per model unit: a length motion is solved in model units.
    let per_unit = {
        let f = doc.units.length.as_expr_unit().factor();
        if f.is_finite() && f > 0.0 { f } else { 1.0 }
    };
    // An angle counts as a pure number (radians); every other part of a
    // dimension (length, mass, time) must agree.
    let pure = |d: Dimension| Dimension { angle: 0, ..d };
    let mut power = (value.value != 0.0).then_some(pure(value.dimension));
    let mut terms = Vec::with_capacity(c.terms.len());
    for term in &c.terms {
        let mate = assembly
            .mates
            .iter()
            .find(|m| m.name == term.mate)
            .ok_or_else(|| format!("its mate `{}` is not in the assembly", term.mate))?;
        let variable = mate
            .kind
            .free()
            .iter()
            .position(|m| *m == term.motion)
            .ok_or_else(|| {
                format!(
                    "its mate `{}` ({}) has no free motion `{}`",
                    term.mate,
                    mate.kind.name(),
                    term.motion
                )
            })?;
        let spec = at(&term.mate)?;
        let coefficient = eval(values, term.coefficient.source()).map_err(|e| {
            format!(
                "the coefficient {} of `{}_{}`: {e}",
                term.coefficient.source(),
                term.mate,
                term.motion
            )
        })?;
        let length = term.motion != "angle";
        let mut product = pure(coefficient.dimension);
        product.length = product.length.saturating_add(i32::from(length));
        match power {
            None => power = Some(product),
            Some(p) if p != product => {
                return Err(format!(
                    "the term {} · {}_{} is not of the dimension of the others or the value {}",
                    term.coefficient.source(),
                    term.mate,
                    term.motion,
                    c.value.source()
                ));
            }
            Some(_) => {}
        }
        let k = if length {
            coefficient.value * per_unit
        } else {
            coefficient.value
        };
        if !k.is_finite() {
            return Err(format!(
                "the coefficient {} of `{}_{}` is not finite",
                term.coefficient.source(),
                term.mate,
                term.motion
            ));
        }
        terms.push((spec, variable, k));
    }
    Ok((terms, value.value))
}

type Solved = (
    Vec<MateState>,
    Vec<CouplingState>,
    Option<String>,
    bool,
    Option<(usize, Vec<String>)>,
    (Vec<NamedSpec>, Vec<solver::CouplingSpec>),
    Option<Carried>,
    // Why a moment was not carried from the one before it.
    Option<String>,
);

/// The mates' frames and the solve; the poses go into `instances`.
fn solve_mates(
    doc: &Document,
    assembly: &Assembly,
    values: Option<&craft3d_expr::Evaluation>,
    instances: &mut [InstanceState],
    extra: &Flexed,
    from: Option<&Carried>,
    hold: Option<&Hold>,
) -> Solved {
    let solvable = |i: &InstanceState| i.error.is_none() && !i.flexed;
    let index: BTreeMap<String, usize> = instances
        .iter()
        .filter(|i| solvable(i))
        .enumerate()
        .map(|(k, i)| (i.name.clone(), k))
        .collect();
    let solved: Vec<usize> = instances
        .iter()
        .enumerate()
        .filter(|(_, i)| solvable(i))
        .map(|(k, _)| k)
        .collect();
    let mut states = Vec::with_capacity(assembly.mates.len());
    let mut specs = Vec::new();
    // Per spec: the mate it stands for.
    let mut spec_mate = Vec::new();
    // Per spec: what each of its drives depends on.
    let mut spec_deps: Vec<[BTreeSet<String>; solver::VARIABLES]> = Vec::new();
    let length = |q: Quantity| in_units(doc, q.value);
    for (m, mate) in assembly.mates.iter().enumerate() {
        let mut st = MateState {
            name: mate.name.clone(),
            kind: mate.kind,
            error: None,
            free: None,
            values: Vec::new(),
            driven: false,
            main_free: false,
        };
        let spec = mate_spec(doc, mate, instances, &index, values);
        match spec {
            Ok(spec) => {
                st.driven = spec.driven.iter().any(Option::is_some);
                spec_deps.push(drive_deps(doc, mate, &spec));
                specs.push(spec);
                spec_mate.push(m);
            }
            Err(why) => st.error = Some(why),
        }
        states.push(st);
    }
    // A joint held by hand for the view: the drivers let go of, the held
    // motion driven at its value.
    if let Some(hold) = hold {
        for (spec, &m) in specs.iter_mut().zip(&spec_mate) {
            let Some(name) = assembly.mates.get(m).map(|mate| mate.name.as_str()) else {
                continue;
            };
            if hold.release.contains(name) {
                spec.driven = [None; solver::VARIABLES];
            }
            if let Some(held) = hold.held.as_ref().filter(|h| h.mate == name)
                && let Some(slot) = spec.driven.get_mut(held.slot)
            {
                *slot = Some(held.value);
            }
        }
    }
    let mut couplings = Vec::new();
    let mut coupling_states = Vec::with_capacity(assembly.couplings.len());
    for c in &assembly.couplings {
        // The mate's spec, or why the coupling cannot take it.
        let at = |name: &str| -> Result<usize, String> {
            let m = assembly
                .mates
                .iter()
                .position(|m| m.name == name)
                .ok_or_else(|| format!("its mate `{name}` is not in the assembly"))?;
            spec_mate.iter().position(|&s| s == m).ok_or_else(|| {
                match states.get(m).and_then(|s| s.error.as_deref()) {
                    Some(why) => format!("its mate `{name}` takes no part: {why}"),
                    None => format!("its mate `{name}` takes no part"),
                }
            })
        };
        let value = eval(values, c.value.source())
            .map_err(|e| format!("its value {}: {e}", c.value.source()))
            .and_then(|q| {
                let v = if q.dimension == Dimension::LENGTH {
                    length(q)
                } else {
                    q.value
                };
                if v.is_finite() {
                    Ok(v)
                } else {
                    Err(format!("its value {} is not finite", c.value.source()))
                }
            });
        let taken = match c.kind {
            craft3d_doc::CouplingKind::Linear => {
                linear_terms(doc, assembly, c, values, &at).map(|(terms, rhs)| {
                    let first = terms.first().map_or(0, |t| t.0);
                    (solver::CouplingKind::Linear(terms), (first, first), rhs)
                })
            }
            kind => at(&c.mates.0).and_then(|a| {
                let kind = match kind {
                    craft3d_doc::CouplingKind::Gear => solver::CouplingKind::Gear,
                    craft3d_doc::CouplingKind::Rack => solver::CouplingKind::Rack,
                    _ => solver::CouplingKind::Screw,
                };
                Ok((kind, (a, at(&c.mates.1)?), value?))
            }),
        };
        coupling_states.push(CouplingState {
            name: c.name.clone(),
            error: taken.as_ref().err().cloned(),
        });
        if let Ok((kind, mates, value)) = taken {
            couplings.push(solver::CouplingSpec { kind, mates, value });
        }
    }
    // The mates of its flexible subassemblies, by the names of their
    // instances here, and their couplings.
    let mut extra_at: Vec<Option<usize>> = Vec::with_capacity(extra.specs.len());
    for e in &extra.specs {
        let at = |name: &Option<String>| match name {
            None => Some(solver::WORLD),
            Some(n) => index.get(n).copied(),
        };
        if let (Some(a), Some(b)) = (at(&e.a), at(&e.b)) {
            let mut spec = e.spec.clone();
            spec.a.0 = a;
            spec.b.0 = b;
            extra_at.push(Some(specs.len()));
            specs.push(spec);
            spec_deps.push(e.drive_deps.clone());
        } else {
            extra_at.push(None);
        }
    }
    for c in &extra.couplings {
        let map = |i: usize| extra_at.get(i).copied().flatten();
        let kind = match &c.kind {
            solver::CouplingKind::Linear(terms) => terms
                .iter()
                .map(|&(m, v, k)| Some((map(m)?, v, k)))
                .collect::<Option<Vec<_>>>()
                .map(solver::CouplingKind::Linear),
            other => Some(other.clone()),
        };
        if let (Some(kind), Some(a), Some(b)) = (kind, map(c.mates.0), map(c.mates.1)) {
            couplings.push(solver::CouplingSpec {
                kind,
                mates: (a, b),
                value: c.value,
            });
        }
    }
    // Its mates (its flexible subassemblies' among them) by instance name
    // and its couplings, for a parent that holds it as a flexible
    // subassembly.
    let name_of = |k: usize| -> Option<String> {
        if k == solver::WORLD {
            None
        } else {
            solved
                .get(k)
                .and_then(|&i| instances.get(i))
                .map(|i| i.name.clone())
        }
    };
    let named: Vec<NamedSpec> = specs
        .iter()
        .zip(&spec_deps)
        .enumerate()
        .map(|(k, (spec, deps))| NamedSpec {
            a: name_of(spec.a.0),
            b: name_of(spec.b.0),
            spec: spec.clone(),
            drive_deps: deps.clone(),
            // Its own mates' specs come first, one per mate it took.
            own: k < spec_mate.len(),
        })
        .collect();
    let coupling_specs = couplings.clone();
    let touches_world = specs
        .iter()
        .any(|s| s.a.0 == solver::WORLD || s.b.0 == solver::WORLD);
    let floating = !touches_world && !solved.is_empty();
    // A moment of a motion starts where the one before it left the
    // instances (the same instances take part: else from the placements),
    // and stays there when it cannot be solved.
    let mut note = from.filter(|f| f.poses.len() != solved.len()).map(|f| {
        format!(
            "solved from the placements, not carried from the moment before: {} instances took part there, {} here",
            f.poses.len(),
            solved.len()
        )
    });
    let from = from.filter(|f| f.poses.len() == solved.len());
    if let Some(from) = from {
        for (&k, pose) in solved.iter().zip(&from.poses) {
            if let Some(st) = instances.get_mut(k) {
                st.pose = *pose;
            }
        }
    }
    let start: Vec<Transform> = solved
        .iter()
        .filter_map(|&k| instances.get(k).map(|i| i.pose))
        .collect();
    let size = model_scale(instances);
    let problem = solver::Problem {
        instances: solved.len(),
        names: solved
            .iter()
            .filter_map(|&k| instances.get(k).map(|i| i.name.clone()))
            .collect(),
        grounded: if floating { vec![0] } else { Vec::new() },
        mates: specs,
        couplings,
        start,
        tolerance: doc.tolerance.linear,
        scale: size,
    };
    // Held by hand: from the placement `from` holds, at its drives with
    // the held motion at its start, to the value along the branch.
    let held_from = hold.zip(from).map(|(hold, _)| {
        let held = hold.held.as_ref();
        let mut drives = Vec::new();
        for (k, spec) in problem.mates.iter().enumerate() {
            for (v, d) in spec.driven.iter().enumerate() {
                let Some(d) = d else { continue };
                let is_held = held.is_some_and(|h| {
                    h.slot == v
                        && k < spec_mate.len()
                        && spec_mate
                            .get(k)
                            .and_then(|&m| assembly.mates.get(m))
                            .is_some_and(|mate| mate.name == h.mate)
                });
                drives.push(match held {
                    Some(h) if is_held => h.start,
                    _ => *d,
                });
            }
        }
        drives
    });
    let solved_by = match (held_from, from) {
        (Some(drives), Some(from)) => solver::advance_near(&problem, &drives, &from.values),
        (Some(drives), None) => solver::advance(&problem, &drives),
        (None, Some(from)) => solver::advance(&problem, &from.drives),
        (None, None) => solver::solve(&problem),
    };
    match solved_by {
        Ok(solution) => {
            if note.is_none() {
                note.clone_from(&solution.note);
            }
            let carried = Carried {
                poses: solution.poses.clone(),
                drives: solution.drives.clone(),
                values: solution.values.clone(),
                dof: solution.dof,
            };
            for (&k, pose) in solved.iter().zip(&solution.poses) {
                if let Some(st) = instances.get_mut(k) {
                    st.pose = *pose;
                }
            }
            for (s, &m) in spec_mate.iter().enumerate() {
                if let Some(st) = states.get_mut(m) {
                    st.free = solution.values.get(s).and_then(|v| v.first()).copied();
                    st.values = solution.values.get(s).cloned().unwrap_or_default();
                    st.main_free = solution.main_free.get(s).copied().unwrap_or(false);
                }
            }
            (
                states,
                coupling_states,
                None,
                floating,
                Some((solution.dof, solution.free)),
                (named, coupling_specs),
                Some(carried),
                note,
            )
        }
        Err(solver::Diagnosis(text)) => (
            states,
            coupling_states,
            Some(text),
            floating,
            None,
            (named, coupling_specs),
            None,
            note,
        ),
    }
}

/// The size of a placed assembly: the diagonal of the box around its
/// placed bodies, one unit without bodies.
pub(crate) fn model_size(placed: &Placed) -> f64 {
    let mut extent = craft3d_math::Bbox3::EMPTY;
    for body in placed.regen.features.iter().filter_map(|f| f.own_body()) {
        if let Some((lo, hi)) = body.held_bbox().ok().and_then(|b| b.corners()) {
            extent.add(lo);
            extent.add(hi);
        }
    }
    let d = extent.diagonal();
    if d.is_finite() && d > 0.0 { d } else { 1.0 }
}

/// The size of the assembly, for the solver's scale: the diagonal of its
/// instances' bodies' boxes, one unit without bodies.
fn model_scale(instances: &[InstanceState]) -> f64 {
    let mut extent = craft3d_math::Bbox3::EMPTY;
    for st in instances.iter().filter(|i| i.error.is_none()) {
        for (_, body, _, _) in bodies_of(st) {
            if let Some((lo, hi)) = body.held_bbox().ok().and_then(|b| b.corners()) {
                extent.add(lo);
                extent.add(hi);
            }
        }
    }
    let d = extent.diagonal();
    if d.is_finite() && d > 0.0 { d } else { 1.0 }
}

impl AsmState {
    /// The folder an assembly at `path` reads its sources from: its own,
    /// or for one without a file the folder they were read from.
    fn base(&self, path: Option<&str>) -> String {
        match path {
            Some(path) => dir_of(path),
            None => self.unsaved_base.clone(),
        }
    }

    /// True when the placement of `revision` is kept (with the files as
    /// they are): [`Self::placed`] answers it without placing.
    pub(crate) fn is_placed(&self, revision: u64) -> bool {
        self.placed
            .as_ref()
            .is_some_and(|(r, g, _)| *r == revision && *g == self.generation)
    }

    /// The placement of `doc` at `revision`, from the cache when nothing
    /// changed since.
    pub(crate) fn placed(
        &mut self,
        doc: &Document,
        path: Option<&str>,
        revision: u64,
    ) -> Arc<Placed> {
        if let Some((r, g, placed)) = &self.placed
            && *r == revision
            && *g == self.generation
        {
            return Arc::clone(placed);
        }
        let mut chain = Vec::new();
        let base = self.base(path);
        let placed = Arc::new(place(doc, &base, self, &mut chain));
        self.placed = Some((revision, self.generation, Arc::clone(&placed)));
        placed
    }

    /// A moment of a motion of the assembly `doc` (the assembly with its
    /// timeline set): placed with the files read for it, its solve
    /// carried from `from` (the moment before it) when given, from the
    /// placements otherwise. Not kept: a motion keeps what it needs.
    pub(crate) fn place_moment(
        &mut self,
        doc: &Document,
        path: Option<&str>,
        from: Option<&Carried>,
    ) -> Placed {
        let mut chain = Vec::new();
        let base = self.base(path);
        place_from(doc, &base, self, &mut chain, from, true, None, None)
    }

    /// [`Self::place_moment`] without building a part's configuration
    /// that is not kept: such an instance is shown with the configuration
    /// `substitutes` gives for its name when that is kept, and is listed
    /// in [`Placed::stale`]. A provisional moment, made at once while the
    /// configurations it lacks are built elsewhere (`motion_build.rs`).
    pub(crate) fn place_ready(
        &mut self,
        doc: &Document,
        path: Option<&str>,
        from: Option<&Carried>,
        substitutes: &BTreeMap<String, String>,
    ) -> Placed {
        let mut chain = Vec::new();
        let base = self.base(path);
        place_from(
            doc,
            &base,
            self,
            &mut chain,
            from,
            true,
            Some(substitutes),
            None,
        )
    }

    /// The parts' configurations `docs` (states of this assembly) ask for
    /// that are not kept, as jobs to build elsewhere, in the order asked
    /// for (each once); and whether a subassembly's is missing too (built
    /// where it is asked for, never elsewhere).
    pub(crate) fn missing_parts(
        &mut self,
        docs: &[Document],
        path: Option<&str>,
    ) -> (Vec<crate::motion_build::PartJob>, bool) {
        let base = self.base(path);
        let mut wanted: BTreeSet<String> = BTreeSet::new();
        let chain: Vec<String> = Vec::new();
        let mut parts = Vec::new();
        let mut subs = false;
        for doc in docs {
            let Some(assembly) = doc.assembly.as_ref() else {
                continue;
            };
            let (_, jobs) = gather(doc, assembly, &base, self, &chain, &mut wanted);
            for job in jobs {
                match job.doc {
                    Ok(source) if !source.is_assembly() => {
                        let cache = job
                            .source
                            .as_ref()
                            .and_then(|s| self.regen_caches.get(s))
                            .cloned();
                        parts.push(crate::motion_build::PartJob {
                            key: job.key,
                            doc: source,
                            source: job.source,
                            cache,
                            slim: job.slim,
                        });
                    }
                    Ok(_) => subs = true,
                    // A source that cannot be configured fails where it
                    // is asked for, at once.
                    Err(_) => subs = true,
                }
            }
        }
        (parts, subs)
    }

    /// The instances of `doc` (a state of this assembly) with the key of
    /// the configuration each asks for, in the assembly's order; nothing
    /// is built.
    pub(crate) fn instance_keys(
        &mut self,
        doc: &Document,
        path: Option<&str>,
    ) -> Vec<(String, String)> {
        let Some(assembly) = doc.assembly.as_ref() else {
            return Vec::new();
        };
        let base = self.base(path);
        let mut wanted: BTreeSet<String> = BTreeSet::new();
        let (instances, _) = gather(doc, assembly, &base, self, &[], &mut wanted);
        instances
            .into_iter()
            .filter_map(|st| Some((st.name, st.config?)))
            .collect()
    }

    /// Lets configurations go until within the budget, none protected for
    /// having been used last (a bake asking what of it the budget keeps).
    pub(crate) fn settle_budget(&mut self) {
        self.tick = self.tick.saturating_add(1);
        self.evict();
    }

    /// Pins `keys` ([`Self::pinned`]) in place of what was pinned.
    pub(crate) fn pin(&mut self, keys: BTreeSet<String>) {
        self.pinned = keys;
    }

    /// The keys of the configurations kept.
    pub(crate) fn config_keys(&self) -> BTreeSet<String> {
        self.configs.keys().cloned().collect()
    }

    /// The parts' configurations kept now that `before` did not hold, as
    /// configurations built elsewhere (for [`Self::take_built`]).
    pub(crate) fn configs_since(
        &self,
        before: &BTreeSet<String>,
    ) -> Vec<crate::motion_build::PartDone> {
        self.configs
            .iter()
            .filter(|(key, c)| {
                !before.contains(*key)
                    && !matches!(&c.built, Ok(b) if matches!(**b, Built::Assembly(_)))
            })
            .map(|(key, c)| crate::motion_build::PartDone {
                key: key.clone(),
                built: c.built.clone(),
                slim: c.slim,
                solved: c.solved && !c.shown,
            })
            .collect()
    }

    /// True when the configuration `key` is kept (built, or known to
    /// fail).
    pub(crate) fn has_config(&self, key: &str) -> bool {
        self.configs.contains_key(key)
    }

    /// Keeps configurations built elsewhere (`motion_build.rs`), in the
    /// order given (sorted by key), as [`build`] keeps those it builds;
    /// then keeps within the budget. One that is kept already stays as it
    /// is (the same, bit for bit).
    pub(crate) fn take_built(&mut self, done: Vec<crate::motion_build::PartDone>) -> usize {
        self.tick = self.tick.saturating_add(1);
        let tick = self.tick;
        let mut taken = 0usize;
        for d in done {
            if self.configs.contains_key(&d.key) {
                continue;
            }
            if d.built.is_ok() {
                self.builds = self.builds.saturating_add(1);
            }
            let solved = d.solved;
            let key = d.key.clone();
            self.keep(d.key, d.built, tick, d.slim);
            if solved && let Some(c) = self.configs.get_mut(&key) {
                c.solved = true;
            }
            taken = taken.saturating_add(1);
        }
        self.evict();
        taken
    }

    /// The solve of a moment as `placed` holds it (a placement of `doc`
    /// with bodies, [`Self::place_moment`]), with its bodies placed again:
    /// the same placement, made without solving. `None` when it cannot be
    /// made so (a configuration it used is no longer kept, a flexible
    /// subassembly): then the moment is placed as usual.
    pub(crate) fn show_solved(&mut self, doc: &Document, solved: &Placed) -> Option<Placed> {
        let assembly = doc.assembly.as_ref()?;
        if solved
            .instances
            .iter()
            .any(|st| st.flexed || st.flex_of.is_some())
        {
            return None;
        }
        self.tick = self.tick.saturating_add(1);
        let tick = self.tick;
        let mut instances = solved.instances.clone();
        for st in &mut instances {
            st.bodies.clear();
            st.built = None;
            let Some(key) = &st.config else { continue };
            match self.configs.get_mut(key) {
                Some(Config {
                    built, used, shown, ..
                }) => {
                    *used = tick;
                    *shown = true;
                    st.built = built.as_ref().ok().cloned();
                }
                None => return None,
            }
        }
        self.placed_copies.begin();
        let mut placed = build_shown(doc, assembly, &mut instances, &mut self.placed_copies);
        self.placed_copies.end();
        placed.specs = solved.specs.clone();
        placed.coupling_specs = solved.coupling_specs.clone();
        placed.mates = solved.mates.clone();
        placed.couplings = solved.couplings.clone();
        placed.solve_error = solved.solve_error.clone();
        placed.floating = solved.floating;
        placed.dof = solved.dof.clone();
        placed.carried = solved.carried.clone();
        placed.solve_note = solved.solve_note.clone();
        Some(placed)
    }

    /// `placed` (a moment placed with its bodies) as a solve to keep for
    /// [`Self::show_solved`]: without its bodies and without holding its
    /// configurations (they are found again by their keys). `None` for one
    /// that cannot be shown so.
    pub(crate) fn solved_of(placed: &Placed) -> Option<Placed> {
        if !placed.stale.is_empty()
            || placed
                .instances
                .iter()
                .any(|st| st.flexed || st.flex_of.is_some())
        {
            return None;
        }
        let mut solved = placed.clone();
        solved.doc = Arc::new(Document::default());
        solved.regen = Arc::new(Regen::default());
        for st in &mut solved.instances {
            st.bodies.clear();
            st.built = None;
        }
        Some(solved)
    }

    /// [`Self::place_moment`] solved only, its bodies not placed: a grid
    /// point of a motion, which keeps the solve's state alone.
    pub(crate) fn solve_moment(
        &mut self,
        doc: &Document,
        path: Option<&str>,
        from: Option<&Carried>,
    ) -> Placed {
        let mut chain = Vec::new();
        let base = self.base(path);
        place_from(doc, &base, self, &mut chain, from, false, None, None)
    }

    /// The assembly `doc` with a joint held by hand for the view
    /// ([`Hold`]), its solve carried from `from` (the document's
    /// placement); its bodies placed only with `bodies`. Not kept.
    pub(crate) fn place_held(
        &mut self,
        doc: &Document,
        path: Option<&str>,
        from: &Carried,
        hold: &Hold,
        bodies: bool,
    ) -> Placed {
        let mut chain = Vec::new();
        let base = self.base(path);
        place_from(
            doc,
            &base,
            self,
            &mut chain,
            Some(from),
            bodies,
            None,
            Some(hold),
        )
    }

    /// Takes `placed` as the placement of the document at `revision`
    /// (a moment of a motion, measured as the document: `motion.check`).
    pub(crate) fn hold(&mut self, revision: u64, placed: Arc<Placed>) {
        self.placed = Some((revision, self.generation, placed));
    }

    /// Forgets the placement, so the next one is worked out again.
    /// A subassembly's placement depends on the files read for its own
    /// sources, which its key does not hold, so those are placed again
    /// too; parts are kept (their key is their bytes).
    pub(crate) fn invalidate(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.placed = None;
        self.configs
            .retain(|_, c| !matches!(&c.built, Ok(b) if matches!(**b, Built::Assembly(_))));
        self.config_bytes = self.configs.values().map(|c| c.bytes).sum();
    }

    /// Keeps a configuration built at `tick` (one built again replaces
    /// the one kept).
    fn keep(&mut self, key: String, built: Result<Arc<Built>, String>, tick: u64, slim: bool) {
        let bytes = built_bytes(&built);
        self.config_bytes = self.config_bytes.saturating_add(bytes);
        if let Some(old) = self.configs.insert(
            key,
            Config {
                built,
                bytes,
                used: tick,
                shown: false,
                slim,
                solved: false,
            },
        ) {
            self.config_bytes = self.config_bytes.saturating_sub(old.bytes);
        }
    }

    /// Lets configurations go until those kept are within the budget:
    /// first those only ever solved (a motion's grid points, which need
    /// them once), then those placed with their bodies, each the least
    /// recently used first, ties in the order of their keys; never one
    /// used at the latest tick (what the placement just made uses). The
    /// regeneration caches of sources none of whose configurations is kept
    /// go with them.
    fn evict(&mut self) {
        let budget = self.config_budget();
        if self.config_bytes > budget {
            let mut order: Vec<(bool, u64, String, usize)> = self
                .configs
                .iter()
                .filter(|(k, c)| c.used < self.tick && !self.pinned.contains(*k))
                .map(|(k, c)| (c.shown, c.used, k.clone(), c.bytes))
                .collect();
            order.sort();
            for (_, _, key, bytes) in order {
                if self.config_bytes <= budget {
                    break;
                }
                self.configs.remove(&key);
                self.config_bytes = self.config_bytes.saturating_sub(bytes);
            }
        }
        if self.regen_caches.is_empty() {
            return;
        }
        let sources: BTreeSet<String> = self
            .configs
            .keys()
            .filter_map(|k| {
                let hash = k.split(['|', '#']).next()?;
                let unit = k.rsplit_once('#').map(|(_, u)| u)?;
                Some(format!("{hash}#{unit}"))
            })
            .collect();
        self.regen_caches
            .retain(|source, _| sources.contains(source));
    }

    /// Lets go of the configurations only ever used by solves without
    /// bodies (a motion's grid points, each solved once and kept as poses)
    /// that `keep` does not hold (the frames of a bake, the grid point
    /// just solved): they would only wait for the budget to push them out.
    pub(crate) fn drop_solved_only(&mut self, keep: &BTreeSet<String>) {
        let gone: Vec<String> = self
            .configs
            .iter()
            .filter(|(key, c)| c.solved && !c.shown && !keep.contains(*key))
            .map(|(key, _)| key.clone())
            .collect();
        for key in gone {
            if let Some(c) = self.configs.remove(&key) {
                self.config_bytes = self.config_bytes.saturating_sub(c.bytes);
            }
        }
    }

    /// Most bytes of configurations kept.
    pub(crate) fn config_budget(&self) -> usize {
        self.config_budget.unwrap_or(DEFAULT_CONFIG_BUDGET)
    }

    /// Keeps at most `bytes` of configurations from now on.
    pub(crate) fn set_config_budget(&mut self, bytes: usize) {
        self.config_budget = Some(bytes);
        self.tick = self.tick.saturating_add(1);
        self.evict();
    }

    /// How many configurations are kept and about how many bytes they take.
    pub(crate) fn configs_held(&self) -> (usize, usize) {
        (self.configs.len(), self.config_bytes)
    }

    /// Builds, side by side, every configuration the assembly states
    /// `docs` ask for that is not kept, at most `limit` of them (in the
    /// order the states ask for them), as a placement of each would build
    /// them; nothing is solved or placed. The motion's bake
    /// (`motion.bake`) builds a whole motion's configurations this way.
    pub(crate) fn prebuild(
        &mut self,
        docs: &[Document],
        path: Option<&str>,
        limit: usize,
    ) -> Prebuilt {
        let base = self.base(path);
        let mut wanted: BTreeSet<String> = BTreeSet::new();
        let mut jobs: Vec<Job> = Vec::new();
        let mut asked: BTreeSet<String> = BTreeSet::new();
        let chain: Vec<String> = Vec::new();
        for doc in docs {
            let Some(assembly) = doc.assembly.as_ref() else {
                continue;
            };
            let (instances, more) = gather(doc, assembly, &base, self, &chain, &mut wanted);
            asked.extend(instances.into_iter().filter_map(|i| i.config));
            jobs.extend(more);
        }
        let remaining = jobs.len().saturating_sub(limit);
        jobs.truncate(limit);
        let built = jobs.len();
        let keys: Vec<String> = jobs.iter().map(|j| j.key.clone()).collect();
        let mut chain = Vec::new();
        build(self, &mut chain, jobs);
        let failed = keys
            .iter()
            .filter(|k| matches!(self.configs.get(*k), Some(Config { built: Err(_), .. })))
            .count();
        // What was just built is held (the states asked for it), the rest
        // goes beyond the budget: a bake in batches holds one batch more.
        self.evict();
        Prebuilt {
            wanted: asked.len(),
            built,
            failed,
            remaining,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn paths_join_and_relate_as_text() {
        assert_eq!(join("a/b", "../c/d.3dcraft").unwrap(), "a/c/d.3dcraft");
        assert_eq!(join("", "../d.3dcraft").unwrap(), "../d.3dcraft");
        assert_eq!(join("/w/x", "p.3dcraft").unwrap(), "/w/x/p.3dcraft");
        assert!(join("/", "../p").is_err());
        assert_eq!(dir_of("a/b/c.3dcraftasm"), "a/b");
        assert_eq!(dir_of("c.3dcraftasm"), "");
        assert_eq!(relative_to("a/b", "a/c/d.3dcraft"), "../c/d.3dcraft");
        assert_eq!(relative_to("", "p/d.3dcraft"), "p/d.3dcraft");
    }

    #[test]
    fn a_placement_turns_then_moves() {
        let pose = placement_pose(&craft3d_doc::Placement {
            position: [1.0, 0.0, 0.0],
            rotation: [0.0, 0.0, 90.0],
        });
        let p = pose.point(Vec3::X);
        assert!(p.distance(Vec3::new(1.0, 1.0, 0.0)) < 1e-12, "{p:?}");
    }

    /// A part made by commands, as its document.
    fn part(commands: &[(&str, Value)]) -> Document {
        let mut s = crate::Session::new(crate::Workspace::none());
        s.execute("file.new", &serde_json::json!({})).unwrap();
        for (id, params) in commands {
            s.execute(id, params).unwrap();
        }
        s.document().unwrap().history.current().as_ref().clone()
    }

    /// A small part: `w` reaches only its last feature, a hole.
    fn holed() -> Document {
        use serde_json::json;
        part(&[
            ("variable.set", json!({"name": "w", "expr": "10 mm"})),
            ("variable.publish", json!({"name": "w"})),
            (
                "feature.block",
                json!({"name": "Plate", "size": [60, 30, 5]}),
            ),
            ("feature.block", json!({"name": "Rib", "size": [60, 3, 12]})),
            (
                "feature.boolean",
                json!({"op": "union", "target": "Plate", "tool": "Rib", "name": "Ribbed"}),
            ),
            (
                "feature.cylinder",
                json!({"name": "Pin", "radius": 2, "height": 20, "position": ["w", 15, -5]}),
            ),
            (
                "feature.boolean",
                json!({"op": "subtract", "target": "Ribbed", "tool": "Pin", "name": "Holed"}),
            ),
        ])
    }

    /// The belt's pattern: `xc` moves a run and sets how many teeth a
    /// pattern has, and everything is united; one tooth does not depend
    /// on it.
    fn belt_like() -> Document {
        use serde_json::json;
        part(&[
            ("variable.set", json!({"name": "xc", "expr": "30 mm"})),
            ("variable.publish", json!({"name": "xc"})),
            (
                "variable.set",
                json!({"name": "n", "expr": "floor(xc / 7 mm) + 1"}),
            ),
            (
                "feature.block",
                json!({"name": "Back", "size": ["xc + 10 mm", 4, 1.5]}),
            ),
            (
                "feature.block",
                json!({"name": "Run", "size": [20, 4, 1.5], "position": ["xc + 12 mm", 0, 0]}),
            ),
            (
                "feature.block",
                json!({"name": "Tooth", "size": [2, 4, 1.2], "position": [1, 0, -1]}),
            ),
            (
                "feature.pattern",
                json!({"name": "Teeth", "source": "Tooth", "count": "n", "step": ["5 mm", 0, 0]}),
            ),
            (
                "feature.boolean",
                json!({"op": "union", "target": "Back", "tool": "Teeth", "name": "U0"}),
            ),
            (
                "feature.boolean",
                json!({"op": "union", "target": "U0", "tool": "Run", "name": "Belt"}),
            ),
        ])
    }

    /// The configurations of `source` for `name` = each of `values`, built
    /// one after the other as a motion builds them (each from the
    /// regeneration cache of the one before), against a regeneration from
    /// nothing of the same configuration: the same results, feature for
    /// feature and bit for bit (their `Debug`, which writes every number
    /// so that it reads back to the same bits). Returns how many features
    /// were rebuilt in all.
    fn built_as_fresh(source: &Document, name: &str, values: &[&str]) -> usize {
        let mut state = AsmState::default();
        let mut rebuilt = 0;
        for value in values {
            let params: BTreeMap<String, String> =
                [(name.to_string(), (*value).to_string())].into();
            let doc = configured(source, &params, source.units.length).unwrap();
            let key = format!("{}#mm", config_key("src", &params));
            let job = Job {
                key: key.clone(),
                doc: Ok(doc.clone()),
                base: String::new(),
                source: Some("src#mm".to_string()),
                slim: false,
            };
            build(&mut state, &mut Vec::new(), vec![job]);
            let Some(Config {
                built: Ok(built), ..
            }) = state.configs.get(&key)
            else {
                panic!("{value}: not built");
            };
            let Built::Part { regen, .. } = &**built else {
                panic!("{value}: not a part");
            };
            let fresh = craft3d_regen::regenerate(&doc);
            assert!(fresh.is_ok(), "{value}: {:?}", fresh.features);
            assert_eq!(regen.features.len(), fresh.features.len());
            for (a, b) in regen.features.iter().zip(&fresh.features) {
                assert_eq!(format!("{a:?}"), format!("{b:?}"), "{value}: {}", a.id);
            }
            assert_eq!(
                format!("{:?}", regen.variables),
                format!("{:?}", fresh.variables)
            );
            let cache = state.regen_caches.get("src#mm").unwrap();
            rebuilt += cache.last_run().0;
        }
        rebuilt
    }

    #[test]
    fn a_configuration_built_from_the_last_one_is_a_fresh_regeneration_bit_for_bit() {
        let holed = holed();
        let features = holed.features.len();
        let values = ["10 mm", "12.5 mm", "40 mm", "12.5 mm", "10 mm"];
        let rebuilt = built_as_fresh(&holed, "w", &values);
        // The first from nothing, then only the hole and the cut.
        assert_eq!(rebuilt, features + 2 * (values.len() - 1), "{rebuilt}");
        let belt = belt_like();
        let values = ["30 mm", "31 mm", "44.5 mm", "29 mm", "30 mm", "57 mm"];
        let rebuilt = built_as_fresh(&belt, "xc", &values);
        // The tooth is never built again.
        assert!(
            rebuilt <= belt.features.len() + (belt.features.len() - 1) * (values.len() - 1),
            "{rebuilt}"
        );
    }

    #[test]
    fn configurations_are_kept_within_a_byte_budget_least_recently_used_first() {
        let holed = holed();
        let mut state = AsmState::default();
        let mut keys = Vec::new();
        for (i, value) in ["10 mm", "20 mm", "30 mm", "40 mm"].iter().enumerate() {
            let params: BTreeMap<String, String> = [("w".to_string(), (*value).to_string())].into();
            let key = format!("{}#mm", config_key("src", &params));
            let job = Job {
                key: key.clone(),
                doc: configured(&holed, &params, holed.units.length),
                base: String::new(),
                source: Some("src#mm".to_string()),
                slim: false,
            };
            build(&mut state, &mut Vec::new(), vec![job]);
            keys.push(key);
            assert_eq!(state.configs.len(), i + 1);
        }
        let (count, bytes) = state.configs_held();
        assert_eq!(count, 4);
        let each = bytes / 4;
        assert!(each > RECORDS, "{each}");
        // Use the first again, then keep room for two and a bit: the second
        // and third (least recently used) go.
        state.tick = state.tick.saturating_add(1);
        let tick = state.tick;
        state.configs.get_mut(&keys[0]).unwrap().used = tick;
        state.set_config_budget(each * 2 + each / 2);
        let kept: Vec<&String> = state.configs.keys().collect();
        assert_eq!(kept, vec![&keys[0], &keys[3]], "{kept:?}");
        assert!(state.configs_held().1 <= each * 2 + each / 2);
        assert!(state.regen_caches.contains_key("src#mm"));
        // Nothing left of a source: its regeneration cache goes too.
        state.set_config_budget(0);
        assert!(state.configs.is_empty());
        assert!(state.regen_caches.is_empty());
    }
}
