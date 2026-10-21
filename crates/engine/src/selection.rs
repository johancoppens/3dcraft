//! The selection: what the person in the window and an agent point at
//! together (docs/decisions.md, 2026-10-21, B1). An ordered list of
//! names, each with its kind: bodies, features, sketches and datums by
//! their feature, faces, edges and vertices by their persistent names.
//!
//! It belongs to the open document as the window holds it, not to the
//! document itself: it is not saved, not part of the history and not
//! undone. A name stays valid through regeneration as long as what it
//! names exists; an item whose entity is gone drops out at the next look
//! ([`Session::selection`]), and the dropped names are said
//! ([`SelectionView::dropped`]).

use crate::Session;
use craft3d_doc::{Document, FeatureId, FeatureKind, PersistentName};
use craft3d_regen::{Names, Regen};
use craft3d_topo::Body;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Most items a selection holds.
pub const MAX_SELECTION: usize = 10_000;
/// Most dropped names kept to report.
const MAX_DROPPED: usize = 50;
/// Longest name taken, in bytes: a persistent name grows with every copy
/// and boolean, but not without end.
pub const MAX_NAME_BYTES: usize = 8192;

/// What an item of a selection is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SelectionKind {
    Body,
    Face,
    Edge,
    Vertex,
    Feature,
    Sketch,
    Datum,
}

impl SelectionKind {
    /// Every kind, in the order a summary counts them.
    pub const ALL: [SelectionKind; 7] = [
        SelectionKind::Body,
        SelectionKind::Face,
        SelectionKind::Edge,
        SelectionKind::Vertex,
        SelectionKind::Feature,
        SelectionKind::Sketch,
        SelectionKind::Datum,
    ];

    /// The word the commands use.
    pub fn name(self) -> &'static str {
        match self {
            SelectionKind::Body => "body",
            SelectionKind::Face => "face",
            SelectionKind::Edge => "edge",
            SelectionKind::Vertex => "vertex",
            SelectionKind::Feature => "feature",
            SelectionKind::Sketch => "sketch",
            SelectionKind::Datum => "datum",
        }
    }

    fn plural(self) -> &'static str {
        match self {
            SelectionKind::Body => "bodies",
            SelectionKind::Face => "faces",
            SelectionKind::Edge => "edges",
            SelectionKind::Vertex => "vertices",
            SelectionKind::Feature => "features",
            SelectionKind::Sketch => "sketches",
            SelectionKind::Datum => "datums",
        }
    }

    /// The kind a word names, singular or plural.
    pub fn parse(word: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|kind| kind.name() == word || kind.plural() == word)
    }

    /// A face, an edge or a vertex: named by a persistent name within a
    /// body, rather than by a feature.
    pub fn is_entity(self) -> bool {
        matches!(
            self,
            SelectionKind::Face | SelectionKind::Edge | SelectionKind::Vertex
        )
    }

    /// "3 bodies", "1 face".
    pub fn count(self, n: usize) -> String {
        if n == 1 {
            format!("1 {}", self.name())
        } else {
            format!("{n} {}", self.plural())
        }
    }
}

/// One item of a selection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectionItem {
    pub kind: SelectionKind,
    /// A body, feature, sketch or datum: the feature's name. A face, edge
    /// or vertex: its persistent name, as `document.inspect` and
    /// `document.edges` list it.
    pub name: String,
    /// The feature, by id (`F3`): the feature itself, or for a face, an
    /// edge or a vertex the feature whose body has it.
    pub feature: String,
}

impl SelectionItem {
    /// What makes two items the same: a feature is itself whatever it is
    /// called now.
    fn key(&self) -> (SelectionKind, &str, &str) {
        let name = if self.kind.is_entity() {
            self.name.as_str()
        } else {
            ""
        };
        (self.kind, self.feature.as_str(), name)
    }

    pub fn same(&self, other: &SelectionItem) -> bool {
        self.key() == other.key()
    }

    /// As the commands give it.
    pub fn json(&self) -> Value {
        json!({ "kind": self.kind.name(), "name": self.name, "feature": self.feature })
    }
}

