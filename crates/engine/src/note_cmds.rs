//! Notes pinned to the model (2026-11-17; the data in
//! `craft3d_doc::note`): `note.add`, `note.list`, `note.resolve`,
//! `note.edit`, `note.delete`. The person writes a note at a place of
//! the model ("2 mm thicker here"); an agent reads the open ones, does
//! what each asks and marks it done with a sentence of reply; an agent
//! leaves a note of its own where it has a question about a place.
//!
//! A note is pinned to a body, a face, an edge or a vertex as a selection
//! keeps an item (`selection.rs`): by the feature whose body has it and
//! its persistent name there, found again at every look from what it was
//! pinned to, followed into the later features that took its body, hidden
//! while its feature is suppressed, rolled back or failed, and *detached*
//! when it is gone. A detached note is not changed: it stays at its
//! point, says why, and is pinned again when the entity comes back.
//!
//! Every change is one undo step, as every change of the document is;
//! none changes a body, since regeneration does not read the notes.

use crate::commands::{Run, edit, fail, spec};
use crate::params::Params;
use crate::selection::{Fate, Resolver, SelectionItem, SelectionKind};
use crate::{CommandError, CommandSpec, Session};
use craft3d_doc::{Document, FeatureId, MAX_NOTES, Note, NoteKind, NoteOn, PersistentName};
use craft3d_math::Vec3;
use craft3d_regen::Regen;
use craft3d_topo::Body;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::sync::Arc;

pub(crate) static SPECS: &[CommandSpec] = &[
    spec!("note.add", "Add Note", r#"{"text": string, "on": face, edge or vertex name, or a feature (its body) = the selection, "feature": id or name = the first body that has the name, "point": [x, y, z] = a point in the middle of what it is on}"#,
        "Pins a note to a place of the model: a sentence for whoever works on it next (\"2 mm thicker here\", \"this hole can go\", or an agent's question about that place). Where: on, what it is pinned to: a face, an edge or a vertex by its persistent name (document.inspect faceNames, document.edges, view.pick; a feature's name may stand for its id: Plate:face(1)), or a feature's name or id for its body; without on, the one item selected (select.get), a body, a face, an edge or a vertex. feature says which body has the name when several do. point is where on it, in the document's length unit, at most a fiftieth of the body's size off it; without it the point of the entity nearest its middle (a face's centre of area, an edge's middle, a vertex itself). The note stays pinned to that name through every edit that keeps the entity (a changed size, a later boolean or fillet that took its body); when the entity is gone it stays at its point, detached, and says why (note.list). Who wrote it is the agent running the command (by the name its client gave), or the person. text: 1 to 2000 characters, line breaks kept, no other control characters; a document holds at most 1000 notes. One undo step; saved with the document, left out of everything published (jig.export) and of every export. Returns the note as note.list lists it, with summary.", doc: true, names: ["on"], note_add),
    spec!("note.list", "List Notes", r#"{"state": "open|done|all" = "all"}"#,
        "Lists the notes pinned to the model, in the order they were written; state open lists only those still to do, done only those done. Each has id, text, state (open or done), author (person or agent, with agent its name), timeMs (when it was written, milliseconds since 1970; 0 without a clock), on (what it was pinned to: kind body, face, edge or vertex, name, feature: the body's feature id, body: that feature's name, origin: the feature that made the entity, at the end of its name's way, with its name and type: the one to edit to change it), point ([x, y, z] where it was put), attachment (attached: the entity is there, with at: the point of the entity as it is now nearest point, where the window draws its marker, and now: what it is in the newest body when a later feature took it; hidden: its feature is suppressed, rolled back or failed for now, with why; detached: the entity is gone, with why: the note stays at its point) and, for a done note, done: {author, agent, timeMs, reply}. Also count, open, done, detached, fromAgents (open notes an agent wrote: its questions to the person) and summary. An agent works through the open ones: does what each asks (on and origin say where), then note.resolve with a sentence of reply.", doc: true, note_list),
    spec!("note.resolve", "Resolve Note", r#"{"id": integer, "reply": string, "done": boolean = true}"#,
        "Marks note id done, with reply: one sentence of what was done or why not (1 to 2000 characters, as a note's text). It is closed by the agent running the command, or the person; done: false opens a done note again (its reply goes). A note done already, or open already with done: false, is an error that says so. One undo step. Returns the note, with summary.", doc: true, note_resolve),
    spec!("note.edit", "Edit Note", r#"{"id": integer, "text": string, "on": face, edge or vertex name, or a feature (its body), "feature": id or name, "point": [x, y, z]}"#,
        "Changes note id: its text, or where it is pinned. Given text replaces its text; on (with feature, as note.add takes them) pins it to another entity, at point or the new entity's middle; point alone moves it on what it is pinned to (anywhere, for a detached note). Who wrote it, its state and its reply stay. One undo step. Returns the note as note.list lists it, with summary.", doc: true, names: ["on"], note_edit),
    spec!("note.delete", "Delete Note", r#"{"id": integer or [integer, ...], "state": "done|all"}"#,
        "Takes notes away: the note id (or a list of ids), or every note done (state: done) or every note (state: all), one of the two. An id that is no note's is an error, and then none goes. One undo step (none when nothing is taken away). Returns deleted (the ids) and summary.", doc: true, note_delete),
];

