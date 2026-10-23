//! Regenerating only what an edit touches.
//!
//! The result of a feature is a function of its own definition, the values
//! of its expressions, the document's units and tolerance, and the results
//! of the features it is made from. [`RegenCache`] remembers, per feature,
//! what those were and what came out; when they are all the same the
//! result is reused, otherwise the feature is rebuilt. A result that comes
//! out the same as before keeps its stamp, so what is made from it is not
//! rebuilt either.
//!
//! The cache changes how long regeneration takes and nothing else:
//! [`crate::regenerate_cached`] returns exactly what [`crate::regenerate`]
//! does.

use crate::sketch::{Placement, SketchResult, solve_sketch};
use crate::{FeatureResult, Status};
use craft3d_doc::{Document, Feature, FeatureId, FeatureKind, Sketch, Tolerance, Units};
use craft3d_expr::{Dimension, Evaluation};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// The value an expression has, or why it has none: exact, so a change in
/// the last bit is a change.
type Evaluated = Result<(u64, Dimension), String>;

fn evaluated(variables: &Evaluation, exprs: &[(String, &craft3d_doc::Expr)]) -> Vec<Evaluated> {
    exprs
        .iter()
        .map(|(_, expr)| {
            variables
                .evaluate(expr.source())
                .map(|q| (q.value.to_bits(), q.dimension))
                .map_err(|e| e.to_string())
        })
        .collect()
}

fn bits(tolerance: Tolerance) -> (u64, u64) {
    (tolerance.linear.to_bits(), tolerance.angular.to_bits())
}

/// A feature this one is made from, directly or through others, as it was
/// when this one was built.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Input {
    id: FeatureId,
    /// Its name and kind, which the messages of a failure quote: also
    /// when it comes later in the history, where it has no stamp that a
    /// rename would change. `None` when there is no such feature.
    named: Option<(Arc<str>, &'static str)>,
    /// The stamp of its result; `None` when it did not come earlier in the
    /// history (or does not exist).
    stamp: Option<u64>,
    /// The later feature its body had gone into by then.
    taken_by: Option<(FeatureId, Arc<str>)>,
    /// For one that does not come earlier: whether it is itself made from
    /// this feature, which the reason of the failure says
    /// ([`crate::comes_after`]).
    circle: bool,
}

/// Everything but the feature's own definition that its result depends on.
#[derive(Clone, Debug, PartialEq)]
struct Context {
    rolled_back: bool,
    /// The feature with its names followed to the newest body of their
    /// chain, or why they could not be (`crate::follow`).
    followed: Result<Option<Feature>, String>,
    values: Vec<Evaluated>,
    units: Units,
    tolerance: (u64, u64),
    inputs: Vec<Input>,
    /// For a split: the other splits of the same body as they stand,
    /// each with whether it is built (`crate::split::group_state`), which
    /// its result depends on (a lump is one split's, and the last split
    /// answers for lumps none names). Empty for every other kind.
    group: Vec<(Feature, bool)>,
}

#[derive(Clone)]
struct Entry {
    feature: Feature,
    context: Context,
    /// As built: before a later feature took the body.
    result: FeatureResult,
    stamp: u64,
    /// About what the result's body takes in memory (`crate::body_bytes`);
    /// 0 without one.
    bytes: usize,
    /// The body was let go of to stay within [`RegenCache::body_budget`]
    /// (`result.body` is `None`, and its names are without those of its
    /// edges and vertices); it is derived again from the results before
    /// it when a feature built from it needs it, the same bits.
    evicted: bool,
}

/// A sketch as it was solved, and how it solved on the flat plane (where a
/// sketch lies does not change how it solves).
#[derive(Clone)]
struct Solve {
    sketch: Sketch,
    values: Vec<Evaluated>,
    units: Units,
    tolerance: (u64, u64),
    outcome: (Status, Option<SketchResult>),
}

