//! Which colour and which material each body and each face has: the
//! entries of the document (`Document::bodies`, by the feature that makes
//! a body) carried through the features that make bodies out of bodies.
//!
//! The rule (docs/design/document.md):
//!
//! - **A body** has the appearance and the material of the feature that
//!   makes it, when that feature has one; otherwise those of the body it
//!   was made from as instance 0 (a boolean's target, the body a fillet
//!   rounds, a pattern repeats or a transform moves), and so on back. So
//!   a cut keeps the colour of the plate it cut, and a fillet does not
//!   turn a red part grey.
//! - **A face** has its body's colour, unless it came into its body as
//!   part of another body that has a colour of its own (decided
//!   2026-10-15). Precisely: walk the face's way back as its persistent
//!   name records it (`F7:copy(1)/F3:side(e2)` came through instance 1 of
//!   F7, the tool, from F3), the feature whose body it is in first. The
//!   first feature on that way that has an appearance of its own gives
//!   the face its colour. A step through an instance whose material the
//!   result does not keep (the tool of a subtraction: its faces bound
//!   the target's material there, turned inside out) ends the way: the
//!   face has the colour of the body at that step, the target's. So a
//!   red boss united with a grey block keeps red faces, and the wall of
//!   a hole a red tool cut into a grey plate is grey, as SolidWorks,
//!   Fusion and Onshape colour it. A face the feature made itself (a
//!   fillet's round) has the body's colour, and so has a face whose way
//!   meets no appearance.
//! - **A face given a colour of its own** (`Document::faces`, by the
//!   feature whose body has it and its name there) has it, before its
//!   body's at that feature; the faces later features make of it (copies
//!   through a boolean, a pattern, a transform, a blend; the pieces of a
//!   face cut in two, which share its name) find it on the same way back,
//!   which looks at each step for an entry of the face's name in that
//!   step's body before the body's own appearance. A face's own colour
//!   anywhere on its way wins over every body colour on it, on whichever
//!   feature of the chain that is given (the developer's rule of
//!   2026-11-10, J-040; [`face_given`]); among body colours the nearest
//!   feature on the way wins.
//!
//! - **A feature that is off** (suppressed, or inactive by its condition)
//!   and passes a body on gives that body its appearance, material and
//!   face colours only where the body has none of its own (decided by the
//!   developer, 2026-10-26; docs/design/regeneration.md › *Switched
//!   off*). Each is found twice: first with the entries of the features
//!   that are on alone, as if those that are off were not in the
//!   history, so a red part moved by an inactive transform given blue
//!   stays red (R1 item 23); only when that finds nothing are the entries
//!   of the features that are off read too, the nearest on the same way
//!   first, so a material given to a pattern that is switched off still
//!   weighs the body the pattern passes on (the CubeSat's spacers). The
//!   colour and the material are found apart: a body can have the colour
//!   of its source and the material of the feature that is off.
//!
//! - **A colour that follows the variables** (`BodyColor::Exprs`) is
//!   evaluated when it is read, against the document's variables found
//!   once per command ([`Values`]). An entry whose colour does not
//!   evaluate (a variable it uses was deleted) decides the walk all the
//!   same: the body has no colour from it, and none from an earlier body
//!   or from a feature that is off, so a broken colour shows as the
//!   theme's, never silently as another entry's; `document.check` and
//!   `document.inspect` (`colorError`) say why (decided 2026-10-26).
//!
//! Neither changes regeneration: a colour is not part of the history.

use craft3d_doc::{
    Appearance, BodyColor, BooleanOp, Document, Feature, FeatureId, FeatureKind, Material,
    NameRole, PersistentName,
};
use craft3d_expr::Evaluation;
use craft3d_regen::Regen;
use craft3d_render::studio::layers::{Optics as RenderOptics, TangentField};
use craft3d_topo::Body;
use std::cell::OnceCell;
use std::collections::BTreeMap;

