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
/// Configurations kept beyond those the last placement used.
const MAX_KEPT_CONFIGS: usize = 64;

/// What an open assembly has derived, kept between commands. Not saved.
#[derive(Clone, Default)]
pub(crate) struct AsmState {
    /// The source files read, by their path as [`join`] writes it (from
    /// the read root): their bytes, or why they could not be read.
    pub files: BTreeMap<String, Result<Blob, String>>,
    /// Source documents, by the SHA-256 of their bytes.
    parsed: BTreeMap<String, Result<Arc<Document>, String>>,
    /// Configurations built, by [`config_key`].
    configs: BTreeMap<String, Result<Arc<Built>, String>>,
    /// Raised whenever `files` changes, so the placement is redone.
    pub generation: u64,
    /// The last placement: for the document revision and the generation.
    placed: Option<(u64, u64, Arc<Placed>)>,
    /// How many configurations were regenerated so far (a count for
    /// `assembly.inspect` and the tests: two instances of one
    /// configuration cost one).
    pub builds: u64,
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
    /// Why the mate takes no part, when it does not.
    pub error: Option<String>,
    /// The value of its main free motion after the solve (radians or
    /// model units), for a mate that has one.
    pub free: Option<f64>,
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
    /// The state the solve reached, for the next moment of a motion to
    /// be carried from (`motion.rs`); `None` when it failed.
    pub carried: Option<Carried>,
    /// A moment of a motion that could not be carried from the moment
    /// before it (the instances or the driven values taking part changed
    /// in between) and was solved from the placements instead: why. It
    /// may then lie on another branch than the motion's.
    pub solve_note: Option<String>,
}

/// A solved state of an assembly's motion: the poses of the instances
/// that took part in the solve (in their order) and the driven values
/// they were solved at. The next moment starts from it
/// ([`AsmState::place_moment`]).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Carried {
    pub poses: Vec<Transform>,
    pub drives: Vec<f64>,
    /// The degrees of freedom left there.
    pub dof: usize,
}