/// A one-line count of `items` by kind: "3 bodies, 2 faces"; "nothing"
/// for none.
pub fn summary(items: &[SelectionItem]) -> String {
    let mut counts: BTreeMap<SelectionKind, usize> = BTreeMap::new();
    for item in items {
        *counts.entry(item.kind).or_default() += 1;
    }
    let parts: Vec<String> = SelectionKind::ALL
        .into_iter()
        .filter_map(|kind| counts.get(&kind).map(|&n| kind.count(n)))
        .collect();
    if parts.is_empty() {
        "nothing".to_string()
    } else {
        parts.join(", ")
    }
}

/// A serial no two selections share, across documents: the shell redraws
/// when it moves.
fn next_serial() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// The selection of one open document.
#[derive(Clone, Debug, Default)]
pub(crate) struct Selection {
    items: Vec<SelectionItem>,
    serial: u64,
    /// The names that dropped out at the last check, and why.
    dropped: Vec<String>,
    /// The revision of the document the items were last checked against.
    checked: Option<u64>,
}

/// The selection as it is now ([`Session::selection`]).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct SelectionView {
    pub items: Vec<SelectionItem>,
    /// Moves whenever the items change.
    pub serial: u64,
    /// What dropped out because it no longer exists, since the selection
    /// was last set: one line each.
    pub dropped: Vec<String>,
}

impl SelectionView {
    /// As `select.get` and `ui.get` give it.
    pub fn json(&self) -> Value {
        json!({
            "items": self.items.iter().map(SelectionItem::json).collect::<Vec<_>>(),
            "count": self.items.len(),
            "summary": summary(&self.items),
            "dropped": self.dropped,
        })
    }
}

/// How a list of items changes the selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectMode {
    Replace,
    Add,
    Remove,
    /// Each item: removed when it is selected, added when not (a click
    /// with Shift).
    Toggle,
    Clear,
}

impl SelectMode {
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "replace" => Some(SelectMode::Replace),
            "add" => Some(SelectMode::Add),
            "remove" => Some(SelectMode::Remove),
            "toggle" => Some(SelectMode::Toggle),
            "clear" => Some(SelectMode::Clear),
            _ => None,
        }
    }
}

impl Selection {
    /// Changes the items by `mode`; the serial moves when they changed.
    /// Returns how many were added and how many removed.
    pub(crate) fn apply(&mut self, items: Vec<SelectionItem>, mode: SelectMode) -> (usize, usize) {
        let before = self.items.clone();
        match mode {
            SelectMode::Clear => self.items.clear(),
            SelectMode::Replace => {
                self.items.clear();
                self.add(items);
            }
            SelectMode::Add => self.add(items),
            SelectMode::Remove => self
                .items
                .retain(|kept| !items.iter().any(|i| i.same(kept))),
            SelectMode::Toggle => {
                for item in items {
                    match self.items.iter().position(|kept| kept.same(&item)) {
                        Some(at) => {
                            self.items.remove(at);
                        }
                        None => self.add(vec![item]),
                    }
                }
            }
        }
        let added = self
            .items
            .iter()
            .filter(|i| !before.iter().any(|b| b.same(i)))
            .count();
        let removed = before
            .iter()
            .filter(|b| !self.items.iter().any(|i| i.same(b)))
            .count();
        // A new choice: what dropped out of the old one is old news.
        if self.items != before || !self.dropped.is_empty() {
            self.serial = next_serial();
        }
        self.dropped.clear();
        (added, removed)
    }

    fn add(&mut self, items: Vec<SelectionItem>) {
        for item in items {
            if self.items.len() >= MAX_SELECTION {
                break;
            }
            if !self.items.iter().any(|kept| kept.same(&item)) {
                self.items.push(item);
            }
        }
    }

    /// Checks every item against `regen` of revision `revision` (once per
    /// revision): what no longer exists drops out and is said in
    /// `dropped`, which holds what dropped out at the last change of the
    /// document (an undo that follows clears it); a renamed feature's
    /// item takes its new name. Items are found again by their feature's
    /// id, never by its name, so a selection cannot move to another
    /// feature that took the name (rule 4).
    pub(crate) fn check(&mut self, doc: &Document, regen: &Regen, revision: u64) {
        if self.checked == Some(revision) {
            return;
        }
        self.checked = Some(revision);
        let mut kept = Vec::with_capacity(self.items.len());
        let mut dropped = Vec::new();
        if !self.items.is_empty() {
            let mut resolver = Resolver::new(doc, regen);
            for item in &self.items {
                match resolver.find_again(item) {
                    Ok(found) => kept.push(found.item),
                    Err(why) => {
                        dropped.push(format!("{} {}: {why}", item.kind.name(), item.name));
                    }
                }
            }
        }
        dropped.truncate(MAX_DROPPED);
        if kept != self.items || dropped != self.dropped {
            self.items = kept;
            self.dropped = dropped;
            self.serial = next_serial();
        }
    }