/// The document's variables and tables as expressions are evaluated
/// against them; none when its table does not build (a hand-edited file).
pub(crate) fn evaluation(doc: &Document) -> Option<Evaluation> {
    doc.variable_table().ok().map(|table| table.evaluate())
}

/// The document and its variables, evaluated the first time a colour or
/// a density that follows them is read: make one per command and pass it
/// to the `_in` functions, so the variables are evaluated once however
/// many bodies and faces are coloured or weighed. A document without such
/// a colour or density never evaluates them here.
pub struct Values<'d> {
    doc: &'d Document,
    values: OnceCell<Option<Evaluation>>,
}

impl<'d> Values<'d> {
    pub fn new(doc: &'d Document) -> Self {
        Values {
            doc,
            values: OnceCell::new(),
        }
    }

    /// The document they are of.
    pub fn doc(&self) -> &'d Document {
        self.doc
    }

    /// The variables, evaluated once.
    pub fn get(&self) -> Option<&Evaluation> {
        self.values.get_or_init(|| evaluation(self.doc)).as_ref()
    }
}

/// The density of `material` in kg/m³, its expression evaluated against
/// the document's variables: why not, when it does not evaluate.
pub fn density_in(values: &Values, material: &Material) -> Result<f64, String> {
    if material.density_expr.is_none() {
        return Ok(material.density);
    }
    material.density_with(values.get())
}

/// [`density_in`] for one material.
pub fn density(doc: &Document, material: &Material) -> Result<f64, String> {
    density_in(&Values::new(doc), material)
}

/// Why the colour or the density given to the body of a feature does not
/// evaluate, for each that does not: for `document.check`.
pub(crate) fn property_errors(
    doc: &Document,
    values: Option<&Evaluation>,
) -> Vec<(FeatureId, String)> {
    let mut out = Vec::new();
    for entry in &doc.bodies {
        if let Some(appearance) = &entry.appearance
            && matches!(appearance.color, BodyColor::Exprs(_))
            && let Err(why) = appearance.resolve(values)
        {
            out.push((entry.feature, why));
        }
        if let Some(material) = &entry.material
            && let Err(why) = material.density_with(values)
        {
            out.push((
                entry.feature,
                format!("material `{}`: {why}", material.name),
            ));
        }
    }
    out
}

/// What the walks below ask of the document and the regeneration, by
/// feature, found once per walk: each is a function of the document and
/// the regeneration, so what is remembered is what asking again gives.
/// A body of many faces asks the same questions for every face (a scene
/// colours tens of thousands of faces), and each asks for a feature's
/// result and definition, which the regeneration and the document find
/// by a search through the features.
#[derive(Default)]
struct Memo<'v> {
    /// Whether the entries of features that are off are read (the second
    /// look, see the module documentation).
    with_off: bool,
    /// The variables, for a colour that follows them.
    values: Option<&'v Values<'v>>,
    /// [`sources`] of a feature.
    sources: BTreeMap<FeatureId, Vec<FeatureId>>,
    /// Whether a feature's definition is a subtraction, for
    /// [`keeps_material`].
    subtracts: BTreeMap<FeatureId, bool>,
    /// [`body_appearance_with`] of a feature: what decided its colour in
    /// this look, if anything did.
    bodies: BTreeMap<FeatureId, Decided>,
}

/// What a look found for a body's colour: `None` when nothing on the way
/// decides it, `Some(None)` when an entry does but its colour does not
/// evaluate, `Some(Some(appearance))` when an entry gives it.
type Decided = Option<Option<Appearance>>;

/// The definition of `feature` as it was built (with its references
/// followed to the newest body, when they were), else as it stands.
fn built<'a>(doc: &'a Document, regen: &'a Regen, feature: FeatureId) -> Option<&'a Feature> {
    regen
        .feature(feature)
        .and_then(|result| result.followed.as_deref())
        .or_else(|| doc.feature(feature))
}