/// Most ids `note.delete` takes at once.
const MAX_IDS: usize = MAX_NOTES;
/// How far from what it is pinned to a point may be, as a share of the
/// size of the body (the diagonal of its box): a point picked in a
/// picture lies on that picture's mesh, a little off the exact surface.
const NEAR: f64 = 0.02;
/// The mesh a note's point is placed on and checked against: within this
/// share of the body's size, at most this many triangles.
const MESH_SHARE: f64 = 1e-3;
const MESH_TRIANGLES: usize = 200_000;

fn cut(text: &str) -> String {
    text.chars().take(80).collect()
}

/// The agent running the command, by its client's name; `None` for the
/// person.
fn author(session: &Session) -> Option<String> {
    session.actor.as_ref().map(|actor| actor.client.clone())
}

fn selection_kind(kind: NoteKind) -> SelectionKind {
    match kind {
        NoteKind::Body => SelectionKind::Body,
        NoteKind::Face => SelectionKind::Face,
        NoteKind::Edge => SelectionKind::Edge,
        NoteKind::Vertex => SelectionKind::Vertex,
    }
}

fn note_kind(kind: SelectionKind) -> Option<NoteKind> {
    match kind {
        SelectionKind::Body => Some(NoteKind::Body),
        SelectionKind::Face => Some(NoteKind::Face),
        SelectionKind::Edge => Some(NoteKind::Edge),
        SelectionKind::Vertex => Some(NoteKind::Vertex),
        SelectionKind::Feature | SelectionKind::Sketch | SelectionKind::Datum => None,
    }
}

/// The feature id `F3` as an item writes it.
fn feature_id(text: &str) -> Option<FeatureId> {
    text.strip_prefix('F')
        .and_then(|digits| digits.parse::<u64>().ok())
        .map(FeatureId)
}

/// What a note is pinned to, as the selection keeps an item: the seed it
/// is found again from.
fn seed_of(doc: &Document, on: &NoteOn) -> SelectionItem {
    let name = match &on.name {
        Some(name) => name.to_string(),
        None => doc
            .feature(on.feature)
            .map_or_else(|| on.feature.to_string(), |f| f.name.clone()),
    };
    SelectionItem {
        kind: selection_kind(on.kind),
        name,
        feature: on.feature.to_string(),
    }
}

/// The feature that made what a note is pinned to: at the end of its
/// name's way (the cylinder that drilled a hole), or the body's feature.
fn origin(doc: &Document, on: &NoteOn) -> Value {
    let mut at = on.name.as_ref();
    let mut id = on.feature;
    // Bounded by the name itself: each link is a smaller part of it.
    while let Some(name) = at {
        id = name.feature;
        at = name.of.as_deref();
    }
    let made = doc.feature(id);
    json!({
        "feature": id.to_string(),
        "name": made.map(|f| f.name.clone()),
        "type": made.map(|f| f.kind.type_name()),
    })
}

fn author_json(by: Option<&String>) -> (Value, Value) {
    match by {
        Some(agent) => (json!("agent"), json!(agent)),
        None => (json!("person"), Value::Null),
    }
}

fn point_json(p: [f64; 3]) -> Value {
    json!(p)
}

fn item_json(doc: &Document, item: &SelectionItem) -> Value {
    let body = feature_id(&item.feature)
        .and_then(|id| doc.feature(id))
        .map(|f| f.name.clone());
    json!({"kind": item.kind.name(), "name": item.name, "feature": item.feature, "body": body})
}

/// Where a note is now ([`listed`]): what became of the entity it is
/// pinned to (`None` for a note at its point alone), and the point of that
/// entity nearest the note's point, where its marker is drawn.
struct Now {
    fate: Option<Fate>,
    at: Option<[f64; 3]>,
}

/// A note as the commands give it; with `now`, where it is now
/// (`attachment`, `now`, `at`).
fn note_json(doc: &Document, note: &Note, now: Option<&Now>) -> Value {
    let (author, agent) = author_json(note.by.as_ref());
    let mut out = Map::new();
    out.insert("id".into(), note.id.into());
    out.insert("text".into(), note.text.clone().into());
    out.insert(
        "state".into(),
        if note.done.is_some() { "done" } else { "open" }.into(),
    );
    out.insert("author".into(), author);
    out.insert("agent".into(), agent);
    out.insert("timeMs".into(), note.time_ms.into());
    match &note.on {
        Some(on) => {
            let seed = seed_of(doc, on);
            let mut pinned = item_json(doc, &seed);
            if let Some(map) = pinned.as_object_mut() {
                map.insert("origin".into(), origin(doc, on));
            }
            out.insert("on".into(), pinned);
            match now.and_then(|now| now.fate.as_ref()) {
                Some(Fate::Found(found)) => {
                    out.insert("attachment".into(), "attached".into());
                    if let Some((first, _)) = found.first()
                        && !first.same(&seed)
                    {
                        out.insert("now".into(), item_json(doc, first));
                    }
                }
                Some(Fate::Hidden(why)) => {
                    out.insert("attachment".into(), "hidden".into());
                    out.insert("why".into(), why.clone().into());
                }
                Some(Fate::Gone(why)) => {
                    out.insert("attachment".into(), "detached".into());
                    out.insert("why".into(), why.clone().into());
                }
                None => {}
            }
        }
        None => {
            out.insert("on".into(), Value::Null);
            if now.is_some() {
                out.insert("attachment".into(), "detached".into());
                out.insert("why".into(), "it is pinned to its point alone".into());
            }
        }
    }
    out.insert("point".into(), point_json(note.point));
    if let Some(at) = now.and_then(|now| now.at) {
        out.insert("at".into(), point_json(at));
    }
    if let Some(done) = &note.done {
        let (author, agent) = author_json(done.by.as_ref());
        out.insert(
            "done".into(),
            json!({"author": author, "agent": agent, "timeMs": done.time_ms, "reply": done.reply}),
        );
    }
    Value::Object(out)
}