    pub(crate) fn view(&self) -> SelectionView {
        SelectionView {
            items: self.items.clone(),
            serial: self.serial,
            dropped: self.dropped.clone(),
        }
    }

    pub(crate) fn serial(&self) -> u64 {
        self.serial
    }
}

/// An item as it was found: with its body and its place there.
#[derive(Clone, Debug)]
pub(crate) struct Found {
    pub item: SelectionItem,
}

/// The feature a reference means: its name, or its id (`F3`, `3`).
///
/// Text written as an id (`F3`, or `3`) means that id and nothing else:
/// an older file may still have a feature *named* `F1`, and a selection
/// stored by id must never move to it (rule 4).
pub(crate) fn feature_named(doc: &Document, text: &str) -> Option<FeatureId> {
    if craft3d_doc::looks_like_id(text)
        || (!text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()))
    {
        let digits = text.strip_prefix('F').unwrap_or(text);
        let id = FeatureId(digits.parse::<u64>().ok()?);
        return doc.feature(id).map(|f| f.id);
    }
    doc.feature_by_name(text).map(|feature| feature.id)
}

/// Whether a feature is reference geometry.
pub(crate) fn is_datum(kind: &FeatureKind) -> bool {
    matches!(
        kind,
        FeatureKind::DatumPlane(_)
            | FeatureKind::DatumAxis(_)
            | FeatureKind::DatumPoint(_)
            | FeatureKind::DatumHelix(_)
    )
}

/// The names of one body's faces, edges and vertices, looked up by text.
#[derive(Default)]
struct BodyIndex {
    faces: Option<BTreeMap<String, String>>,
    edges: Option<BTreeMap<String, String>>,
    vertices: Option<BTreeMap<String, String>>,
}

/// A body of a regeneration: its feature, the body and its names.
type BodyNames<'a> = (FeatureId, &'a Arc<Body>, Option<&'a Arc<Names>>);

/// Finds what names mean in one regeneration of a document.
pub(crate) struct Resolver<'a> {
    doc: &'a Document,
    regen: &'a Regen,
    /// The bodies, in history order.
    bodies: Vec<BodyNames<'a>>,
    index: BTreeMap<FeatureId, BodyIndex>,
}

impl<'a> Resolver<'a> {
    pub(crate) fn new(doc: &'a Document, regen: &'a Regen) -> Self {
        let bodies = regen
            .features
            .iter()
            .filter_map(|f| Some((f.id, f.own_body()?, f.names.as_ref())))
            .collect();
        Self {
            doc,
            regen,
            bodies,
            index: BTreeMap::new(),
        }
    }

    /// The canonical name `wanted` has among the entities of `kind` of the
    /// body of `feature`: a face by its name or an alias, an edge by its
    /// name or an alias, a vertex by its name.
    fn lookup(&mut self, feature: FeatureId, kind: SelectionKind, wanted: &str) -> Option<String> {
        let (_, body, names) = self.bodies.iter().find(|(id, ..)| *id == feature)?;
        let names = (*names)?;
        let entry = self.index.entry(feature).or_default();
        let table = match kind {
            SelectionKind::Face => entry.faces.get_or_insert_with(|| {
                let mut table = BTreeMap::new();
                for (k, name) in names.faces.iter().enumerate() {
                    let canonical = name.to_string();
                    for alias in names.aliases_of(k) {
                        table
                            .entry(alias.to_string())
                            .or_insert_with(|| canonical.clone());
                    }
                    table.insert(canonical.clone(), canonical);
                }
                table
            }),
            SelectionKind::Edge => entry.edges.get_or_insert_with(|| {
                let mut table = BTreeMap::new();
                for k in 0..body.edges.len() {
                    if let Some(name) = names.edge_name(k) {
                        let canonical = name.to_string();
                        for alias in names.edge_aliases_of(k) {
                            table
                                .entry(alias.to_string())
                                .or_insert_with(|| canonical.clone());
                        }
                        table.insert(canonical.clone(), canonical);
                    }
                }
                table
            }),
            SelectionKind::Vertex => entry.vertices.get_or_insert_with(|| {
                let mut table = BTreeMap::new();
                for k in 0..body.vertices.len() {
                    if let Some(name) = names.vertex_name(k) {
                        let canonical = name.to_string();
                        table.insert(canonical.clone(), canonical);
                    }
                }
                table
            }),
            _ => return None,
        };
        table.get(wanted).cloned()
    }