/// The features whose bodies `feature` was made from, in instance order
/// (for a boolean the target, then the tool), as it was built.
fn sources(doc: &Document, regen: &Regen, memo: &mut Memo, feature: FeatureId) -> Vec<FeatureId> {
    if let Some(found) = memo.sources.get(&feature) {
        return found.clone();
    }
    // One that passes a body on is made from that body alone, in every
    // instance its names are copies in.
    let passed = regen
        .feature(feature)
        .and_then(|r| r.passes_on())
        .map(|source| vec![source]);
    let found: Vec<FeatureId> = passed
        .or_else(|| {
            built(doc, regen, feature)
                .map(|f| f.kind.sources().into_iter().map(|(id, _)| id).collect())
        })
        .unwrap_or_default();
    memo.sources.insert(feature, found.clone());
    found
}

/// The feature whose body is instance `instance` of `feature`'s: the
/// source of that number (a boolean's tool for 1), or the one source of
/// a feature that repeats one body.
fn source_of(
    doc: &Document,
    regen: &Regen,
    memo: &mut Memo,
    feature: FeatureId,
    instance: u64,
) -> Option<FeatureId> {
    let sources = sources(doc, regen, memo, feature);
    usize::try_from(instance)
        .ok()
        .and_then(|i| sources.get(i))
        .or_else(|| sources.first())
        .copied()
}

/// Whether the result of `feature` keeps the material of its instance
/// `instance`, so that a face that came through it still bounds that
/// body's material: every instance of every feature but the tools of a
/// subtraction (whose faces bound the target's material, inside out).
fn keeps_material(
    doc: &Document,
    regen: &Regen,
    memo: &mut Memo,
    feature: FeatureId,
    instance: u64,
) -> bool {
    let subtracts = match memo.subtracts.get(&feature) {
        Some(&subtracts) => subtracts,
        None => {
            let subtracts = matches!(
                built(doc, regen, feature).map(|f| &f.kind),
                Some(FeatureKind::Boolean(b)) if b.op == BooleanOp::Subtract
            );
            memo.subtracts.insert(feature, subtracts);
            subtracts
        }
    };
    !subtracts || instance == 0
}

/// The first of what `pick` finds on the way from `feature` back through
/// the bodies each was made from as instance 0, `feature` itself first.
fn along_instance_zero<T>(
    doc: &Document,
    regen: &Regen,
    memo: &mut Memo,
    feature: FeatureId,
    pick: impl Fn(FeatureId) -> Option<T>,
) -> Option<T> {
    let mut at = feature;
    // Each step goes to an earlier feature; the cap is for a bad file.
    for _ in 0..=doc.features.len() {
        if let Some(found) = pick(at) {
            return Some(found);
        }
        at = source_of(doc, regen, memo, at, 0)?;
    }
    None
}

/// True when `feature` is switched off: what it was given applies only
/// where nothing that is on gives one.
fn off(regen: &Regen, feature: FeatureId) -> bool {
    regen.feature(feature).is_some_and(|r| r.is_off())
}

impl<'v> Memo<'v> {
    /// The memo of the first look, which reads the entries of features
    /// that are on.
    fn first(values: &'v Values<'v>) -> Self {
        Memo {
            values: Some(values),
            ..Memo::default()
        }
    }

    /// The memo of the second look, which reads the entries of features
    /// that are off too.
    fn with_off(values: &'v Values<'v>) -> Self {
        Memo {
            with_off: true,
            values: Some(values),
            ..Memo::default()
        }
    }

    /// True when the entries of `feature` are not read in this look.
    fn skips(&self, regen: &Regen, feature: FeatureId) -> bool {
        !self.with_off && off(regen, feature)
    }
}

/// True when some feature that is off has an entry (a colour, a material
/// or a face colour): only then can the second look find anything. One
/// pass over the features and two binary searches for each that is off,
/// so a document with nothing off, or with many face colours (a STEP
/// import), pays next to nothing.
fn any_off_entry(doc: &Document, regen: &Regen) -> bool {
    regen
        .features
        .iter()
        .filter(|r| r.is_off())
        .any(|r| doc.body_properties(r.id).is_some() || !doc.face_properties(r.id).is_empty())
}