/// What one document's last regeneration found, per feature. Keep one per
/// open document and pass it to every regeneration of that document; using
/// it for another document is safe (everything is compared), only useless.
#[derive(Clone, Default)]
pub struct RegenCache {
    entries: BTreeMap<FeatureId, Entry>,
    solves: BTreeMap<FeatureId, Solve>,
    next_stamp: u64,
    /// What the last regeneration found of each feature, in the order of
    /// its history: what the next one compares with to know, without
    /// looking at everything a feature is made from, that none of it
    /// changed ([`Step`]).
    trail: Vec<(FeatureId, Step)>,
    /// Features rebuilt and reused by the last regeneration.
    rebuilt: usize,
    reused: usize,
    /// How many entries of what features are made from the last
    /// regeneration looked at ([`RegenCache::last_run_work`]).
    work: usize,
    /// Most bytes of bodies that went into later features that are kept
    /// ([`RegenCache::set_body_budget`]); `None` for the default.
    body_budget: Option<usize>,
    /// Bodies the last regeneration derived again because a feature built
    /// from them needed them.
    rederived: usize,
}

/// Most bytes of bodies that went into later features that a cache keeps
/// by default: enough for every body of a history of hundreds of unions
/// of a thousand faces each, and bounded, so that a history whose bodies
/// together take more (a chain of unions stores one body per step, each
/// as big as everything before it) does not grow without end.
#[cfg(not(target_arch = "wasm32"))]
pub const DEFAULT_BODY_BUDGET: usize = 1 << 30;
#[cfg(target_arch = "wasm32")]
pub const DEFAULT_BODY_BUDGET: usize = 1 << 28;

/// Which bodies a regeneration must leave in its result, besides those
/// of the document (the bodies no later feature took, which are always
/// there): a body that went into a later feature may have been let go of
/// to stay within the cache's budget ([`RegenCache::set_body_budget`]),
/// and is derived again on request.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Need<'a> {
    /// Only what regeneration itself needs.
    #[default]
    Bodies,
    /// Every feature's body: what [`crate::regenerate`] gives.
    All,
    /// The bodies of these features.
    These(&'a [FeatureId]),
}

impl Need<'_> {
    fn includes(&self, id: FeatureId) -> bool {
        match self {
            Need::Bodies => false,
            Need::All => true,
            Need::These(ids) => ids.contains(&id),
        }
    }
}

/// What a regeneration found of one feature that the features after it
/// can see: its name and kind (which reasons quote), the stamp of its
/// result, and the bodies it took. When every feature a feature is made
/// from, directly or through others, has the same step as the last time,
/// and so has every feature that took one of their bodies before it, all
/// of its context but its own definition is what it was.
#[derive(Clone, Debug, PartialEq)]
struct Step {
    named: (Arc<str>, &'static str),
    stamp: u64,
    takes: Vec<FeatureId>,
}

impl RegenCache {
    /// How many features the last regeneration rebuilt and how many it
    /// reused.
    pub fn last_run(&self) -> (usize, usize) {
        (self.rebuilt, self.reused)
    }

    /// How many entries of what features are made from the last
    /// regeneration put together and compared: the features made from
    /// others, directly or through others, summed over the features whose
    /// context had to be looked at in full. A measure of the work that is
    /// not building, which tests hold to grow with the length of the
    /// history and not with its square when one feature is added.
    pub fn last_run_work(&self) -> usize {
        self.work
    }

    /// How many bodies the last regeneration derived again, having let go
    /// of them to stay within the budget, because a feature built from
    /// them needed them.
    pub fn last_run_rederived(&self) -> usize {
        self.rederived
    }

    /// Most bytes of bodies that went into later features that are kept
    /// between regenerations ([`DEFAULT_BODY_BUDGET`] unless set). The
    /// bodies of the document itself are always kept. Beyond the budget
    /// the oldest in the history are let go of first, and derived again
    /// from the results before them when an edit needs them: an edit
    /// near the end of a long history then costs what it costs, and one
    /// further up at most a full regeneration more. Results do not
    /// change: a body derived again is the same bits.
    pub fn set_body_budget(&mut self, bytes: usize) {
        self.body_budget = Some(bytes);
    }

    pub fn body_budget(&self) -> usize {
        self.body_budget.unwrap_or(DEFAULT_BODY_BUDGET)
    }

    /// About how many bytes the bodies kept take that went into later
    /// features, and how many such bodies were let go of.
    pub fn bodies_held(&self) -> (usize, usize) {
        let taken: BTreeSet<FeatureId> = self
            .trail
            .iter()
            .flat_map(|(_, step)| step.takes.iter().copied())
            .collect();
        self.entries
            .iter()
            .filter(|(id, _)| taken.contains(id))
            .fold((0, 0), |(bytes, gone), (_, entry)| {
                if entry.evicted {
                    (bytes, gone.saturating_add(1))
                } else {
                    (bytes.saturating_add(entry.bytes), gone)
                }
            })
    }

