//! The selection: what the person in the window and an agent point at
//! together (docs/decisions.md, 2026-10-21, B1). An ordered list of
//! names, each with its kind: bodies, features, sketches and datums by
//! their feature, faces, edges and vertices by their persistent names.
//!
//! It belongs to the open document as the window holds it, not to the
//! document itself: it is not saved, not part of the history and not
//! undone. Each item is kept as it was selected and found again from
//! there at every change of the document: an entity whose body a later
//! feature took is followed to its image in the newest body, an item of a
//! feature that is suppressed, rolled back or failed is kept hidden
//! ([`SelectionView::hidden`]) until the feature has a body again, and an
//! item whose entity is gone drops out at the next look
//! ([`Session::selection`]), the dropped names said
//! ([`SelectionView::dropped`]).

use crate::Session;
use craft3d_doc::{Document, FeatureId, FeatureKind, PersistentName};
use craft3d_regen::{Names, Regen, Status};
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

/// One item of a selection as it is kept: as it was selected (its seed),
/// and what it is now.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Entry {
    /// The item as it was selected: what it is found again from at every
    /// change of the document, so that it follows its entity into later
    /// features and comes back to it when they go.
    seed: SelectionItem,
    /// What it is now, each with how many copies the features that took
    /// it made of it (it is the first of them): one item, or every piece
    /// of a body split in pieces; none while it is hidden.
    now: Vec<(SelectionItem, u64)>,
    /// Why it is hidden: its feature is suppressed, rolled back or failed,
    /// so it has no body now but will again.
    hidden: Option<String>,
}

impl Entry {
    fn new(item: SelectionItem) -> Self {
        Self {
            seed: item.clone(),
            now: vec![(item, 1)],
            hidden: None,
        }
    }

    /// Whether `item` means this entry: what it is now, or what it was
    /// selected as.
    fn means(&self, item: &SelectionItem) -> bool {
        self.now.iter().any(|(now, _)| now.same(item)) || self.seed.same(item)
    }
}

/// The selection of one open document.
#[derive(Clone, Debug, Default)]
pub(crate) struct Selection {
    entries: Vec<Entry>,
    serial: u64,
    /// The names that dropped out at the last check, and why.
    dropped: Vec<String>,
    /// The revision of the document the items were last checked against.
    checked: Option<u64>,
}

/// An item that is selected but hidden for now ([`SelectionView::hidden`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HiddenItem {
    /// The item as it was selected.
    pub item: SelectionItem,
    /// Why it is hidden: "`Boss` is suppressed".
    pub why: String,
}

/// An item that was followed from what was selected into a later feature
/// ([`SelectionView::followed`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FollowedItem {
    /// What it is now.
    pub item: SelectionItem,
    /// What was selected.
    pub from: SelectionItem,
    /// How many copies the features it went through made of what was
    /// selected (a pattern: its instances); the item is the first. 1 when
    /// it was not copied.
    pub copies: u64,
    /// How many items it is now: more than 1 when a split cut its body in
    /// pieces that each kept some of it.
    pub pieces: usize,
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
    /// What is selected but has no body for now (its feature is
    /// suppressed, rolled back or failed): it comes back, in its place,
    /// when the feature does.
    pub hidden: Vec<HiddenItem>,
    /// The items that are not what was selected but what it became in a
    /// later feature, with what was selected: so a selection that went
    /// into a pattern (the first instance of several) or into the pieces
    /// of a split is said, never narrowed silently.
    pub followed: Vec<FollowedItem>,
}