/// The appearance given to `feature` itself, when it is read in this
/// look (`with_off`: also when the feature is off): `Some(None)` when it
/// has one whose colour does not evaluate ([`Decided`]).
fn own_appearance(
    doc: &Document,
    regen: &Regen,
    (with_off, values): (bool, Option<&Values>),
    feature: FeatureId,
) -> Decided {
    if !with_off && off(regen, feature) {
        return None;
    }
    let given = doc.body_properties(feature)?.appearance.as_ref()?;
    Some(match given.color {
        BodyColor::Fixed(_) => given.resolve(None).ok(),
        BodyColor::Exprs(_) => given.resolve(values.and_then(Values::get)).ok(),
    })
}

/// What [`own_appearance`] needs of a memo.
fn look<'v>(memo: &Memo<'v>) -> (bool, Option<&'v Values<'v>>) {
    (memo.with_off, memo.values)
}

/// The colour given to a face of the body of `at` whose name there is the
/// copy in `at` along `path` of `of` (`way`), or `name` itself: an entry
/// of that name, else one of another name the same face answers to there
/// ([`colour_by_names`]).
fn own_face_appearance(
    doc: &Document,
    regen: &Regen,
    memo: &Memo,
    at: FeatureId,
    way: Option<(&[u64], &PersistentName)>,
    name: &PersistentName,
) -> Option<Appearance> {
    let entries = doc.face_properties(at);
    if entries.is_empty() || memo.skips(regen, at) {
        return None;
    }
    let direct = match way {
        Some((path, of)) => entries
            .iter()
            .find(|e| {
                e.face.feature == at
                    && e.face.role == NameRole::Copy
                    && e.face.by.is_none()
                    && e.face.sources == path
                    && e.face.of.as_deref() == Some(of)
            })
            .map(|e| e.appearance),
        None => doc.face_appearance(at, name),
    };
    if direct.is_some() {
        return direct;
    }
    // The face may have another name there too (an alias: faces of
    // several instances or operands merged into one).
    let names = regen.feature(at)?.names.as_deref()?;
    if names.aliases.is_empty() {
        return None;
    }
    let here = match way {
        Some((path, of)) => PersistentName {
            of: Some(std::sync::Arc::new(of.clone())),
            ..PersistentName::new(at, NameRole::Copy, path.to_vec())
        },
        None => name.clone(),
    };
    let face = craft3d_regen::faces_named(names, &here).first()?.0;
    colour_by_names(doc, at, names, usize::try_from(face).ok()?)
}

/// The colour given to face `face` of the body of `feature` (whose names
/// are `names`) by any name it answers to: its own name first, then its
/// aliases in their order (the target's or the first instance's before
/// the others'), the first that has one. So a face that several coloured
/// faces were merged into has the colour of the first of them.
fn colour_by_names(
    doc: &Document,
    feature: FeatureId,
    names: &craft3d_regen::Names,
    face: usize,
) -> Option<Appearance> {
    names
        .names_of(face)
        .find_map(|name| doc.face_appearance(feature, name))
}

/// The appearance of the body `feature` makes (see the module
/// documentation); `None` for the default look.
pub fn body_appearance(doc: &Document, regen: &Regen, feature: FeatureId) -> Option<Appearance> {
    body_appearance_in(&Values::new(doc), regen, feature)
}

/// [`body_appearance`] with the document's variables found once for the
/// command ([`Values`]).
pub fn body_appearance_in(
    values: &Values,
    regen: &Regen,
    feature: FeatureId,
) -> Option<Appearance> {
    let doc = values.doc;
    match body_appearance_with(doc, regen, &mut Memo::first(values), feature) {
        Some(decided) => decided,
        None if any_off_entry(doc, regen) => {
            body_appearance_with(doc, regen, &mut Memo::with_off(values), feature).flatten()
        }
        None => None,
    }
}