    /// Solves `sketch` on the flat plane, or returns how it solved the
    /// last time when nothing it depends on has changed.
    pub(crate) fn solve(
        &mut self,
        id: FeatureId,
        doc: &Document,
        variables: &Evaluation,
        sketch: &Sketch,
    ) -> (Status, Option<SketchResult>) {
        let values = evaluated(variables, &sketch.exprs());
        let tolerance = bits(doc.tolerance);
        if let Some(solve) = self.solves.get(&id)
            && solve.units == doc.units
            && solve.tolerance == tolerance
            && solve.values == values
            && solve.sketch == *sketch
        {
            return solve.outcome.clone();
        }
        let outcome = solve_sketch(doc, variables, sketch, Ok(Placement::FLAT));
        self.solves.insert(
            id,
            Solve {
                sketch: sketch.clone(),
                values,
                units: doc.units,
                tolerance,
                outcome: outcome.clone(),
            },
        );
        outcome
    }

    /// Forgets the features that are no longer in the document.
    fn prune(&mut self, doc: &Document) {
        let present: BTreeSet<FeatureId> = doc.features.iter().map(|f| f.id).collect();
        self.entries.retain(|id, _| present.contains(id));
        self.solves.retain(|id, _| present.contains(id));
    }
}

/// Every feature `feature` is made from, directly or through others, in
/// ascending order of id. Bounded by the number of features, so a cycle in
/// a damaged document ends.
pub(crate) fn dependencies(doc: &Document, feature: &Feature) -> Vec<FeatureId> {
    Lookup::new(doc).dependencies(feature)
}

/// What one regeneration looks up again and again, by feature id: the
/// features, and what each is made from (filled in as it is asked for).
/// A row of a thousand features each made from the one before asks for
/// every one of them for each of them; looked up in the history one by one
/// that was the cube of the number of features.
struct Lookup<'a> {
    doc: &'a Document,
    by_id: BTreeMap<FeatureId, (usize, &'a Feature)>,
    names: BTreeMap<FeatureId, Arc<str>>,
    made_from: BTreeMap<FeatureId, Arc<[FeatureId]>>,
    /// Entries of what features are made from that were looked at.
    work: usize,
}

impl<'a> Lookup<'a> {
    fn new(doc: &'a Document) -> Self {
        let mut by_id = BTreeMap::new();
        let mut names = BTreeMap::new();
        for (index, feature) in doc.features.iter().enumerate() {
            // The first of a repeated id, as `Document::feature` finds.
            by_id.entry(feature.id).or_insert((index, feature));
            names
                .entry(feature.id)
                .or_insert_with(|| Arc::from(feature.name.as_str()));
        }
        Self {
            doc,
            by_id,
            names,
            made_from: BTreeMap::new(),
            work: 0,
        }
    }

    fn feature(&self, id: FeatureId) -> Option<&'a Feature> {
        self.by_id.get(&id).map(|(_, f)| *f)
    }

    /// What a feature names, for a feature that looks at every body
    /// built so far (an extrusion through all) every feature before it in
    /// the history, and for a split the splits of the same body before
    /// it (a lump is one split's: `crate::split`).
    fn inputs(&self, feature: &Feature) -> Vec<FeatureId> {
        let mut inputs = feature.kind.inputs();
        if feature.kind.uses_all_earlier() {
            inputs.extend(
                self.doc
                    .features
                    .iter()
                    .take_while(|f| f.id != feature.id)
                    .map(|f| f.id),
            );
        }
        if let FeatureKind::Split(split) = &feature.kind {
            inputs.extend(
                self.doc
                    .features
                    .iter()
                    .take_while(|f| f.id != feature.id)
                    .filter(|f| {
                        matches!(&f.kind, FeatureKind::Split(other) if other.source == split.source)
                    })
                    .map(|f| f.id),
            );
        }
        inputs
    }

    /// [`dependencies`] by a walk of the graph, as it always was.
    fn walked(&mut self, feature: &Feature) -> Vec<FeatureId> {
        let mut seen = BTreeSet::new();
        let mut open = self.inputs(feature);
        while let Some(id) = open.pop() {
            self.work = self.work.saturating_add(1);
            if id == feature.id || !seen.insert(id) {
                continue;
            }
            if let Some(input) = self.feature(id) {
                open.extend(self.inputs(input));
            }
        }
        seen.into_iter().collect()
    }