/// What a note is on, in words: "face F2:side(e5) of Plate".
fn where_words(doc: &Document, on: Option<&NoteOn>) -> String {
    let Some(on) = on else {
        return "its point".to_string();
    };
    let body = doc
        .feature(on.feature)
        .map_or_else(|| on.feature.to_string(), |f| f.name.clone());
    match &on.name {
        Some(name) => format!("{} {} of {}", on.kind.name(), cut(&name.to_string()), body),
        None => format!("the body of {body}"),
    }
}

/// The note `id` of the active document, or the error that says there is
/// none.
fn note_of(session: &Session, command: &str, id: u64) -> Result<Note, CommandError> {
    let doc = session.document()?.history.current();
    doc.note(id).cloned().ok_or_else(|| {
        let ids: Vec<String> = doc
            .notes
            .items
            .iter()
            .take(20)
            .map(|n| n.id.to_string())
            .collect();
        CommandError::new(format!(
            "{command}: there is no note {id}; notes: {}",
            if ids.is_empty() {
                "none".to_string()
            } else {
                ids.join(", ")
            }
        ))
    })
}

/// A point given as `[x, y, z]`: three finite numbers.
fn point_param(p: &Params, key: &str) -> Result<Option<[f64; 3]>, CommandError> {
    let Some(value) = p.get(key) else {
        return Ok(None);
    };
    let wrong = || {
        CommandError::new(format!(
            "{}: `{key}` must be [x, y, z], three finite numbers in the document's length unit",
            p.command()
        ))
    };
    let Some([x, y, z]) = value.as_array().map(Vec::as_slice) else {
        return Err(wrong());
    };
    let coordinate = |v: &Value| v.as_f64().filter(|c| c.is_finite());
    match (coordinate(x), coordinate(y), coordinate(z)) {
        (Some(x), Some(y), Some(z)) => Ok(Some([x, y, z])),
        _ => Err(wrong()),
    }
}

/// The entity a note goes on, found in the document as it is now: its
/// seed, the body that has it and its place there.
struct Target {
    on: NoteOn,
    body: Arc<Body>,
    /// Its index among the body's faces, edges or vertices (unused for a
    /// body).
    index: usize,
}

/// What `on` (with `feature`) names, or without it the one item
/// selected, as a [`Target`].
fn target(
    session: &mut Session,
    command: &str,
    on: Option<&str>,
    feature: Option<&str>,
) -> Result<Target, CommandError> {
    let err = |e: String| CommandError::new(format!("{command}: {e}"));
    let selected = match on {
        Some(_) => None,
        None => {
            if feature.is_some() {
                return Err(err(
                    "`feature` says which body has the name given as `on`; give `on` too".into(),
                ));
            }
            let view = session.checked_selection();
            match view.items.as_slice() {
                [one] => Some(one.clone()),
                [] => {
                    return Err(err(
                        "nothing is selected: say where the note goes with `on` (a face, an edge or a vertex by its name, or a feature for its body), or select one thing first".into(),
                    ));
                }
                many => {
                    return Err(err(format!(
                        "{} are selected; a note goes on one: select one, or name it with `on`",
                        crate::selection::summary(many)
                    )));
                }
            }
        }
    };
    let open = session.document_mut()?;
    if open.history.current().is_assembly() {
        return Err(err(
            "notes are pinned in a part, not yet in an assembly: open the part (instance.open) and pin it there".into(),
        ));
    }
    let regen = open.regen();
    let doc = Arc::clone(open.history.current());
    let mut resolver = Resolver::new(&doc, &regen);
    // What the note is pinned by (`seed`), and what that is now (`now`),
    // whose geometry places the point.
    let (seed, now) = match (selected, on) {
        (Some(item), _) => (item.clone(), item),
        (None, Some(name)) => match resolver.resolve(None, name, feature) {
            Ok(found) => (found.item.clone(), found.item),
            // A name a body had before a later feature took the body: the
            // note is pinned by that name, as the selection keeps what was
            // selected, and found in the newest body.
            Err(why) => earlier(&doc, &mut resolver, name, feature).ok_or_else(|| err(why))?,
        },
        (None, None) => return Err(err("say where the note goes with `on`".into())),
    };
    let kind = note_kind(seed.kind).ok_or_else(|| {
        if seed.kind == SelectionKind::Feature {
            err(format!(
                "`{}` has no body of its own now (a later feature took it, or it makes none): pin the note on the body that has its faces, or on one of them",
                cut(&seed.name)
            ))
        } else {
            err(format!(
                "`{}` is a {}; a note is pinned to a body, a face, an edge or a vertex",
                cut(&seed.name),
                seed.kind.name()
            ))
        }
    })?;
    let id = feature_id(&seed.feature)
        .ok_or_else(|| err(format!("there is no feature {}", cut(&seed.feature))))?;
    let body = feature_id(&now.feature)
        .and_then(|now| regen.feature(now))
        .and_then(|o| o.own_body())
        .cloned()
        .ok_or_else(|| err(format!("`{}` has no body now", cut(&now.name))))?;
    if kind == NoteKind::Body {
        return Ok(Target {
            on: NoteOn {
                kind,
                feature: id,
                name: None,
            },
            body,
            index: 0,
        });
    }
    let name = seed
        .name
        .parse::<PersistentName>()
        .map_err(|e| {
            err(format!(
                "`{}` is not a persistent name ({e})",
                cut(&seed.name)
            ))
        })?
        .canonical();
    let (_, _, index) = resolver
        .locate(&now.name, Some(&now.feature))
        .map_err(err)?;
    Ok(Target {
        on: NoteOn {
            kind,
            feature: id,
            name: Some(name),
        },
        body,
        index,
    })
}