fn body_appearance_with(
    doc: &Document,
    regen: &Regen,
    memo: &mut Memo,
    feature: FeatureId,
) -> Decided {
    if let Some(&found) = memo.bodies.get(&feature) {
        return found;
    }
    let how = (memo.with_off, memo.values);
    let found = along_instance_zero(doc, regen, memo, feature, |at| {
        own_appearance(doc, regen, how, at)
    });
    memo.bodies.insert(feature, found);
    found
}

/// The material of the body `feature` makes, and the feature it was given
/// to; `None` when it has none.
pub fn body_material(
    doc: &Document,
    regen: &Regen,
    feature: FeatureId,
) -> Option<(FeatureId, Material)> {
    let look = |with_off: bool| {
        let mut memo = Memo {
            with_off,
            ..Memo::default()
        };
        along_instance_zero(doc, regen, &mut memo, feature, |at| {
            if !with_off && off(regen, at) {
                return None;
            }
            Some((at, doc.body_properties(at)?.material.clone()?))
        })
    };
    look(false).or_else(|| {
        if any_off_entry(doc, regen) {
            look(true)
        } else {
            None
        }
    })
}

/// The appearance of one face of the body of `feature`, named `name` there
/// (see the module documentation), when its way meets one; the caller
/// falls back to the body's.
fn face_appearance(
    doc: &Document,
    regen: &Regen,
    memo: &mut Memo,
    feature: FeatureId,
    name: &PersistentName,
) -> Option<Appearance> {
    walk_face(doc, regen, memo, feature, name, false)
}

/// The colour given to the face itself (`face.appearance`) anywhere on its
/// way back, whatever body colours lie on the way: a face's own colour
/// wins over a body's on whichever feature of the chain that is given
/// (J-040: a body colour given to a later transform hid a face colour
/// given after the extrude). `None` when the way meets none.
fn face_given(
    doc: &Document,
    regen: &Regen,
    memo: &mut Memo,
    feature: FeatureId,
    name: &PersistentName,
) -> Option<Appearance> {
    walk_face(doc, regen, memo, feature, name, true)
}

/// The walk of [`face_appearance`]; with `faces_only` the colours given to
/// the face alone, past every body colour on the way, `None` where the way
/// ends.
fn walk_face(
    doc: &Document,
    regen: &Regen,
    memo: &mut Memo,
    feature: FeatureId,
    name: &PersistentName,
    faces_only: bool,
) -> Option<Appearance> {
    if doc.bodies.is_empty() && doc.faces.is_empty() {
        return None;
    }
    if faces_only && doc.faces.is_empty() {
        return None;
    }
    // Where the way ends: the body's colour there, or none when only the
    // face's own colour is looked for.
    let body_at = |memo: &mut Memo, at: FeatureId| {
        if faces_only {
            None
        } else {
            body_appearance_with(doc, regen, memo, at).flatten()
        }
    };
    // Where the way stands: a feature, and what is left of the way from
    // its body, as the instances to go through and the name at the end.
    let mut at = feature;
    let mut way: Option<(Vec<u64>, &PersistentName)> = None;
    let mut current = name;
    for _ in 0..=doc.features.len().saturating_mul(2) {
        let here = way.as_ref().map(|(path, of)| (path.as_slice(), *of));
        if let Some(own) = own_face_appearance(doc, regen, memo, at, here, current) {
            return Some(own);
        }
        if !faces_only && let Some(own) = own_appearance(doc, regen, look(memo), at) {
            return own;
        }
        if way.is_none() {
            if current.role == NameRole::Copy
                && current.feature == at
                && let Some(of) = current.of.as_deref()
            {
                way = Some((current.sources.clone(), of));
            } else {
                // Made here (or named in a way that does not say): the
                // body's colour.
                return body_at(memo, at);
            }
        }
        let Some((path, of)) = way.take() else {
            return body_at(memo, at);
        };
        let Some((&first, rest)) = path.split_first() else {
            return body_at(memo, at);
        };
        if !keeps_material(doc, regen, memo, at, first) {
            // Cut by a tool: the face bounds this body's material.
            return body_at(memo, at);
        }
        let Some(next) = source_of(doc, regen, memo, at, first) else {
            return body_at(memo, at);
        };
        at = next;
        if path == [0] && next != of.feature {
            // Instance 0 on and on, until the feature the name after `/`
            // is of.
            way = Some((path, of));
        } else if !rest.is_empty() {
            way = Some((rest.to_vec(), of));
        } else if next != of.feature && !(of.role == NameRole::Copy && of.by.is_none()) {
            // Other instances, then instance 0 on and on down to a name
            // that is no copy (`PersistentName::copy` leaves that run out).
            way = Some((vec![0], of));
        } else {
            current = of;
        }
    }
    body_at(memo, feature)
}

