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
use craft3d_doc::{Document, Feature, FeatureId, Sketch, Tolerance, Units};
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
            craft3d_expr::evaluate(expr.source(), |variable| variables.value(variable))
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
}

#[derive(Clone)]
struct Entry {
    feature: Feature,
    context: Context,
    /// As built: before a later feature took the body.
    result: FeatureResult,
    stamp: u64,
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
    /// Features rebuilt and reused by the last regeneration.
    rebuilt: usize,
    reused: usize,
}

impl RegenCache {
    /// How many features the last regeneration rebuilt and how many it
    /// reused.
    pub fn last_run(&self) -> (usize, usize) {
        (self.rebuilt, self.reused)
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
        }
    }

    fn feature(&self, id: FeatureId) -> Option<&'a Feature> {
        self.by_id.get(&id).map(|(_, f)| *f)
    }

    /// What a feature names, and for a feature that looks at every body
    /// built so far (an extrusion through all) every feature before it in
    /// the history.
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
        inputs
    }

    /// [`dependencies`] by a walk of the graph, as it always was.
    fn walked(&self, feature: &Feature) -> Vec<FeatureId> {
        let mut seen = BTreeSet::new();
        let mut open = self.inputs(feature);
        while let Some(id) = open.pop() {
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
        all.sort();
        all.dedup();
        all.retain(|id| *id != feature.id);
        all
    }
}

/// Runs `build` for every feature of the document that cannot reuse what
/// it gave the last time. `build` gets the feature, whether it is rolled
/// back, and the results so far; it returns the feature's result and the
/// earlier feature whose body it took, if any.
pub(crate) fn features(
    doc: &Document,
    variables: &Evaluation,
    cache: &mut RegenCache,
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
    let mut results: Vec<FeatureResult> = Vec::with_capacity(doc.features.len());
    // Where each feature's result is in `results`.
    let mut at: BTreeMap<FeatureId, usize> = BTreeMap::new();
    let mut lookup = Lookup::new(doc);
    let mut stamps: BTreeMap<FeatureId, u64> = BTreeMap::new();
    let mut rolled_back = false;
    let steps = doc.features.len();
    for (index, stated) in doc.features.iter().enumerate() {
        // Names of an earlier body are followed to the newest one before
        // this feature; it is then built, depends and takes as followed.
        let followed = if rolled_back || stated.suppressed {
            Ok(None)
        } else {
            crate::follow::feature(doc, &results, stated)
        };
        let feature = match &followed {
            Ok(Some(followed)) => followed,
            Ok(None) | Err(_) => stated,
        };
        let made_from = lookup.dependencies(feature);
        // Remembered for the features made from this one, when it is the
        // feature of the history with this id, as stated.
        if !matches!(followed, Ok(Some(_)))
            && lookup.by_id.get(&stated.id).map(|(i, _)| *i) == Some(index)
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
        let context = Context {
            rolled_back,
            followed: followed.clone(),
            values: evaluated(variables, &feature.kind.exprs()),
            units: doc.units,
            tolerance: bits(doc.tolerance),
            inputs,
        };
        let hit = cache
            .entries
            .get(&feature.id)
            .filter(|entry| entry.context == context && entry.feature == *stated)
            .map(|entry| (entry.result.clone(), entry.stamp));
        let (result, stamp) = match hit {
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
                        consumed_by: None,
                        followed: None,
                    },
                    Ok(as_built) => FeatureResult {
                        followed: as_built.clone().map(Arc::new),
                        ..build(feature, rolled_back, &results, cache).0
                    },
                };
                // The same definition giving the same result keeps its
                // stamp: nothing made from it has to be rebuilt.
                let stamp = match cache.entries.get(&feature.id) {
                    Some(old) if old.feature == *stated && old.result == result => old.stamp,
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
                        result: result.clone(),
                        stamp,
                    },
                );
                (result, stamp)
            }
        };
        // A feature made from another one's body takes that body, on a
        // reused result as on a rebuilt one.
        if matches!(result.status, Status::Ok) {
            for (source, keep) in feature.kind.sources() {
                if !keep && let Some(taken) = at.get(&source).and_then(|i| results.get_mut(*i)) {
                    taken.consumed_by = Some(feature.id);
                }
            }
        }
        stamps.insert(feature.id, stamp);
        at.entry(result.id).or_insert(results.len());
        results.push(result);
        if doc.rollback == Some(stated.id) {
            rolled_back = true;
        }
    }
    results
}