/// `name`, a face, an edge or a vertex of the body of its own feature
/// (or of `feature`) that a later feature took, as a seed and what it is
/// now in the newest body: `None` when it is no such name, or is not
/// found there.
fn earlier(
    doc: &Document,
    resolver: &mut Resolver<'_>,
    name: &str,
    feature: Option<&str>,
) -> Option<(SelectionItem, SelectionItem)> {
    let parsed = name.parse::<PersistentName>().ok()?.canonical();
    let kind = if parsed.is_face() {
        SelectionKind::Face
    } else if parsed.is_edge() {
        SelectionKind::Edge
    } else if parsed.is_vertex() {
        SelectionKind::Vertex
    } else {
        return None;
    };
    let body = match feature {
        Some(text) => crate::selection::feature_named(doc, text)?,
        None => parsed.feature,
    };
    let seed = SelectionItem {
        kind,
        name: parsed.to_string(),
        feature: body.to_string(),
    };
    match resolver.follow(&seed) {
        Fate::Found(now) => {
            let (first, _) = now.into_iter().next()?;
            Some((seed, first))
        }
        Fate::Hidden(_) | Fate::Gone(_) => None,
    }
}

/// The mesh and the edges a note's point is placed on and checked
/// against, and the size of the body.
struct Shape {
    mesh: craft3d_tess::Mesh,
    edges: Vec<Vec<Vec3>>,
    chordal: f64,
    size: f64,
}

fn shape_of(body: &Body, linear: f64) -> Result<Shape, String> {
    let size = body
        .bbox()
        .ok()
        .map(|b| b.diagonal())
        .filter(|d| d.is_finite() && *d > 0.0)
        .unwrap_or(1.0);
    let chordal = (size * MESH_SHARE).max(10.0 * linear);
    let params = craft3d_tess::TessParams {
        chordal_tolerance: chordal,
        max_triangles: Some(MESH_TRIANGLES),
        max_angle: Some(15f64.to_radians()),
        ..craft3d_tess::TessParams::default()
    };
    let made = craft3d_tess::tessellate_within_budget(body, &params)
        .map_err(|e| format!("the body cannot be meshed to place the note: {e}"))?;
    Ok(Shape {
        mesh: made.mesh,
        edges: made.edges,
        chordal: made.tolerance,
        size,
    })
}

/// The triangles of `face` (every face when `None`).
fn triangles(mesh: &craft3d_tess::Mesh, face: Option<usize>) -> Vec<[Vec3; 3]> {
    mesh.face_ids
        .iter()
        .enumerate()
        .filter(|(_, f)| face.is_none_or(|face| usize::try_from(**f).ok() == Some(face)))
        .filter_map(|(t, _)| mesh.corners(t))
        .collect()
}

/// The point of `tris` nearest `p`, and its distance; `None` for none.
fn nearest_on(tris: &[[Vec3; 3]], p: Vec3) -> Option<(Vec3, f64)> {
    tris.iter()
        .map(|[a, b, c]| {
            let q = crate::analysis::closest_on_triangle(p, *a, *b, *c);
            (q, (q - p).length())
        })
        .filter(|(_, d)| !d.is_nan())
        .min_by(|x, y| x.1.total_cmp(&y.1))
}