impl SelectionView {
    /// As `select.get` and `ui.get` give it.
    pub fn json(&self) -> Value {
        json!({
            "items": self.items.iter().map(SelectionItem::json).collect::<Vec<_>>(),
            "count": self.items.len(),
            "summary": summary(&self.items),
            "dropped": self.dropped,
            "hidden": self.hidden.iter().map(|h| {
                let mut entry = h.item.json();
                if let Some(map) = entry.as_object_mut() {
                    map.insert("why".into(), h.why.clone().into());
                }
                entry
            }).collect::<Vec<_>>(),
            "followed": self.followed.iter().map(|f| {
                let mut entry = f.item.json();
                if let Some(map) = entry.as_object_mut() {
                    map.insert("from".into(), f.from.json());
                    map.insert("copies".into(), f.copies.into());
                    map.insert("pieces".into(), f.pieces.into());
                }
                entry
            }).collect::<Vec<_>>(),
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
    /// Returns how many were added and how many removed. An item that
    /// means an entry already kept (what it is now, or what it was
    /// selected as) is that entry: it keeps its seed.
    pub(crate) fn apply(&mut self, items: Vec<SelectionItem>, mode: SelectMode) -> (usize, usize) {
        let before = self.entries.clone();
        match mode {
            SelectMode::Clear => self.entries.clear(),
            SelectMode::Replace => {
                let old = std::mem::take(&mut self.entries);
                for item in items {
                    if self.entries.len() >= MAX_SELECTION {
                        break;
                    }
                    if self.entries.iter().any(|kept| kept.means(&item)) {
                        continue;
                    }
                    let entry = old
                        .iter()
                        .find(|kept| kept.means(&item))
                        .cloned()
                        .unwrap_or_else(|| Entry::new(item));
                    self.entries.push(entry);
                }
            }
            SelectMode::Add => self.add(items),
            SelectMode::Remove => self
                .entries
                .retain(|kept| !items.iter().any(|i| kept.means(i))),
            SelectMode::Toggle => {
                for item in items {
                    match self.entries.iter().position(|kept| kept.means(&item)) {
                        Some(at) => {
                            self.entries.remove(at);
                        }
                        None => self.add(vec![item]),
                    }
                }
            }
        }
        let added = self
            .entries
            .iter()
            .filter(|e| !before.iter().any(|b| b.seed.same(&e.seed)))
            .count();
        let removed = before
            .iter()
            .filter(|b| !self.entries.iter().any(|e| e.seed.same(&b.seed)))
            .count();
        // A new choice: what dropped out of the old one is old news.
        if self.entries != before || !self.dropped.is_empty() {
            self.serial = next_serial();
        }
        self.dropped.clear();
        (added, removed)
    }

    fn add(&mut self, items: Vec<SelectionItem>) {
        for item in items {
            if self.entries.len() >= MAX_SELECTION {
                break;
            }
            if !self.entries.iter().any(|kept| kept.means(&item)) {
                self.entries.push(Entry::new(item));
            }
        }
    }

    /// Checks every item against `regen` of revision `revision` (once per
    /// revision), from what it was selected as ([`Resolver::follow`]): an
    /// entity whose body went into a later feature is followed to its
    /// image there, a renamed feature's item takes its new name, what is
    /// hidden for now (its feature suppressed, rolled back or failed) is
    /// kept, and what no longer exists drops out and is said in
    /// `dropped`, which holds what dropped out at the last change of the
    /// document (an undo that follows clears it). Items are found again
    /// by their feature's id, never by its name, so a selection cannot
    /// move to another feature that took the name (rule 4).
    pub(crate) fn check(&mut self, doc: &Document, regen: &Regen, revision: u64) {
        if self.checked == Some(revision) {
            return;
        }
        self.checked = Some(revision);
        let mut kept = Vec::with_capacity(self.entries.len());
        let mut dropped = Vec::new();
        if !self.entries.is_empty() {
            let mut resolver = Resolver::new(doc, regen);
            for entry in &self.entries {
                match resolver.follow(&entry.seed) {
                    Fate::Found(now) => kept.push(Entry {
                        seed: entry.seed.clone(),
                        now,
                        hidden: None,
                    }),
                    Fate::Hidden(why) => kept.push(Entry {
                        seed: entry.seed.clone(),
                        now: Vec::new(),
                        hidden: Some(why),
                    }),
                    Fate::Gone(why) => {
                        let shown = entry.now.first().map_or(&entry.seed, |(now, _)| now);
                        dropped.push(format!("{} {}: {why}", shown.kind.name(), shown.name));
                    }
                }
            }
        }
        dropped.truncate(MAX_DROPPED);
        if kept != self.entries || dropped != self.dropped {
            self.entries = kept;
            self.dropped = dropped;
            self.serial = next_serial();
        }
    }

    pub(crate) fn view(&self) -> SelectionView {
        let mut items: Vec<SelectionItem> = Vec::with_capacity(self.entries.len());
        let mut hidden = Vec::new();
        let mut followed = Vec::new();
        for entry in &self.entries {
            if entry.now.is_empty() {
                hidden.push(HiddenItem {
                    item: entry.seed.clone(),
                    why: entry.hidden.clone().unwrap_or_default(),
                });
                continue;
            }
            for (now, copies) in &entry.now {
                if *now != entry.seed {
                    followed.push(FollowedItem {
                        item: now.clone(),
                        from: entry.seed.clone(),
                        copies: *copies,
                        pieces: entry.now.len(),
                    });
                }
                // Two seeds followed into one entity are one item.
                if !items.iter().any(|i| i.same(now)) {
                    items.push(now.clone());
                }
            }
        }
        SelectionView {
            items,
            serial: self.serial,
            dropped: self.dropped.clone(),
            hidden,
            followed,
        }
    }

    pub(crate) fn serial(&self) -> u64 {
        self.serial
    }
}

/// Entities followed into later features: per image, its body's feature,
/// its canonical name and how many copies were made on the way.
type FollowedNames = Result<Vec<(FeatureId, String, u64)>, String>;

/// What became of a kept item at a check ([`Resolver::follow`]).
pub(crate) enum Fate {
    /// What it is now: one item, or the pieces of a split, each with how
    /// many copies were made of it on the way.
    Found(Vec<(SelectionItem, u64)>),
    /// It has no body for now, and why.
    Hidden(String),
    /// It no longer exists, and why.
    Gone(String),
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

/// The names of one body's faces, edges and vertices, looked up by text:
/// the canonical name and the place of the entity (of a face cut into
/// pieces, its first piece).
#[derive(Default)]
struct BodyIndex {
    faces: Option<BTreeMap<String, (String, usize)>>,
    edges: Option<BTreeMap<String, (String, usize)>>,
    vertices: Option<BTreeMap<String, (String, usize)>>,
}

/// A body of a regeneration: its feature, the body, its names, and
/// whether it is still its feature's own (not taken by a later feature).
type BodyNames<'a> = (FeatureId, &'a Arc<Body>, Option<&'a Arc<Names>>, bool);

/// Finds what names mean in one regeneration of a document.
pub(crate) struct Resolver<'a> {
    doc: &'a Document,
    regen: &'a Regen,
    /// Every body, taken by a later feature or not, in history order.
    bodies: Vec<BodyNames<'a>>,
    index: BTreeMap<FeatureId, BodyIndex>,
}

impl<'a> Resolver<'a> {
    pub(crate) fn new(doc: &'a Document, regen: &'a Regen) -> Self {
        let bodies = regen
            .features
            .iter()
            .filter_map(|f| {
                Some((
                    f.id,
                    f.body.as_ref()?,
                    f.names.as_ref(),
                    f.consumed_by.is_none(),
                ))
            })
            .collect();
        Self {
            doc,
            regen,
            bodies,
            index: BTreeMap::new(),
        }
    }

    /// The canonical name `wanted` has among the entities of `kind` of the
    /// body of `feature`, and the entity's place there: a face by its name
    /// or an alias, an edge by its name or an alias, a vertex by its name.
    fn lookup(
        &mut self,
        feature: FeatureId,
        kind: SelectionKind,
        wanted: &str,
    ) -> Option<(String, usize)> {
        let (_, body, names, _) = self.bodies.iter().find(|(id, ..)| *id == feature)?;
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
                            .or_insert_with(|| (canonical.clone(), k));
                    }
                    table
                        .entry(canonical.clone())
                        .or_insert_with(|| (canonical, k));
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
                                .or_insert_with(|| (canonical.clone(), k));
                        }
                        table.insert(canonical.clone(), (canonical, k));
                    }
                }
                table
            }),
            SelectionKind::Vertex => entry.vertices.get_or_insert_with(|| {
                let mut table = BTreeMap::new();
                for k in 0..body.vertices.len() {
                    if let Some(name) = names.vertex_name(k) {
                        let canonical = name.to_string();
                        table.insert(canonical.clone(), (canonical, k));
                    }
                }
                table
            }),
            _ => return None,
        };
        if let Some(found) = table.get(wanted) {
            return Some(found.clone());
        }
        // The short form of a cap (`F2:startCap()`) names the cap of that
        // role when the body has one (`Names::face_answers_to`); with
        // several (a sketch of two profiles) it names none of them here.
        let short = wanted.parse::<PersistentName>().ok()?;
        if kind != SelectionKind::Face || !short.is_short_cap() {
            return None;
        }
        // Pieces of one cap share its name: the first stands for it, as
        // for any name. Caps of several loops are refused.
        if craft3d_regen::ambiguous_cap(names, &short).is_some() {
            return None;
        }
        let k = (0..names.faces.len()).find(|&k| names.face_answers_to(k, &short))?;
        names.faces.get(k).map(|name| (name.to_string(), k))
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
            .filter(|(.., own)| *own)
            .map(|(id, ..)| *id)
            .filter(|id| only.is_none_or(|only| only == *id))
            .collect();
        if let Some(only) = only
            && candidates.is_empty()
        {
            return Err(format!("`{}` has no body", self.called(only)));
        }
        for body in candidates {
            if let Some((canonical, _)) = self.lookup(body, kind, &wanted) {
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

    /// A face, an edge or a vertex by its persistent name, within the
    /// body of `feature` when given: the feature whose body has it, its
    /// kind and its index in that body (`asm.rs`, a mate's frame).
    pub(crate) fn locate(
        &mut self,
        name: &str,
        feature: Option<&str>,
    ) -> Result<(FeatureId, SelectionKind, usize), String> {
        let parsed: PersistentName = name.parse().map_err(|e| {
            format!(
                "`{}` is not a persistent name ({e})",
                name.chars().take(80).collect::<String>()
            )
        })?;
        let kind = if parsed.is_face() {
            SelectionKind::Face
        } else if parsed.is_edge() {
            SelectionKind::Edge
        } else if parsed.is_vertex() {
            SelectionKind::Vertex
        } else {
            return Err(format!("`{parsed}` names no face, edge or vertex"));
        };
        let found = self.resolve(Some(kind), name, feature)?;
        let id = feature_named(self.doc, &found.item.feature)
            .ok_or_else(|| format!("there is no feature {}", found.item.feature))?;
        let (_, index) = self
            .lookup(id, kind, &found.item.name)
            .ok_or_else(|| format!("no body has a {} `{}`", kind.name(), found.item.name))?;
        Ok((id, kind, index))
    }

    /// The feature's name, or its id when it has none.
    fn called(&self, id: FeatureId) -> String {
        self.doc
            .feature(id)
            .map_or_else(|| id.to_string(), |f| f.name.clone())
    }

    /// Why feature `id` has no body for now, when that is so because of
    /// its state (suppressed, rolled back, failed): it may have one again.
    /// `None` when it is built and has none, or is not there.
    fn absent(&self, id: FeatureId) -> Option<String> {
        let name = self.called(id);
        match &self.regen.feature(id)?.status {
            Status::Suppressed => Some(format!("`{name}` is suppressed")),
            Status::Inactive => Some(format!("`{name}` is inactive")),
            Status::RolledBack => Some(format!("`{name}` is rolled back")),
            Status::Failed(why) => Some(format!("`{name}` failed: {}", cut(why))),
            Status::Ok => None,
        }
    }

    /// What became of an item kept as `seed`, found again from what it
    /// was selected as: its feature by id only (`F3`), a face, an edge or
    /// a vertex by its name in that feature's body. A body or an entity
    /// whose body went into a later feature (a boolean, a fillet, a
    /// pattern) is followed to its image in the newest body, by the rule
    /// features follow names by (`craft3d_regen::follow_face`), through
    /// the copy names; of a pattern's several copies, the first. An item
    /// whose feature has no body for now (suppressed, rolled back, failed)
    /// is hidden, not gone: it comes back with the feature. Only what no
    /// longer exists anywhere is gone.
    pub(crate) fn follow(&mut self, seed: &SelectionItem) -> Fate {
        let Some(id) = seed
            .feature
            .strip_prefix('F')
            .and_then(|digits| digits.parse::<u64>().ok())
            .map(FeatureId)
            .filter(|id| self.doc.feature(*id).is_some())
        else {
            return Fate::Gone(format!("there is no feature {}", cut(&seed.feature)));
        };
        match seed.kind {
            SelectionKind::Body => self.follow_body(id),
            kind if kind.is_entity() => self.follow_entity(kind, id, &seed.name),
            kind => match self.feature_item(Some(kind), id) {
                Ok(found) => Fate::Found(vec![(found.item, 1)]),
                Err(why) => Fate::Gone(why),
            },
        }
    }

    /// The feature `id` has no body now: hidden or gone.
    fn without_body(&self, id: FeatureId) -> Fate {
        match self.absent(id) {
            Some(why) => Fate::Hidden(why),
            None => Fate::Gone(format!("`{}` has no body", self.called(id))),
        }
    }

    /// The features whose bodies the body of `id` is now: the end of its
    /// chain of takers, or every piece of a split (each a body of its
    /// own), in history order. Bounded by the history.
    fn newest_bodies(&self, id: FeatureId) -> Vec<FeatureId> {
        let mut out = Vec::new();
        let mut todo = vec![id];
        let mut seen = std::collections::BTreeSet::from([id]);
        while let Some(at) = todo.pop() {
            let Some(taker) = self.regen.feature(at).and_then(|o| o.consumed_by) else {
                out.push(at);
                continue;
            };
            let group = if craft3d_regen::taken_by_its_splits(self.doc, at, taker) {
                craft3d_regen::splits_of(self.doc, &self.regen.features, at)
            } else {
                vec![taker]
            };
            for next in group {
                if seen.insert(next) {
                    todo.push(next);
                }
            }
        }
        out.sort_by_key(|id| {
            self.doc
                .features
                .iter()
                .position(|f| f.id == *id)
                .unwrap_or(usize::MAX)
        });
        out
    }

    fn follow_body(&mut self, id: FeatureId) -> Fate {
        if self
            .regen
            .feature(id)
            .and_then(|o| o.body.as_ref())
            .is_none()
        {
            return self.without_body(id);
        }
        let found: Vec<(SelectionItem, u64)> = self
            .newest_bodies(id)
            .into_iter()
            .filter(|at| self.regen.feature(*at).and_then(|o| o.own_body()).is_some())
            .map(|at| {
                (
                    SelectionItem {
                        kind: SelectionKind::Body,
                        name: self.called(at),
                        feature: at.to_string(),
                    },
                    1,
                )
            })
            .collect();
        if found.is_empty() {
            return Fate::Gone(format!(
                "the body of `{}` went into later features, which have none",
                self.called(id)
            ));
        }
        Fate::Found(found)
    }

    fn follow_entity(&mut self, kind: SelectionKind, id: FeatureId, name: &str) -> Fate {
        let Some(outcome) = self.regen.feature(id) else {
            return self.without_body(id);
        };
        if outcome.body.is_none() {
            return self.without_body(id);
        }
        let own = outcome.consumed_by.is_none();
        let wanted = name
            .parse::<PersistentName>()
            .map(|name| name.to_string())
            .unwrap_or_else(|_| name.to_string());
        let Some((canonical, index)) = self.lookup(id, kind, &wanted) else {
            return Fate::Gone(format!(
                "the body of {id} has no {} of that name any more",
                kind.name()
            ));
        };
        if own {
            return Fate::Found(vec![(
                SelectionItem {
                    kind,
                    name: canonical,
                    feature: id.to_string(),
                },
                1,
            )]);
        }
        let followed = match kind {
            SelectionKind::Face => canonical
                .parse::<PersistentName>()
                .map_err(|e| e.to_string())
                .and_then(|name| self.follow_face(&name)),
            SelectionKind::Edge => self.follow_edge(id, index),
            _ => Err(format!(
                "the body of `{}` went into `{}`, and a vertex is not followed into a later feature yet",
                self.called(id),
                self.called(craft3d_regen::newest_body(&self.regen.features, id))
            )),
        };
        match followed {
            Ok(found) => Fate::Found(
                found
                    .into_iter()
                    .map(|(feature, name, copies)| {
                        (
                            SelectionItem {
                                kind,
                                name,
                                feature: feature.to_string(),
                            },
                            copies,
                        )
                    })
                    .collect(),
            ),
            Err(why) => Fate::Gone(why),
        }
    }

    /// The images of face `name` (of a body taken by later features) in
    /// the newest bodies: each body's feature, the face's canonical name
    /// there and how many copies were made of it on the way
    /// (`craft3d_regen::follow_face_pieces`).
    fn follow_face(&mut self, name: &PersistentName) -> FollowedNames {
        let images = craft3d_regen::follow_face_pieces(self.doc, &self.regen.features, name)?
            .unwrap_or_else(|| vec![(name.clone(), 1)]);
        let mut out = Vec::new();
        for (image, copies) in images {
            let newest = image.feature;
            let (canonical, _) = self
                .lookup(newest, SelectionKind::Face, &image.to_string())
                .ok_or_else(|| format!("{image} is not a face of `{}`", self.called(newest)))?;
            out.push((newest, canonical, copies));
        }
        Ok(out)
    }

    /// The edge at `index` of the body of `id`, taken by later features,
    /// in the newest bodies: every edge there between the images of the
    /// two faces it lies between (an edge a boolean cut in two is both
    /// pieces; a body split in pieces gives the edge in each).
    fn follow_edge(&mut self, id: FeatureId, index: usize) -> FollowedNames {
        let (_, body, names, _) = self
            .bodies
            .iter()
            .find(|(f, ..)| *f == id)
            .ok_or_else(|| format!("{id} has no body"))?;
        let names = names.ok_or_else(|| format!("`{}` names no edges", self.called(id)))?;
        let faces = craft3d_regen::faces_of_edges(body)
            .into_iter()
            .nth(index)
            .unwrap_or_default();
        let sides: Vec<PersistentName> = faces
            .iter()
            .filter_map(|face| usize::try_from(face.0).ok())
            .filter_map(|face| names.faces.get(face).cloned())
            .collect();
        let (a, b) = match sides.as_slice() {
            [a, b] => (a.clone(), b.clone()),
            [a] => (a.clone(), a.clone()),
            _ => return Err("the edge does not lie between faces".to_string()),
        };
        let follow = |name: &PersistentName| -> Result<Vec<(PersistentName, u64)>, String> {
            Ok(
                craft3d_regen::follow_face_pieces(self.doc, &self.regen.features, name)?
                    .unwrap_or_else(|| vec![(name.clone(), 1)]),
            )
        };
        let (left, right) = (follow(&a)?, follow(&b)?);
        let mut out = Vec::new();
        for (a, copies) in &left {
            for (b, _) in right.iter().filter(|(b, _)| b.feature == a.feature) {
                let Some((_, body, Some(names), _)) =
                    self.bodies.iter().find(|(f, ..)| *f == a.feature)
                else {
                    continue;
                };
                for edge in craft3d_regen::edges_between(body, names, a, b) {
                    if let Some(name) = usize::try_from(edge.0)
                        .ok()
                        .and_then(|edge| names.edge_name(edge))
                    {
                        let entry = (a.feature, name.to_string(), *copies);
                        if !out.contains(&entry) {
                            out.push(entry);
                        }
                    }
                }
            }
        }
        if out.is_empty() {
            let into = left
                .first()
                .map_or_else(|| id.to_string(), |(a, _)| self.called(a.feature));
            return Err(format!("the edge between {a} and {b} is gone in `{into}`"));
        }
        Ok(out)
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
                    Some(taker) => format!(
                        "the body of `{}` went into {taker}, `{}`",
                        feature.name,
                        self.doc
                            .feature(taker)
                            .map_or_else(|| taker.to_string(), |f| f.name.clone())
                    ),
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
    ///
    /// What a frontend reads every frame: while the document is
    /// regenerated elsewhere ([`Session::regen_elsewhere`]) and its
    /// current state not here, it is the selection as it was last
    /// checked (or as it was set), as [`Session::selection_serial`] is:
    /// reading it never regenerates. A command that needs it checked
    /// reads [`Session::checked_selection`].
    pub fn selection(&mut self) -> SelectionView {
        if !(self.regen_elsewhere() && !self.regenerated_here()) {
            self.check_selection();
        }
        self.document()
            .map_or_else(|_| SelectionView::default(), |open| open.selection.view())
    }

    /// The active document's selection, checked against its current
    /// regeneration first whatever it takes (it regenerates here when
    /// the state is not): what a command reads. Empty without a
    /// document.
    pub(crate) fn checked_selection(&mut self) -> SelectionView {
        self.check_selection();
        self.document()
            .map_or_else(|_| SelectionView::default(), |open| open.selection.view())
    }

    /// The serial of the active document's selection, checked against
    /// its current regeneration first: it moves whenever the items or
    /// what dropped out change, so a frontend copies the selection only
    /// then. 0 without a document.
    ///
    /// While the document is regenerated elsewhere
    /// ([`Session::regen_elsewhere`]) and not here, it is checked once it
    /// is regenerated here (a command that needs the bodies): a frame
    /// that asks never regenerates.
    pub fn selection_serial(&mut self) -> u64 {
        if self.regen_elsewhere() && !self.regenerated_here() {
            return self.document().map_or(0, |open| open.selection.serial());
        }
        self.check_selection()
    }

    /// [`Session::selection_serial`], checked whatever it takes.
    fn check_selection(&mut self) -> u64 {
        let Ok(open) = self.document_mut() else {
            return 0;
        };
        let revision = open.history.revision();
        if open.selection.checked != Some(revision) {
            let regen = open.regen();
            let doc = open.bodies_doc();
            open.selection.check(&doc, &regen, revision);
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