    /// What the feature of the history with id `id` is made from,
    /// remembered.
    fn made_from(&mut self, id: FeatureId) -> Arc<[FeatureId]> {
        if let Some(known) = self.made_from.get(&id) {
            return known.clone();
        }
        let found: Arc<[FeatureId]> = match self.feature(id) {
            Some(feature) => self.walked(feature).into(),
            None => Arc::from(Vec::new()),
        };
        self.made_from.insert(id, found.clone());
        found
    }

    /// [`dependencies`]: what everything `feature` names is made from,
    /// with those themselves and without `feature`. The same set as the
    /// walk (what can be reached from an input is that input and what can
    /// be reached from it, cycles or not), from what is known of the
    /// inputs: in a history in order they all came before.
    fn dependencies(&mut self, feature: &Feature) -> Vec<FeatureId> {
        // Each list is in ascending order, so sorting them together is a
        // merge of runs.
        let mut all = Vec::new();
        for input in self.inputs(feature) {
            all.push(input);
            if self.feature(input).is_some() {
                all.extend(self.made_from(input).iter().copied());
            }
        }
        self.work = self.work.saturating_add(all.len());
        all.sort();
        all.dedup();
        all.retain(|id| *id != feature.id);
        all
    }
}

/// The features whose [`Step`] is not what it was in the last
/// regeneration, the bodies they took or no longer take, and everything
/// made from any of those, directly or through others: what cannot be
/// known to have the context it had without looking at it in full. Marked
/// as the regeneration goes; a mark on a feature already built changes
/// nothing of it, only of the features made from it that come later.
struct Changed {
    marked: BTreeSet<FeatureId>,
    /// The features that name each feature (`FeatureKind::inputs`), made
    /// when the first feature is marked.
    users: Option<BTreeMap<FeatureId, Vec<FeatureId>>>,
    /// The features that are made from every feature before them
    /// (`FeatureKind::uses_all_earlier`), by their place in the history.
    all_earlier: Vec<(usize, FeatureId)>,
    /// Every one of `all_earlier` after this place is marked.
    all_earlier_after: Option<usize>,
}

impl Changed {
    fn new(doc: &Document) -> Self {
        Self {
            marked: BTreeSet::new(),
            users: None,
            all_earlier: doc
                .features
                .iter()
                .enumerate()
                .filter(|(_, f)| f.kind.uses_all_earlier())
                .map(|(place, f)| (place, f.id))
                .collect(),
            all_earlier_after: None,
        }
    }

    fn is_marked(&self, id: FeatureId) -> bool {
        self.marked.contains(&id)
    }

    /// Marks `id` and everything made from it. Each feature is marked at
    /// most once, so all the marks of one regeneration together take time
    /// in proportion to the history and what its features name.
    fn mark(&mut self, doc: &Document, lookup: &Lookup, id: FeatureId) {
        if !self.marked.insert(id) {
            return;
        }
        let users = self.users.get_or_insert_with(|| {
            let mut users: BTreeMap<FeatureId, Vec<FeatureId>> = BTreeMap::new();
            for feature in &doc.features {
                for input in feature.kind.inputs() {
                    users.entry(input).or_default().push(feature.id);
                }
            }
            users
        });
        let mut open = vec![id];
        while let Some(at) = open.pop() {
            for user in users.get(&at).map(Vec::as_slice).unwrap_or_default() {
                if self.marked.insert(*user) {
                    open.push(*user);
                }
            }
            // The features made from every one before them, after this
            // one: those after the last place this was done for are
            // marked already.
            let Some(place) = lookup.by_id.get(&at).map(|(place, _)| *place) else {
                continue;
            };
            if self.all_earlier_after.is_some_and(|after| after <= place) {
                continue;
            }
            let after = self.all_earlier_after;
            for (later, user) in &self.all_earlier {
                if *later > place
                    && after.is_none_or(|after| *later <= after)
                    && self.marked.insert(*user)
                {
                    open.push(*user);
                }
            }
            self.all_earlier_after = Some(place);
        }
    }
}