/// The appearance of every face of the body of `feature`, in the order of
/// its faces: the body's for every face when the faces have no names.
pub fn face_appearances(
    doc: &Document,
    regen: &Regen,
    feature: FeatureId,
    faces: usize,
) -> Vec<Option<Appearance>> {
    face_appearances_in(&Values::new(doc), regen, feature, faces)
}

/// [`face_appearances`] with the document's variables found once for the
/// command ([`Values`]).
pub fn face_appearances_in(
    values: &Values,
    regen: &Regen,
    feature: FeatureId,
    faces: usize,
) -> Vec<Option<Appearance>> {
    let doc = values.doc;
    if doc.bodies.is_empty() && doc.faces.is_empty() {
        return vec![None; faces];
    }
    let mut first = Memo::first(values);
    // A colour of the body's own way that does not evaluate decides it:
    // no second look fills its faces in from a feature that is off.
    let broken = matches!(
        body_appearance_with(doc, regen, &mut first, feature),
        Some(None)
    );
    let mut found = face_appearances_with(doc, regen, &mut first, feature, faces);
    // The faces nothing that is on gives a colour: the second look.
    if !broken && found.iter().any(Option::is_none) && any_off_entry(doc, regen) {
        let again = face_appearances_with(doc, regen, &mut Memo::with_off(values), feature, faces);
        for (face, second) in found.iter_mut().zip(again) {
            if face.is_none() {
                *face = second;
            }
        }
    }
    found
}

fn face_appearances_with(
    doc: &Document,
    regen: &Regen,
    memo: &mut Memo,
    feature: FeatureId,
    faces: usize,
) -> Vec<Option<Appearance>> {
    let body = body_appearance_with(doc, regen, memo, feature).flatten();
    let all = regen
        .feature(feature)
        .and_then(|result| result.names.as_deref());
    let names = all.map(|names| names.faces.as_slice()).unwrap_or_default();
    let has_entries = !doc.face_properties(feature).is_empty() && !memo.skips(regen, feature);
    // A face whose name does not say it came in through an instance of
    // this feature has the body's colour, as the walk would find at its
    // first step (and when the feature has an appearance of its own,
    // every face has that: the body's).
    let walks = own_appearance(doc, regen, look(memo), feature).is_none();
    (0..faces)
        .map(|face| match names.get(face) {
            Some(name) => {
                let own = if has_entries {
                    all.and_then(|all| colour_by_names(doc, feature, all, face))
                } else {
                    None
                };
                let copy = name.role == NameRole::Copy && name.feature == feature;
                if own.is_some() {
                    own
                } else if let Some(given) = copy
                    .then(|| face_given(doc, regen, memo, feature, name))
                    .flatten()
                {
                    // Its own colour, given at an earlier feature, wins
                    // over the body colours on its way.
                    Some(given)
                } else if walks && copy {
                    face_appearance(doc, regen, memo, feature, name).or(body)
                } else {
                    body
                }
            }
            None => body,
        })
        .collect()
}