    /// What `name` means: `kind` when given (otherwise read from the name:
    /// a feature's name or id, or a persistent name of a face, an edge or
    /// a vertex), within the body of `feature` when given. The error is a
    /// sentence for whoever asked.
    pub(crate) fn resolve(
        &mut self,
        kind: Option<SelectionKind>,
        name: &str,
        feature: Option<&str>,
    ) -> Result<Found, String> {
        if name.len() > MAX_NAME_BYTES {
            return Err(format!("a name is at most {MAX_NAME_BYTES} bytes"));
        }
        let shown: String = name.chars().take(80).collect();
        let entity = kind.is_none_or(SelectionKind::is_entity);
        if !entity || kind.is_none() {
            // A feature, unless the name reads as a persistent name.
            let as_feature = if name.is_empty() {
                feature.and_then(|f| feature_named(self.doc, f))
            } else {
                feature_named(self.doc, name)
            };
            if let Some(id) = as_feature {
                return self.feature_item(kind, id);
            }
            if !entity {
                return Err(format!("there is no feature `{shown}`"));
            }
        }
        let parsed: PersistentName = name
            .parse()
            .map_err(|e| format!("`{shown}` is neither a feature nor a persistent name ({e})"))?;
        let kind = match kind {
            Some(kind) => kind,
            None if parsed.is_face() => SelectionKind::Face,
            None if parsed.is_edge() => SelectionKind::Edge,
            None if parsed.is_vertex() => SelectionKind::Vertex,
            None => return Err(format!("`{shown}` names no face, edge or vertex")),
        };
        let wanted = parsed.to_string();
        let only = match feature {
            Some(text) => Some(
                feature_named(self.doc, text)
                    .ok_or_else(|| format!("there is no feature `{}`", cut(text)))?,
            ),
            None => None,
        };
        let candidates: Vec<FeatureId> = self
            .bodies
            .iter()
            .map(|(id, ..)| *id)
            .filter(|id| only.is_none_or(|only| only == *id))
            .collect();
        if let Some(only) = only
            && candidates.is_empty()
        {
            let made = self
                .doc
                .feature(only)
                .map_or_else(|| only.to_string(), |f| f.name.clone());
            return Err(format!("`{made}` has no body"));
        }
        for body in candidates {
            if let Some(canonical) = self.lookup(body, kind, &wanted) {
                return Ok(Found {
                    item: SelectionItem {
                        kind,
                        name: canonical,
                        feature: body.to_string(),
                    },
                });
            }
        }
        Err(format!("no body has a {} `{shown}`", kind.name()))
    }

    /// A stored item found again: its feature by id only (`F3`), a face,
    /// an edge or a vertex by its name in that feature's body.
    pub(crate) fn find_again(&mut self, item: &SelectionItem) -> Result<Found, String> {
        let id = item
            .feature
            .strip_prefix('F')
            .and_then(|digits| digits.parse::<u64>().ok())
            .map(FeatureId)
            .filter(|id| self.doc.feature(*id).is_some())
            .ok_or_else(|| format!("there is no feature {}", cut(&item.feature)))?;
        if !item.kind.is_entity() {
            return self.feature_item(Some(item.kind), id);
        }
        if !self.bodies.iter().any(|(body, ..)| *body == id) {
            let made = self
                .doc
                .feature(id)
                .map_or_else(|| id.to_string(), |f| f.name.clone());
            return Err(match self.regen.feature(id).and_then(|o| o.consumed_by) {
                Some(taker) => format!("the body of `{made}` went into {taker}"),
                None => format!("`{made}` has no body"),
            });
        }
        let wanted = item
            .name
            .parse::<PersistentName>()
            .map(|name| name.to_string())
            .unwrap_or_else(|_| item.name.clone());
        match self.lookup(id, item.kind, &wanted) {
            Some(canonical) => Ok(Found {
                item: SelectionItem {
                    kind: item.kind,
                    name: canonical,
                    feature: id.to_string(),
                },
            }),
            None => Err(format!(
                "the body of {id} has no {} of that name any more",
                item.kind.name()
            )),
        }
    }