/// A mate as the solver takes it, its two sides by instance name (`None`
/// for the world) rather than by index.
#[derive(Clone)]
pub(crate) struct NamedSpec {
    pub a: Option<String>,
    pub b: Option<String>,
    pub spec: solver::MateSpec,
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

/// The folder an assembly's source paths are relative to: that of its
/// file, or the read root for one not saved yet.
pub(crate) fn base_of(path: Option<&str>) -> String {
    path.map(dir_of).unwrap_or_default()
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

/// A length in base units (mm) in `doc`'s length unit.
fn in_units(doc: &Document, millimetres: f64) -> f64 {
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

/// The bytes of an instance's source and whether they differ from the
/// hash the document holds.
fn source_bytes(
    assembly: &Assembly,
    instance: &Instance,
    base: &str,
    files: &BTreeMap<String, Result<Blob, String>>,
) -> Result<(Blob, bool), String> {
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
            return Ok((packed.clone(), false));
        }
        return Err(format!(
            "it is pinned to {}…, and its source is now {}… (instance.edit with pinned: null follows it)",
            pinned.get(..12).unwrap_or(pinned),
            now.get(..12).unwrap_or(&now)
        ));
    }
    Ok((blob, *recorded != now))
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

/// The source with its public parameters set.
fn configured(source: &Document, params: &BTreeMap<String, String>) -> Result<Document, String> {
    let mut doc = source.clone();
    if params.is_empty() {
        return Ok(doc);
    }
    let given: Map<String, Value> = params
        .iter()
        .map(|(k, v)| (k.clone(), Value::String(v.clone())))
        .collect();
    let (sources, _) = crate::param_cmds::prepare_set("instance", source, &given, false)
        .map_err(|e| e.message().trim_start_matches("instance: ").to_string())?;
    for (name, text) in &sources {
        doc.set_variable(name, text).map_err(|e| e.to_string())?;
    }
    Ok(doc)
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
    let (blob, changed) = source_bytes(assembly, instance, base, files)?;
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
    if source.units.length != doc.units.length {
        return Err(format!(
            "its source is in {} and the assembly in {}: a source in another length unit is not supported yet",
            source.units.length.symbol(),
            doc.units.length.symbol()
        ));
    }
    let params = override_values(instance, values)?;
    Ok((hash, source, params, changed))
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
/// given, and whether its file changed.
type Prepared = (String, Arc<Document>, BTreeMap<String, String>, bool);

/// What one configuration needs to be built: its source document (set)
/// and, for a subassembly, the folder its own sources are relative to.
struct Job {
    key: String,
    doc: Result<Document, String>,
    base: String,
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
    place_from(doc, base, state, chain, None, true)
}

/// [`place`], its solve carried from `from` when given: the instances
/// start where `from` left them and the drives move from `from`'s values
/// to the document's along the motion's branch (`Solver::advance`).
/// Without `bodies` the bodies are not placed (a grid point of a motion
/// needs the solve alone): the placed part is empty.
fn place_from(
    doc: &Document,
    base: &str,
    state: &mut AsmState,
    chain: &mut Vec<String>,
    from: Option<&Carried>,
    bodies: bool,
) -> Placed {
    let Some(assembly) = doc.assembly.as_ref() else {
        return empty(doc, Some("the document is not an assembly".to_string()));
    };
    let values = evaluation(doc);
    let files = state.files.clone();
    // 1. Sources and configurations.
    let mut instances: Vec<InstanceState> = Vec::with_capacity(assembly.instances.len());
    let mut jobs: Vec<Job> = Vec::new();
    let mut wanted: BTreeSet<String> = BTreeSet::new();
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
        .map(|(hash, source, params, changed)| {
            st.changed = changed;
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
                if source.is_assembly() {
                    // A subassembly's placement also depends on the folder
                    // its own sources are read from: two copies of one file
                    // in two folders are two configurations.
                    key.push('@');
                    key.push_str(&sub_base);
                }
                st.hash = Some(hash.clone());
                st.config = Some(key.clone());
                if !state.configs.contains_key(&key) && wanted.insert(key.clone()) {
                    jobs.push(Job {
                        key,
                        doc: configured(&source, &params),
                        base: sub_base,
                    });
                }
            }
            Err(why) => st.error = Some(why),
        }
        instances.push(st);
    }
    // 2. Build what is not cached: the parts side by side, subassemblies
    // one after the other (each builds its own parts side by side).
    let (subs, parts): (Vec<Job>, Vec<Job>) = jobs
        .into_iter()
        .partition(|job| job.doc.as_ref().is_ok_and(Document::is_assembly));
    let indices: Vec<usize> = (0..parts.len()).collect();
    let built = crate::interference_cmds::in_order(&indices, |i| {
        let job = parts.get(i)?;
        Some(match &job.doc {
            Ok(doc) => {
                let regen = craft3d_regen::regenerate(doc);
                Ok(Arc::new(Built::Part {
                    doc: Arc::new(doc.clone()),
                    regen: Arc::new(regen),
                }))
            }
            Err(why) => Err(why.clone()),
        })
    });
    for (job, result) in parts.iter().zip(built) {
        if job.doc.is_ok() {
            state.builds = state.builds.saturating_add(1);
        }
        let result = result.unwrap_or_else(|| Err("its regeneration stopped".to_string()));
        state.configs.insert(job.key.clone(), result);
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
        state.configs.insert(job.key, result);
    }
    for (st, instance) in instances.iter_mut().zip(&assembly.instances) {
        if st.error.is_some() {
            continue;
        }
        let Some(key) = &st.config else { continue };
        match state.configs.get(key) {
            Some(Ok(built)) => {
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
            Some(Err(why)) => st.error = Some(format!("its configuration failed: {why}")),
            None => st.error = Some("its configuration was not built".to_string()),
        }
    }
    // A flexible subassembly gives its instances and mates to the solve.
    let extra = flex(assembly, &mut instances);
    // 3. and 4. Frames and the solve.
    let (mates, couplings, solve_error, floating, dof, specs, carried, solve_note) =
        solve_mates(doc, assembly, values.as_ref(), &mut instances, &extra, from);
    // 5. Placed bodies.
    let mut placed = if bodies {
        build_shown(doc, assembly, &mut instances)
    } else {
        let mut placed = empty(doc, None);
        placed.instances = instances;
        placed
    };
    placed.specs = specs;
    placed.mates = mates;
    placed.couplings = couplings;
    placed.solve_error = solve_error;
    placed.floating = floating;
    placed.dof = dof;
    placed.carried = carried;
    placed.solve_note = solve_note;
    // Keep the cache to what this placement used and a few more.
    if state.configs.len() > MAX_KEPT_CONFIGS {
        let used: BTreeSet<String> = placed
            .instances
            .iter()
            .filter_map(|i| i.config.clone())
            .collect();
        state.configs.retain(|k, _| used.contains(k));
    }
    placed
}

/// Each flexible subassembly's instances added to `instances` as
/// `Sub/Inner`, starting where the subassembly puts them, and its mates
/// (by those names), its world being the subassembly's placement. The
/// subassembly itself is then not solved.
fn flex(assembly: &Assembly, instances: &mut Vec<InstanceState>) -> Vec<NamedSpec> {
    let mut extra = Vec::new();
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
        for named in &sub.specs {
            let mut spec = named.spec.clone();
            // The subassembly's world is where it is placed.
            if named.a.is_none() {
                spec.a.1 = spec.a.1.moved(&anchor);
            }
            if named.b.is_none() {
                spec.b.1 = spec.b.1.moved(&anchor);
            }
            extra.push(NamedSpec {
                a: named.a.as_deref().map(prefixed),
                b: named.b.as_deref().map(prefixed),
                spec,
            });
        }
    }
    instances.extend(added);
    extra
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
        carried: None,
        solve_note: None,
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
fn build_shown(doc: &Document, assembly: &Assembly, instances: &mut [InstanceState]) -> Placed {
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
            let placed = match body.transformed(&st.pose) {
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
                body: Some(Arc::new(placed)),
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
        carried: None,
        solve_note: None,
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

type Solved = (
    Vec<MateState>,
    Vec<CouplingState>,
    Option<String>,
    bool,
    Option<(usize, Vec<String>)>,
    Vec<NamedSpec>,
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
    extra: &[NamedSpec],
    from: Option<&Carried>,
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
    let length = |q: Quantity| in_units(doc, q.value);
    for (m, mate) in assembly.mates.iter().enumerate() {
        let mut st = MateState {
            name: mate.name.clone(),
            error: None,
            free: None,
            driven: false,
            main_free: false,
        };
        let spec = mate_spec(doc, mate, instances, &index, values);
        match spec {
            Ok(spec) => {
                st.driven = spec.driven.iter().any(Option::is_some);
                specs.push(spec);
                spec_mate.push(m);
            }
            Err(why) => st.error = Some(why),
        }
        states.push(st);
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
        let taken = at(&c.mates.0).and_then(|a| Ok((a, at(&c.mates.1)?, value?)));
        coupling_states.push(CouplingState {
            name: c.name.clone(),
            error: taken.as_ref().err().cloned(),
        });
        if let Ok((a, b, value)) = taken {
            couplings.push(solver::CouplingSpec {
                kind: match c.kind {
                    craft3d_doc::CouplingKind::Gear => solver::CouplingKind::Gear,
                    craft3d_doc::CouplingKind::Rack => solver::CouplingKind::Rack,
                    craft3d_doc::CouplingKind::Screw => solver::CouplingKind::Screw,
                },
                mates: (a, b),
                value,
            });
        }
    }
    // Its own mates by instance name, for a parent that holds it as a
    // flexible subassembly.
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
        .map(|spec| NamedSpec {
            a: name_of(spec.a.0),
            b: name_of(spec.b.0),
            spec: spec.clone(),
        })
        .collect();
    // The mates of its flexible subassemblies, by the names of their
    // instances here.
    for e in extra {
        let at = |name: &Option<String>| match name {
            None => Some(solver::WORLD),
            Some(n) => index.get(n).copied(),
        };
        if let (Some(a), Some(b)) = (at(&e.a), at(&e.b)) {
            let mut spec = e.spec.clone();
            spec.a.0 = a;
            spec.b.0 = b;
            specs.push(spec);
        }
    }
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
    let solved_by = match from {
        Some(from) => solver::advance(&problem, &from.drives),
        None => solver::solve(&problem),
    };
    match solved_by {
        Ok(solution) => {
            if note.is_none() {
                note.clone_from(&solution.note);
            }
            let carried = Carried {
                poses: solution.poses.clone(),
                drives: solution.drives.clone(),
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
                    st.main_free = solution.main_free.get(s).copied().unwrap_or(false);
                }
            }
            (
                states,
                coupling_states,
                None,
                floating,
                Some((solution.dof, solution.free)),
                named,
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
            named,
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
        let placed = Arc::new(place(doc, &base_of(path), self, &mut chain));
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
        place_from(doc, &base_of(path), self, &mut chain, from, true)
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
        place_from(doc, &base_of(path), self, &mut chain, from, false)
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
            .retain(|_, built| !matches!(built, Ok(b) if matches!(**b, Built::Assembly(_))));
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
}