/// How an appearance's metallic and roughness are drawn.
pub(crate) fn finish(appearance: &craft3d_doc::Appearance) -> craft3d_render::Finish {
    craft3d_render::Finish::from_factors(appearance.metallic, appearance.roughness)
}

/// Model units of `doc` per millimetre.
pub(crate) fn units_per_mm(doc: &Document) -> f64 {
    1e-3 / crate::appearance_cmds::metres_per_unit(doc.units.length)
}

/// The direction of an anisotropic face's lobe from its surface's own
/// parametrisation: the `u` direction, which is a plane's first axis and
/// the turn around the axis of a cylinder, cone, sphere or torus. A
/// freeform face (Bézier, NURBS) takes its `u` direction at the middle of
/// its domain as a fixed axis (an approximation: per-point tangents from
/// the tessellator's parameters would follow it everywhere).
pub(crate) fn tangent_field(surface: &craft3d_geom::Surface) -> TangentField {
    use craft3d_geom::Surface;
    let narrow = |v: craft3d_math::Vec3| [v.x as f32, v.y as f32, v.z as f32];
    let around = |frame: &craft3d_math::Frame| TangentField::Around {
        origin: [frame.origin().x, frame.origin().y, frame.origin().z],
        axis: narrow(frame.z()),
    };
    match surface {
        Surface::Plane(p) => TangentField::Axis(narrow(p.frame().x())),
        Surface::Cylinder(c) => around(c.frame()),
        Surface::Cone(c) => around(c.frame()),
        Surface::Sphere(s) => around(s.frame()),
        Surface::Torus(t) => around(t.frame()),
        Surface::Bezier(_) | Surface::Nurbs(_) => {
            let ((u0, u1), (v0, v1)) = surface.domain();
            let (_, du, _) = surface.derivatives(0.5 * (u0 + u1), 0.5 * (v0 + v1));
            TangentField::Axis(narrow(du))
        }
    }
}

/// The layers of `appearance` as the renderers take them for the face
/// `face` of `body` (`None` for none), the absorption's length converted
/// from millimetres with `per_mm` model units per millimetre.
pub(crate) fn render_optics(
    appearance: &craft3d_doc::Appearance,
    body: Option<&Body>,
    face: usize,
    per_mm: f64,
) -> Option<RenderOptics> {
    let o = &appearance.optics;
    if o.is_none() {
        return None;
    }
    let pattern = o
        .pattern
        .as_ref()
        .map(|t| crate::solid_texture::render_pattern(t, body));
    let tangent = match (o.direction, pattern) {
        // Layer 3.5: the anisotropy along the texture's axis (wood's grain,
        // a brushing direction).
        (_, Some(pt)) if o.pattern.is_some_and(|t| t.along_axis) => TangentField::Axis(pt.axis),
        (craft3d_doc::AnisotropyDirection::Axis(a), _) => {
            TangentField::Axis([a[0] as f32, a[1] as f32, a[2] as f32])
        }
        (craft3d_doc::AnisotropyDirection::Parametric, _) => body
            .and_then(|b| b.faces.get(face))
            .map_or(TangentField::Axis([1.0, 0.0, 0.0]), |f| {
                tangent_field(&f.surface)
            }),
    };
    let absorption = o.absorption.map_or([0.0; 3], |c| {
        craft3d_render::studio::layers::absorption_coefficients(c, o.absorption_distance * per_mm)
    });
    Some(
        RenderOptics {
            anisotropy: o.anisotropy as f32,
            tangent,
            clearcoat: o.clearcoat as f32,
            clearcoat_roughness: o.clearcoat_roughness as f32,
            transmission: o.transmission as f32,
            ior: o.ior as f32,
            thin_walled: o.thin_walled,
            absorption,
            subsurface: o.subsurface as f32,
            subsurface_color: o.subsurface_color,
            pattern,
        }
        .sanitized(),
    )
}