/// Runs `build` for every feature of the document that cannot reuse what
/// it gave the last time. `build` gets the feature, whether it is rolled
/// back, and the results so far; it returns the feature's result and the
/// earlier feature whose body it took, if any.
///
/// A feature reuses its result when its definition and its context are
/// what they were ([`Context`]). The context lists everything the feature
/// is made from, directly or through others, which in a long row of
/// features is most of the history: put together for every feature, that
/// is the square of the length of the history for each regeneration. So
/// the context is put together only for a feature that may have a new
/// one. One that stands where it stood in the last regeneration, made
/// only from features before it, none of which (nor any feature that took
/// one of their bodies before it) found something else than the last time
/// ([`Step`], [`Changed`]), has the inputs it had: only the rest of its
/// context is compared. What is reused is exactly what the full
/// comparison reuses.
pub(crate) fn features(
    doc: &Document,
    variables: &Evaluation,
    cache: &mut RegenCache,
    need: Need,
    mut build: impl FnMut(
        &Feature,
        bool,
        &[FeatureResult],
        &mut RegenCache,
    ) -> (FeatureResult, Option<FeatureId>),
) -> Vec<FeatureResult> {
    cache.prune(doc);
    cache.rebuilt = 0;
    cache.reused = 0;
    cache.rederived = 0;
    let mut results: Vec<FeatureResult> = Vec::with_capacity(doc.features.len());
    // Where each feature's result is in `results`.
    let mut at: BTreeMap<FeatureId, usize> = BTreeMap::new();
    let mut lookup = Lookup::new(doc);
    let mut stamps: BTreeMap<FeatureId, u64> = BTreeMap::new();
    let mut rolled_back = false;
    let steps = doc.features.len();
    // The last regeneration's steps; left empty until this one is done,
    // so that one that does not finish leaves nothing to rely on.
    let last = std::mem::take(&mut cache.trail);
    // How many features stand where they stood the last time. With an id
    // twice in the history (a damaged file), what is looked up by id is
    // not each feature's own: then none.
    let same_places = if lookup.by_id.len() == doc.features.len() {
        last.iter()
            .zip(&doc.features)
            .take_while(|((id, _), feature)| *id == feature.id)
            .count()
    } else {
        0
    };
    let mut trail: Vec<(FeatureId, Step)> = Vec::with_capacity(steps);
    let mut changed = Changed::new(doc);
    // For each feature built so far: whether everything it is made from,
    // directly or through others, came before it; and whether that holds
    // of every feature so far.
    let mut before: BTreeMap<FeatureId, bool> = BTreeMap::new();
    let mut all_before = true;
    for (index, stated) in doc.features.iter().enumerate() {
        // Names of an earlier body are followed to the newest one before
        // this feature; it is then built, depends and takes as followed.
        let made_before = |f: &Feature| {
            f.kind
                .inputs()
                .iter()
                .all(|input| before.get(input).copied().unwrap_or(false))
                && (all_before || !f.kind.uses_all_earlier())
        };
        let stated_before = made_before(stated);
        // Switched off (suppressed, or its condition is 0): its names are
        // still followed, since it passes on the body they are of
        // (`crate::pass`); one that cannot be followed passes on what its
        // names name as given, or says why it cannot.
        let off = !rolled_back && matches!(crate::pass::switch(variables, stated), Ok(Some(_)));
        let followed = if rolled_back {
            Ok(None)
        } else if let Some(entry) = cache.entries.get(&stated.id).filter(|entry| {
            // Following names reads the results before this feature and
            // the names and kinds of the features it names and of those
            // that took their bodies, all of them earlier. When every
            // feature before it found what it found the last time (no
            // mark yet) and it stands where it stood, made only from
            // earlier features, the names follow as they did then.
            index < same_places
                && changed.marked.is_empty()
                && stated_before
                && !entry.context.rolled_back
                && entry.feature == *stated
        }) {
            entry.context.followed.clone()
        } else {
            crate::follow::feature(doc, &results, stated)
        };
        let followed = match followed {
            Err(_) if off => Ok(None),
            other => other,
        };
        let feature = match &followed {
            Ok(Some(followed)) => followed,
            Ok(None) | Err(_) => stated,
        };
        // Its condition with the rest: a change of what it evaluates to
        // switches the feature on or off.
        let values = evaluated(variables, &feature.exprs());
        let group = crate::split::group_state(doc, variables, stated);
        let unchanged = !changed.is_marked(stated.id)
            && feature
                .kind
                .inputs()
                .iter()
                .all(|input| !changed.is_marked(*input));
        let known_inputs = index < same_places && unchanged && made_before(feature);
        before.insert(stated.id, stated_before);
        all_before &= stated_before;
        let reused = cache
            .entries
            .get(&feature.id)
            .filter(|entry| {
                known_inputs
                    && entry.feature == *stated
                    && entry.context.rolled_back == rolled_back
                    && entry.context.units == doc.units
                    && entry.context.tolerance == bits(doc.tolerance)
                    && entry.context.values == values
                    && entry.context.followed == followed
                    && entry.context.group == group
            })
            .map(|entry| (entry.result.clone(), entry.stamp));
        let (result, stamp) = match reused {
            Some(hit) => {
                cache.reused += 1;
                hit
            }
            None => {
                let context = context(
                    &mut lookup,
                    &stamps,
                    &at,
                    &results,
                    (index, stated, feature),
                    Context {
                        rolled_back,
                        followed: followed.clone(),
                        values,
                        units: doc.units,
                        tolerance: bits(doc.tolerance),
                        inputs: Vec::new(),
                        group,
                    },
                );
                let hit = cache
                    .entries
                    .get(&feature.id)
                    .filter(|entry| entry.context == context && entry.feature == *stated)
                    .map(|entry| (entry.result.clone(), entry.stamp));
                match hit {
                    Some(hit) => {
                        cache.reused += 1;
                        hit
                    }
                    None => {
                        cache.rebuilt += 1;
                        crate::progress::report(&crate::progress::Progress {
                            phase: "regenerating",
                            feature: Some(feature.id.to_string()),
                            name: Some(feature.name.clone()),
                            step: index.saturating_add(1),
                            steps,
                        });
                        let result = match &followed {
                            Err(reason) => FeatureResult {
                                id: stated.id,
                                status: Status::Failed(reason.clone()),
                                body: None,
                                sketch: None,
                                datum: None,
                                names: None,
                                instances: None,
                                sweep: None,
                                thread: None,
                                offset: None,
                                consumed_by: None,
                                followed: None,
                                passed: None,
                            },
                            Ok(as_built) => {
                                rederive(doc, &mut results, &at, cache, &mut build, feature);
                                FeatureResult {
                                    followed: as_built.clone().map(Arc::new),
                                    ..build(feature, rolled_back, &results, cache).0
                                }
                            }
                        };
                        // The same definition giving the same result keeps
                        // its stamp: nothing made from it has to be rebuilt.
                        let stamp = match cache.entries.get(&feature.id) {
                            // An old body let go of cannot be compared:
                            // what is made from this feature is rebuilt.
                            Some(old)
                                if old.feature == *stated
                                    && !old.evicted
                                    && old.result == result =>
                            {
                                old.stamp
                            }
                            _ => {
                                cache.next_stamp = cache.next_stamp.wrapping_add(1);
                                cache.next_stamp
                            }
                        };
                        cache.entries.insert(
                            feature.id,
                            Entry {
                                feature: stated.clone(),
                                context,
                                bytes: result.body.as_deref().map_or(0, crate::body_bytes),
                                evicted: false,
                                result: result.clone(),
                                stamp,
                            },
                        );
                        (result, stamp)
                    }
                }
            }
        };
        // A feature made from another one's body takes that body, on a
        // reused result as on a rebuilt one.
        // One that passes a body on (`crate::pass`) takes that body only.
        let mut takes = Vec::new();
        let sources = match &result.passed {
            Some(crate::Passed::Body(source)) => vec![(*source, false)],
            Some(_) => Vec::new(),
            None if matches!(result.status, Status::Ok) => feature.kind.sources(),
            None => Vec::new(),
        };
        for (source, keep) in sources {
            if !keep && let Some(taken) = at.get(&source).and_then(|i| results.get_mut(*i)) {
                taken.consumed_by = Some(feature.id);
                takes.push(source);
            }
        }
        let step = Step {
            named: (
                lookup
                    .names
                    .get(&stated.id)
                    .cloned()
                    .unwrap_or_else(|| Arc::from(stated.name.as_str())),
                stated.kind.type_name(),
            ),
            stamp,
            takes,
        };
        if index < same_places
            && let Some((_, old)) = last.get(index)
            && *old != step
        {
            changed.mark(doc, &lookup, stated.id);
            for taken in old.takes.iter().chain(&step.takes) {
                changed.mark(doc, &lookup, *taken);
            }
        }
        trail.push((stated.id, step));
        stamps.insert(feature.id, stamp);
        at.entry(result.id).or_insert(results.len());
        results.push(result);
        if doc.rollback == Some(stated.id) {
            rolled_back = true;
        }
    }
    cache.trail = trail;
    cache.work = lookup.work;
    // A body let go of while a later feature had taken it is the
    // document's own again when that feature no longer takes it.
    let live: Vec<FeatureId> = results
        .iter()
        .filter(|r| r.consumed_by.is_none() && r.body.is_none())
        .filter(|r| cache.entries.get(&r.id).is_some_and(|entry| entry.evicted))
        .map(|r| r.id)
        .collect();
    rederive_all(doc, &mut results, &at, cache, &mut build, &live);
    match need {
        Need::Bodies => {}
        Need::All => {
            let all: Vec<FeatureId> = results.iter().map(|r| r.id).collect();
            rederive_all(doc, &mut results, &at, cache, &mut build, &all);
        }
        Need::These(ids) => rederive_all(doc, &mut results, &at, cache, &mut build, ids),
    }
    evict(cache, &mut results, need);
    crate::work::regeneration(cache.rebuilt, cache.reused, cache.work);
    results
}