    fn feature_item(&self, kind: Option<SelectionKind>, id: FeatureId) -> Result<Found, String> {
        let Some(feature) = self.doc.feature(id) else {
            return Err(format!("there is no feature {id}"));
        };
        let outcome = self.regen.feature(id);
        let has_body = outcome.and_then(|o| o.own_body()).is_some();
        let sketch = matches!(feature.kind, FeatureKind::Sketch(_));
        let datum = is_datum(&feature.kind);
        let kind = match kind {
            Some(SelectionKind::Body) if !has_body => {
                return Err(match outcome.and_then(|o| o.consumed_by) {
                    Some(taker) => format!("the body of `{}` went into {taker}", feature.name),
                    None => format!("`{}` has no body", feature.name),
                });
            }
            Some(SelectionKind::Sketch) if !sketch => {
                return Err(format!("`{}` is not a sketch", feature.name));
            }
            Some(SelectionKind::Datum) if !datum => {
                return Err(format!("`{}` is not a datum", feature.name));
            }
            Some(kind) if kind.is_entity() => {
                return Err(format!(
                    "`{}` is a feature, not a {}",
                    feature.name,
                    kind.name()
                ));
            }
            Some(kind) => kind,
            None if has_body => SelectionKind::Body,
            None if sketch => SelectionKind::Sketch,
            None if datum => SelectionKind::Datum,
            None => SelectionKind::Feature,
        };
        Ok(Found {
            item: SelectionItem {
                kind,
                name: feature.name.clone(),
                feature: id.to_string(),
            },
        })
    }
}

fn cut(text: &str) -> String {
    text.chars().take(80).collect()
}

impl Session {
    /// The active document's selection, checked against its current
    /// regeneration first: what no longer exists has dropped out. Empty
    /// without a document.
    pub fn selection(&mut self) -> SelectionView {
        self.selection_serial();
        self.document()
            .map_or_else(|_| SelectionView::default(), |open| open.selection.view())
    }

    /// The serial of the active document's selection, checked against
    /// its current regeneration first: it moves whenever the items or
    /// what dropped out change, so a frontend copies the selection only
    /// then. 0 without a document.
    pub fn selection_serial(&mut self) -> u64 {
        let Ok(open) = self.document_mut() else {
            return 0;
        };
        let revision = open.history.revision();
        if open.selection.checked != Some(revision) {
            let regen = open.regen();
            open.selection
                .check(open.history.current(), &regen, revision);
        }
        open.selection.serial()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;
    use crate::Workspace;
    use serde_json::json;

    /// An older file may have a feature named `F1` (new names may not be
    /// written like an id): a selection stored by id stays on its own
    /// feature, and `F1` means the id.
    #[test]
    fn a_feature_named_like_another_id_never_takes_its_selection() {
        let mut s = Session::new(Workspace::none());
        s.execute("file.new", &json!({})).unwrap();
        for x in [0, 3] {
            s.execute(
                "feature.block",
                &json!({"size": [1, 1, 1], "position": [x, 0, 0]}),
            )
            .unwrap();
        }
        let mut doc: Document = (**s.document().unwrap().history.current()).clone();
        doc.features[0].name = "Plate".into();
        doc.features[1].name = "F1".into();
        let regen = craft3d_regen::regenerate(&doc);
        let mut selection = Selection::default();
        selection.apply(
            vec![SelectionItem {
                kind: SelectionKind::Body,
                name: "Block1".into(),
                feature: "F1".into(),
            }],
            SelectMode::Replace,
        );
        selection.check(&doc, &regen, 7);
        let view = selection.view();
        assert_eq!(view.items[0].feature, "F1");
        assert_eq!(view.items[0].name, "Plate");
        assert!(view.dropped.is_empty());
        assert_eq!(feature_named(&doc, "F1"), Some(FeatureId(1)));
        assert_eq!(feature_named(&doc, "Plate"), Some(FeatureId(1)));
    }
}