/// A face's look as the renderers take it: colour, finish and layers.
/// Only the replay's pictures take it, which the browser build has not.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) type FaceLook = ([u8; 3], craft3d_render::Finish, Option<RenderOptics>);

/// The looks of the faces of `body` (the body of `feature`), by face.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn face_looks(
    doc: &Document,
    regen: &Regen,
    feature: FeatureId,
    body: &Body,
    forced: Option<&Appearance>,
) -> Vec<Option<FaceLook>> {
    let per_mm = units_per_mm(doc);
    let looks = match forced {
        Some(forced) => vec![Some(*forced); body.faces.len()],
        None => face_appearances(doc, regen, feature, body.faces.len()),
    };
    looks
        .into_iter()
        .enumerate()
        .map(|(face, a)| {
            a.map(|a| {
                (
                    a.color,
                    finish(&a),
                    render_optics(&a, Some(body), face, per_mm),
                )
            })
        })
        .collect()
}

/// Gives the faces of the body of `feature`, whose ids in a scene's or a
/// picture's mesh start at `first_face_id`, their colours in `colors`;
/// every face `forced` when it is given (a body the view draws in the
/// colour of a collision, after the colours were resolved: never through
/// the document's entries, which other bodies walk back to).
pub(crate) fn color_faces(
    colors: &mut craft3d_render::FaceColors,
    values: &Values,
    regen: &Regen,
    (feature, first_face_id, faces): (FeatureId, u32, usize),
    forced: Option<&Appearance>,
) {
    let body = regen
        .feature(feature)
        .and_then(|result| result.body.as_deref());
    let per_mm = units_per_mm(values.doc());
    let looks = match forced {
        Some(forced) => vec![Some(*forced); faces],
        None => face_appearances_in(values, regen, feature, faces),
    };
    for (face, appearance) in looks.into_iter().enumerate() {
        if let (Some(appearance), Ok(index)) = (appearance, u32::try_from(face))
            && let Some(id) = first_face_id.checked_add(index)
        {
            colors.set_look(id, Some((appearance.color, finish(&appearance))));
            colors.set_optics(id, render_optics(&appearance, body, face, per_mm));
        }
    }
}

/// A face colour whose face is not in its body: the feature built a body
/// with named faces and none of them is called that (by its own name or
/// an alias). It is kept (undoing the edit that took the face brings the
/// colour back) and colours nothing; never another face.
#[derive(Clone, Debug, PartialEq)]
pub struct LostFace {
    pub feature: FeatureId,
    pub face: PersistentName,
}

/// The face colours of the document whose faces are gone (see
/// [`LostFace`]). One of a feature that did not build (failed,
/// suppressed, rolled back) is not lost: it waits for the body. Nor is
/// one whose face is absent because a feature that is off would make it
/// (`craft3d_regen::absent_name`: the tool's face of a subtraction that
/// is inactive, an instance of a pattern that is suppressed): it waits
/// for that feature to be switched on, as the rule for what is off says
/// (docs/design/regeneration.md › *Switched off*).
pub fn lost_faces(doc: &Document, regen: &Regen) -> Vec<LostFace> {
    let mut lost = Vec::new();
    let mut at = 0;
    while let Some(first) = doc.faces.get(at) {
        let feature = first.feature;
        let entries = doc.face_properties(feature);
        at = at.saturating_add(entries.len().max(1));
        let Some(names) = regen
            .feature(feature)
            .filter(|result| result.body.is_some())
            .and_then(|result| result.names.as_deref())
        else {
            continue;
        };
        for entry in entries {
            if craft3d_regen::faces_named(names, &entry.face).is_empty()
                && craft3d_regen::absent_name(doc, &regen.features, &entry.face).is_none()
            {
                lost.push(LostFace {
                    feature,
                    face: entry.face.clone(),
                });
            }
        }
    }
    lost
}