/// Derives again, before `feature` is built, the bodies of the features
/// it is made from that were let go of, and of those they are made from,
/// in history order (what a feature is made from comes before it, or it
/// has no body to let go of): each is built from the results before it,
/// as it was, and gives the same bits.
fn rederive(
    doc: &Document,
    results: &mut [FeatureResult],
    at: &BTreeMap<FeatureId, usize>,
    cache: &mut RegenCache,
    build: &mut impl FnMut(
        &Feature,
        bool,
        &[FeatureResult],
        &mut RegenCache,
    ) -> (FeatureResult, Option<FeatureId>),
    feature: &Feature,
) {
    let inputs = feature.kind.inputs();
    rederive_all(doc, results, at, cache, build, &inputs);
}

/// [`rederive`] for the bodies of `ids` and what they are made from.
fn rederive_all(
    doc: &Document,
    results: &mut [FeatureResult],
    at: &BTreeMap<FeatureId, usize>,
    cache: &mut RegenCache,
    build: &mut impl FnMut(
        &Feature,
        bool,
        &[FeatureResult],
        &mut RegenCache,
    ) -> (FeatureResult, Option<FeatureId>),
    ids: &[FeatureId],
) {
    // The features let go of that `ids` are made from, directly or
    // through others that were let go of; each looked at once.
    let gone = |cache: &RegenCache, id: FeatureId| -> Option<Feature> {
        let entry = cache.entries.get(&id).filter(|entry| entry.evicted)?;
        let index = *at.get(&id)?;
        let result = results.get(index)?;
        if result.body.is_some() || !result.is_built() {
            return None;
        }
        Some(
            entry
                .result
                .followed
                .as_deref()
                .cloned()
                .unwrap_or_else(|| entry.feature.clone()),
        )
    };
    let mut needed: BTreeMap<usize, Feature> = BTreeMap::new();
    let mut open: Vec<FeatureId> = ids.to_vec();
    let mut seen: BTreeSet<FeatureId> = BTreeSet::new();
    while let Some(id) = open.pop() {
        if !seen.insert(id) {
            continue;
        }
        let Some(feature) = gone(cache, id) else {
            continue;
        };
        open.extend(feature.kind.inputs());
        if let Some(index) = at.get(&id) {
            needed.insert(*index, feature);
        }
    }
    for (index, feature) in needed {
        let (earlier, rest) = results.split_at_mut(index);
        let Some(result) = rest.first_mut() else {
            continue;
        };
        crate::progress::report(&crate::progress::Progress {
            phase: "deriving again",
            feature: Some(feature.id.to_string()),
            name: Some(feature.name.clone()),
            step: index.saturating_add(1),
            steps: doc.features.len(),
        });
        // Built as it was the first time: nothing that this feature or a
        // later one takes (its own sources, and a source it keeps that a
        // later feature took) was taken yet.
        let taken: Vec<(usize, FeatureId)> = earlier
            .iter()
            .enumerate()
            .filter_map(|(i, r)| {
                let taker = r.consumed_by?;
                (*at.get(&taker)? >= index).then_some((i, taker))
            })
            .collect();
        for (i, _) in &taken {
            if let Some(source) = earlier.get_mut(*i) {
                source.consumed_by = None;
            }
        }
        let made = build(&feature, false, earlier, cache).0;
        for (i, taker) in &taken {
            if let Some(source) = earlier.get_mut(*i) {
                source.consumed_by = Some(*taker);
            }
        }
        cache.rederived = cache.rederived.saturating_add(1);
        let Some(body) = made.body else {
            // Never silently: the result says so, differs from a full
            // regeneration (the oracle finds it), and the entry stays as
            // it is, to be derived again next time.
            let why = match &made.status {
                Status::Failed(reason) => reason.clone(),
                other => format!("{other:?} and no body"),
            };
            result.status = Status::Failed(format!(
                "the body of `{}` could not be derived again from what it is made from ({why}); this is a bug of the regeneration cache",
                feature.name
            ));
            continue;
        };
        result.body = Some(Arc::clone(&body));
        result.names.clone_from(&made.names);
        result.instances.clone_from(&made.instances);
        if let Some(entry) = cache.entries.get_mut(&feature.id) {
            entry.result.body = Some(body);
            entry.result.names = made.names;
            entry.result.instances = made.instances;
            entry.evicted = false;
        }
    }
}