/// The point of the polyline `line` nearest `p`, and its distance.
fn nearest_on_line(line: &[Vec3], p: Vec3) -> Option<(Vec3, f64)> {
    let segment = |a: Vec3, b: Vec3| {
        let ab = b - a;
        let len2 = ab.dot(ab);
        let t = if len2 > 0.0 {
            ((p - a).dot(ab) / len2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let q = a + ab * t;
        (q, (q - p).length())
    };
    let single = line.first().map(|a| (*a, (*a - p).length()));
    line.windows(2)
        .filter_map(|w| match w {
            [a, b] => Some(segment(*a, *b)),
            _ => None,
        })
        .chain(single)
        .filter(|(_, d)| !d.is_nan())
        .min_by(|x, y| x.1.total_cmp(&y.1))
}

/// The point of the triangles nearest their centre of area.
fn middle_of(tris: &[[Vec3; 3]]) -> Option<Vec3> {
    let mut total = 0.0;
    let mut sum = Vec3::new(0.0, 0.0, 0.0);
    for [a, b, c] in tris {
        let area = (*b - *a).cross(*c - *a).length() * 0.5;
        if area.is_finite() && area > 0.0 {
            total += area;
            sum += (*a + *b + *c) * (area / 3.0);
        }
    }
    let centre = if total > 0.0 {
        sum * (1.0 / total)
    } else {
        let [a, b, c] = tris.first()?;
        (*a + *b + *c) * (1.0 / 3.0)
    };
    nearest_on(tris, centre).map(|(q, _)| q)
}

/// The middle of an edge: its curve at the middle of its range, or the
/// middle of its polyline.
fn edge_middle(body: &Body, shape: &Shape, index: usize) -> Option<Vec3> {
    if let Some(edge) = body.edges.get(index) {
        let (t0, t1) = edge.range;
        let mid = 0.5 * (t0 + t1);
        if mid.is_finite() {
            let p = edge.curve.eval(mid);
            if [p.x, p.y, p.z].iter().all(|c| c.is_finite()) {
                return Some(p);
            }
        }
    }
    let line = shape.edges.get(index)?;
    let total: f64 = line.windows(2).map(segment_length).sum();
    let mut left = total * 0.5;
    for w in line.windows(2) {
        let length = segment_length(w);
        if let [a, b] = w
            && length >= left
            && length > 0.0
        {
            return Some(*a + (*b - *a) * (left / length));
        }
        left -= length;
    }
    line.first().copied()
}

fn segment_length(w: &[Vec3]) -> f64 {
    match w {
        [a, b] => (*b - *a).length(),
        _ => 0.0,
    }
}

/// Where a note on `target` goes: `point` when it is near enough the
/// entity, else its middle.
fn place(target: &Target, point: Option<[f64; 3]>, linear: f64) -> Result<[f64; 3], String> {
    let shape = shape_of(&target.body, linear)?;
    let what = target.on.kind.name();
    let face = match target.on.kind {
        NoteKind::Face => Some(target.index),
        _ => None,
    };
    let Some(given) = point else {
        let middle = match target.on.kind {
            NoteKind::Vertex => target.body.vertices.get(target.index).map(|v| v.point),
            NoteKind::Edge => edge_middle(&target.body, &shape, target.index),
            NoteKind::Face | NoteKind::Body => middle_of(&triangles(&shape.mesh, face)),
        };
        let p = middle.ok_or_else(|| format!("the {what} has no point to pin the note at"))?;
        return Ok([p.x, p.y, p.z]);
    };
    let p = Vec3::new(given[0], given[1], given[2]);
    let distance = match target.on.kind {
        NoteKind::Vertex => target
            .body
            .vertices
            .get(target.index)
            .map(|v| (v.point - p).length()),
        NoteKind::Edge => shape
            .edges
            .get(target.index)
            .and_then(|line| nearest_on_line(line, p))
            .map(|(_, d)| d),
        NoteKind::Face | NoteKind::Body => {
            nearest_on(&triangles(&shape.mesh, face), p).map(|(_, d)| d)
        }
    }
    .ok_or_else(|| format!("the {what} has no point to measure the note's point from"))?;
    let allowed = NEAR * shape.size + 2.0 * shape.chordal + linear;
    // NaN is not near.
    let near = distance <= allowed;
    if !near {
        return Err(format!(
            "`point` is {} from the {what} (at most {} is taken as on it); give a point on it, or leave `point` out for one in its middle",
            short(distance),
            short(allowed)
        ));
    }
    Ok(given)
}

/// A length for a message: six significant digits.
fn short(x: f64) -> String {
    if x.is_finite() {
        let rounded: f64 = format!("{x:.5e}").parse().unwrap_or(x);
        rounded.to_string()
    } else {
        "infinitely far".to_string()
    }
}

/// Every note as note.list gives them, checked against the document as
/// it is now.
fn listed(
    session: &mut Session,
    filter: impl Fn(&Note) -> bool,
) -> Result<Vec<Value>, CommandError> {
    let open = session.document_mut()?;
    let doc = Arc::clone(open.history.current());
    if doc.notes.is_empty() {
        return Ok(Vec::new());
    }
    let regen: Arc<Regen> = open.regen();
    let mut resolver = Resolver::new(&doc, &regen);
    let linear = doc.tolerance.linear;
    // One mesh per body at most, made only for a nearest point not
    // remembered from an earlier look.
    let mut shapes: BTreeMap<FeatureId, Option<Shape>> = BTreeMap::new();
    let mut out = Vec::new();
    for note in doc.notes.items.iter().filter(|n| filter(n)) {
        let fate = note
            .on
            .as_ref()
            .map(|on| resolver.follow(&seed_of(&doc, on)));
        let at = match &fate {
            Some(Fate::Found(found)) => found.first().and_then(|(item, _)| {
                nearest_now(&regen, &mut resolver, &mut shapes, linear, item, note.point)
            }),
            _ => None,
        };
        out.push(note_json(&doc, note, Some(&Now { fate, at })));
    }
    Ok(out)
}

/// The point of `item` (what a note's entity is now) nearest the note's
/// `point`: where its marker is drawn, on the entity as it is now. On the
/// mesh a note's point is placed on (within a thousandth of the body's
/// size of the exact surface; a vertex itself), remembered per body.
fn nearest_now(
    regen: &Regen,
    resolver: &mut Resolver<'_>,
    shapes: &mut BTreeMap<FeatureId, Option<Shape>>,
    linear: f64,
    item: &SelectionItem,
    point: [f64; 3],
) -> Option<[f64; 3]> {
    let kind = note_kind(item.kind)?;
    let id = feature_id(&item.feature)?;
    let body = regen.feature(id)?.own_body()?.clone();
    let index = match kind {
        NoteKind::Body => 0,
        _ => resolver.locate(&item.name, Some(&item.feature)).ok()?.2,
    };
    let key: Nearest = (kind, index, point.map(f64::to_bits));
    if let Some(known) = crate::mass::recall_key(&NEAREST, &body, &key) {
        return known;
    }
    let p = Vec3::new(point[0], point[1], point[2]);
    let found = match kind {
        NoteKind::Vertex => body.vertices.get(index).map(|v| v.point),
        _ => {
            let shape = shapes
                .entry(id)
                .or_insert_with(|| shape_of(&body, linear).ok())
                .as_ref();
            shape.and_then(|shape| match kind {
                NoteKind::Edge => shape
                    .edges
                    .get(index)
                    .and_then(|line| nearest_on_line(line, p))
                    .map(|(q, _)| q),
                NoteKind::Face => {
                    nearest_on(&triangles(&shape.mesh, Some(index)), p).map(|(q, _)| q)
                }
                _ => nearest_on(&triangles(&shape.mesh, None), p).map(|(q, _)| q),
            })
        }
    }
    .map(|q| [q.x, q.y, q.z])
    .filter(|q| q.iter().all(|c| c.is_finite()));
    crate::mass::remember_key(&NEAREST, &body, key, found);
    found
}

/// A note's nearest point is remembered by what it is on (its kind and
/// place in the body) and the bits of its point.
type Nearest = (NoteKind, usize, [u64; 3]);
static NEAREST: std::sync::Mutex<crate::mass::Store<BTreeMap<Nearest, Option<[f64; 3]>>>> =
    std::sync::Mutex::new(crate::mass::Store::new());

/// The note `id` as note.list gives it, with `summary`.
fn one(session: &mut Session, id: u64, summary: String) -> Run {
    let mut found = listed(session, |n| n.id == id)?;
    let mut note = found.pop().unwrap_or(Value::Null);
    if let Some(map) = note.as_object_mut() {
        map.insert("summary".into(), summary.into());
    }
    Ok(note)
}

fn note_add(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "note.add";
    let p = Params::new(ID, params, &["text", "on", "feature", "point"])?;
    let text = p.required_str("text")?;
    // Checked before anything is looked up: what is wrong with the text
    // is said first.
    let text =
        craft3d_doc::clean_note_text(text).map_err(|e| CommandError::new(format!("{ID}: {e}")))?;
    let on = p.str("on")?;
    let feature = p.str("feature")?;
    let point = point_param(&p, "point")?;
    {
        let notes = &session.document()?.history.current().notes;
        if notes.items.len() >= MAX_NOTES {
            return fail(format!(
                "{ID}: the document holds {MAX_NOTES} notes, as many as it may; delete some first (note.delete with state: done takes away those done)"
            ));
        }
    }
    let target = target(session, ID, on, feature)?;
    let linear = session.document()?.history.current().tolerance.linear;
    let point =
        place(&target, point, linear).map_err(|e| CommandError::new(format!("{ID}: {e}")))?;
    let by = author(session);
    let time = crate::agent_log::now_ms();
    let doc = session.document()?.history.current();
    let words = where_words(doc, Some(&target.on));
    let label = format!("Add note on {words}");
    let id = edit(session, ID, &label, |doc| {
        doc.add_note(&text, Some(target.on), point, by.as_deref(), time)
    })?;
    one(session, id, format!("Note {id} pinned to {words}"))
}

fn note_list(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "note.list";
    let p = Params::new(ID, params, &["state"])?;
    let state = p.str("state")?.unwrap_or("all");
    let wanted: fn(&Note) -> bool = match state {
        "all" => |_| true,
        "open" => |n| n.done.is_none(),
        "done" => |n| n.done.is_some(),
        _ => return fail(format!("{ID}: `state` must be open, done or all")),
    };
    let notes = listed(session, wanted)?;
    let doc = session.document()?.history.current();
    let count = doc.notes.items.len();
    let open = doc.notes.open();
    let from_agents = doc
        .notes
        .items
        .iter()
        .filter(|n| n.done.is_none() && n.by.is_some())
        .count();
    let detached = notes
        .iter()
        .filter(|n| n["attachment"] == "detached")
        .count();
    let mut summary = match count {
        0 => "no notes".to_string(),
        1 => "1 note".to_string(),
        n => format!("{n} notes"),
    };
    if count > 0 {
        summary.push_str(&format!(": {open} open"));
        if from_agents > 0 {
            summary.push_str(&format!(" ({from_agents} from an agent)"));
        }
        summary.push_str(&format!(", {} done", count - open));
        if detached > 0 {
            summary.push_str(&format!("; {detached} detached"));
        }
    }
    Ok(json!({
        "notes": notes,
        "count": count,
        "open": open,
        "done": count - open,
        "fromAgents": from_agents,
        "detached": detached,
        "summary": summary,
    }))
}

/// The `id` parameter: a note's id.
fn id_param(p: &Params) -> Result<u64, CommandError> {
    p.integer("id", 1, u64::MAX)?.ok_or_else(|| p.missing("id"))
}

fn note_resolve(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "note.resolve";
    let p = Params::new(ID, params, &["id", "reply", "done"])?;
    let id = id_param(&p)?;
    let reply = p.str("reply")?;
    let done = p.bool("done")?.unwrap_or(true);
    if !done && reply.is_some() {
        return fail(format!(
            "{ID}: a reply goes with done: true; to open note {id} again give done: false alone"
        ));
    }
    let reply = reply
        .map(|r| {
            craft3d_doc::clean_note_text(r).map_err(|e| {
                CommandError::new(format!("{ID}: {}", e.replace("note's text", "reply")))
            })
        })
        .transpose()?;
    let note = note_of(session, ID, id)?;
    match (&note.done, done) {
        (Some(was), true) => {
            let by = was.by.as_deref().unwrap_or("the person");
            return fail(format!(
                "{ID}: note {id} is done already (by {by}); open it again with done: false first"
            ));
        }
        (None, false) => return fail(format!("{ID}: note {id} is open")),
        _ => {}
    }
    let by = author(session);
    let time = crate::agent_log::now_ms();
    let label = if done {
        format!("Resolve note {id}")
    } else {
        format!("Reopen note {id}")
    };
    edit(session, ID, &label, |doc| {
        doc.resolve_note(id, done, by.as_deref(), reply.as_deref(), time)
    })?;
    let doc = session.document()?.history.current();
    let Some(note) = doc.note(id) else {
        return fail(format!("{ID}: note {id} is gone"));
    };
    let mut out = note_json(doc, note, None);
    let summary = match (done, &reply) {
        (true, Some(reply)) => format!("Note {id} done: {}", cut(reply)),
        (true, None) => format!("Note {id} done"),
        (false, _) => format!("Note {id} open again"),
    };
    if let Some(map) = out.as_object_mut() {
        map.insert("summary".into(), summary.into());
    }
    Ok(out)
}

fn note_edit(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "note.edit";
    let p = Params::new(ID, params, &["id", "text", "on", "feature", "point"])?;
    let id = id_param(&p)?;
    let text = p
        .str("text")?
        .map(|t| {
            craft3d_doc::clean_note_text(t).map_err(|e| CommandError::new(format!("{ID}: {e}")))
        })
        .transpose()?;
    let on = p.str("on")?;
    let feature = p.str("feature")?;
    let point = point_param(&p, "point")?;
    if text.is_none() && on.is_none() && point.is_none() {
        return fail(format!(
            "{ID}: give what changes: text, on (with feature) or point"
        ));
    }
    if on.is_none() && feature.is_some() {
        return fail(format!(
            "{ID}: `feature` says which body has the name given as `on`; give `on` too"
        ));
    }
    let note = note_of(session, ID, id)?;
    let linear = session.document()?.history.current().tolerance.linear;
    let err = |e: String| CommandError::new(format!("{ID}: {e}"));
    let pin = if let Some(on) = on {
        let target = target(session, ID, Some(on), feature)?;
        let point = place(&target, point, linear).map_err(err)?;
        Some((Some(target.on), point))
    } else if let Some(point) = point {
        // Moved on what it is pinned to: checked against that entity as
        // it is now; a detached note may go anywhere.
        let current = note
            .on
            .as_ref()
            .map(|on| current_target(session, on))
            .transpose()
            .map_err(err)?
            .flatten();
        match current {
            Some(target) => {
                place(&target, Some(point), linear).map_err(err)?;
            }
            None => craft3d_doc::check_note_point(point).map_err(err)?,
        }
        Some((note.on.clone(), point))
    } else {
        None
    };
    let label = format!("Edit note {id}");
    edit(session, ID, &label, |doc| {
        if let Some(text) = &text {
            doc.edit_note_text(id, text)?;
        }
        if let Some((on, point)) = pin {
            doc.repin_note(id, on, point)?;
        }
        Ok(())
    })?;
    let doc = session.document()?.history.current();
    let words = where_words(doc, doc.note(id).and_then(|n| n.on.as_ref()));
    one(session, id, format!("Note {id} on {words} changed"))
}

/// The entity `on` is now, as a [`Target`]: `None` when it is hidden or
/// gone (a detached note).
fn current_target(session: &mut Session, on: &NoteOn) -> Result<Option<Target>, String> {
    let open = session
        .document_mut()
        .map_err(|e| e.message().to_string())?;
    let regen = open.regen();
    let doc = Arc::clone(open.history.current());
    let mut resolver = Resolver::new(&doc, &regen);
    let seed = seed_of(&doc, on);
    let Fate::Found(now) = resolver.follow(&seed) else {
        return Ok(None);
    };
    let Some((item, _)) = now.first() else {
        return Ok(None);
    };
    let Some(id) = feature_id(&item.feature) else {
        return Ok(None);
    };
    let Some(body) = regen.feature(id).and_then(|o| o.own_body()).cloned() else {
        return Ok(None);
    };
    let index = if on.kind == NoteKind::Body {
        0
    } else {
        resolver.locate(&item.name, Some(&item.feature))?.2
    };
    Ok(Some(Target {
        on: on.clone(),
        body,
        index,
    }))
}

fn note_delete(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "note.delete";
    let p = Params::new(ID, params, &["id", "state"])?;
    let state = p.str("state")?;
    let doc = session.document()?.history.current();
    let ids: Vec<u64> = match (p.get("id"), state) {
        (Some(_), Some(_)) => {
            return fail(format!("{ID}: give `id` or `state`, not both"));
        }
        (None, None) => {
            return fail(format!(
                "{ID}: give `id` (a note's id, or a list of them) or `state` (done or all)"
            ));
        }
        (Some(Value::Array(list)), None) => {
            if list.is_empty() || list.len() > MAX_IDS {
                return fail(format!("{ID}: `id` must list 1 to {MAX_IDS} note ids"));
            }
            let mut ids = Vec::with_capacity(list.len());
            for value in list {
                let Some(id) = value.as_u64().filter(|id| *id >= 1) else {
                    return fail(format!(
                        "{ID}: `id` must be a note's id (a whole number from 1) or a list of them, got {}",
                        cut(&value.to_string())
                    ));
                };
                if !ids.contains(&id) {
                    ids.push(id);
                }
            }
            ids
        }
        (Some(_), None) => vec![id_param(&p)?],
        (None, Some("done")) => doc
            .notes
            .items
            .iter()
            .filter(|n| n.done.is_some())
            .map(|n| n.id)
            .collect(),
        (None, Some("all")) => doc.notes.items.iter().map(|n| n.id).collect(),
        (None, Some(_)) => return fail(format!("{ID}: `state` must be done or all")),
    };
    if let Some(missing) = ids.iter().find(|id| doc.note(**id).is_none()) {
        note_of(session, ID, *missing)?;
    }
    if ids.is_empty() {
        return Ok(json!({"deleted": [], "summary": "No note to delete"}));
    }
    let label = match ids.as_slice() {
        [one] => format!("Delete note {one}"),
        many => format!("Delete {} notes", many.len()),
    };
    let gone = edit(session, ID, &label, |doc| doc.remove_notes(&ids))?;
    let deleted: Vec<u64> = gone.iter().map(|n| n.id).collect();
    let summary = match deleted.as_slice() {
        [one] => format!("Deleted note {one}"),
        many => format!("Deleted {} notes", many.len()),
    };
    Ok(json!({"deleted": deleted, "summary": summary}))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;

    #[test]
    fn a_polyline_and_triangles_give_their_nearest_points_and_middles() {
        let line = [
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(2.0, 0.0, 0.0),
            Vec3::new(2.0, 2.0, 0.0),
        ];
        let (q, d) = nearest_on_line(&line, Vec3::new(1.0, 1.0, 0.0)).unwrap();
        assert!((d - 1.0).abs() < 1e-12, "{q:?} {d}");
        assert!(nearest_on_line(&[], Vec3::new(0.0, 0.0, 0.0)).is_none());
        // A square ring of two triangles each side: its middle lies on it.
        let tri = |a: [f64; 3], b: [f64; 3], c: [f64; 3]| {
            [
                Vec3::new(a[0], a[1], a[2]),
                Vec3::new(b[0], b[1], b[2]),
                Vec3::new(c[0], c[1], c[2]),
            ]
        };
        let ring = [
            tri([0.0, 0.0, 0.0], [3.0, 0.0, 0.0], [3.0, 1.0, 0.0]),
            tri([0.0, 0.0, 0.0], [3.0, 1.0, 0.0], [0.0, 1.0, 0.0]),
            tri([0.0, 2.0, 0.0], [3.0, 2.0, 0.0], [3.0, 3.0, 0.0]),
            tri([0.0, 2.0, 0.0], [3.0, 3.0, 0.0], [0.0, 3.0, 0.0]),
        ];
        let middle = middle_of(&ring).unwrap();
        let (_, d) = nearest_on(&ring, middle).unwrap();
        assert!(d < 1e-12, "{middle:?}");
        assert!(middle_of(&[]).is_none());
        assert_eq!(short(f64::INFINITY), "infinitely far");
        assert_eq!(craft3d_doc::MAX_NOTE_CHARS, 2000);
    }
}