/// Lets go of the bodies that went into later features beyond the budget,
/// in the cache and in `results` alike; those `need` asks for are kept.
/// The bodies the latest features took are kept first: an edit of a
/// feature near the end of the history then finds what it is made from
/// (the target and the tool of the last union, wherever in the history
/// the tool was made), and what the earliest features took goes first.
fn evict(cache: &mut RegenCache, results: &mut [FeatureResult], need: Need) {
    if need == Need::All {
        return;
    }
    let budget = cache.body_budget();
    let at: BTreeMap<FeatureId, usize> = results
        .iter()
        .enumerate()
        .map(|(index, r)| (r.id, index))
        .collect();
    // (where the taker stands, where the body stands), latest first.
    let mut taken: Vec<(usize, usize)> = results
        .iter()
        .enumerate()
        .filter(|(_, r)| r.body.is_some())
        .filter_map(|(index, r)| Some((*at.get(&r.consumed_by?)?, index)))
        .collect();
    taken.sort_unstable_by(|a, b| b.cmp(a));
    let mut held = 0usize;
    for (_, index) in taken {
        let Some(result) = results.get_mut(index) else {
            continue;
        };
        let Some(entry) = cache.entries.get_mut(&result.id) else {
            continue;
        };
        held = held.saturating_add(entry.bytes);
        if held > budget && !need.includes(result.id) {
            entry.result.body = None;
            entry.evicted = true;
            result.body = None;
            // The names of its edges and vertices go with it (derived
            // from the body and the faces' names); the faces' names
            // stay, so that a name can be followed through it.
            if let Some(names) = result.names.as_deref().filter(|names| names.has_entities()) {
                let kept = Arc::new(names.without_entities());
                result.names = Some(Arc::clone(&kept));
                entry.result.names = Some(kept);
            }
        }
    }
}

/// The full context of the feature at `index` of the history (`stated`,
/// built as `feature`): `partial` with the inputs filled in.
fn context(
    lookup: &mut Lookup,
    stamps: &BTreeMap<FeatureId, u64>,
    at: &BTreeMap<FeatureId, usize>,
    results: &[FeatureResult],
    (index, stated, feature): (usize, &Feature, &Feature),
    partial: Context,
) -> Context {
    let made_from = lookup.dependencies(feature);
    // Remembered for the features made from this one, when it is the
    // feature of the history with this id, as stated.
    if std::ptr::eq(feature, stated) && lookup.by_id.get(&stated.id).map(|(i, _)| *i) == Some(index)
    {
        lookup
            .made_from
            .entry(stated.id)
            .or_insert_with(|| made_from.clone().into());
    }
    let mut inputs = Vec::with_capacity(made_from.len());
    for id in made_from {
        let circle = !stamps.contains_key(&id)
            && lookup.feature(id).is_some()
            && lookup.made_from(id).contains(&feature.id);
        let name = |id: FeatureId| lookup.names.get(&id).cloned();
        inputs.push(Input {
            id,
            named: lookup
                .feature(id)
                .and_then(|f| Some((name(id)?, f.kind.type_name()))),
            stamp: stamps.get(&id).copied(),
            taken_by: at
                .get(&id)
                .and_then(|i| results.get(*i))
                .and_then(|r| r.consumed_by)
                .map(|taker| (taker, name(taker).unwrap_or_else(|| Arc::from("")))),
            circle,
        });
    }
    Context { inputs, ..partial }
}
