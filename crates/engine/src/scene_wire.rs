//! A [`Scene`] written out as bytes and read back, bit for bit: how the
//! scene a Web Worker regenerated and meshed crosses to the page
//! (`regen_elsewhere.rs`; docs/design/engine.md › *Regeneration in a Web
//! Worker*).
//!
//! The form is compact, deterministic and little-endian: every number as
//! its bits (an `f64` as its eight bytes, so it reads back as the same
//! `f64`, a NaN and a negative zero included), every list with its count
//! first, every text as UTF-8 with its length. The same scene always gives
//! the same bytes. What a scene shares by `Arc` is written once where it
//! is: the meshes of its bodies, and the scene's own mesh only when it is
//! not the bodies' meshes one after the other (as `scene::build` makes
//! it), which is then made again on reading.
//!
//! Reading never panics, and what it makes is bounded by the bytes it is
//! given: every count is checked against the bytes left before anything
//! is made of it (each entry takes at least its least size in bytes), no
//! list is given more room up front than the bytes left would fill, and
//! a face colour must name a face of a body read before it, those bodies
//! numbered as `scene::build` numbers them (so the colours, which are
//! kept by face id, take no more room than the bodies' faces). What
//! reads is a scene the page can draw: every mesh holds a normal per
//! position, a face id per triangle and only corners it has, and its
//! face ids lie within its body's faces. A feature's kind is the page's
//! own (the kind of the feature of that id in the document the scene is
//! read for), so the scene of another document does not read.

use crate::meshes::{BodyFacts, EdgePolylines};
use crate::scene::{
    FeatureState, Scene, SceneBody, SceneDatum, SceneDatumShape, SceneFeature, ScenePolyline,
    SceneSketch, SketchCurve,
};
use craft3d_doc::Document;
use craft3d_math::{Bbox3, Vec3};
use craft3d_render::studio::layers::{Optics, TangentField};
use craft3d_render::studio::pattern::{Pattern, PatternKind};
use craft3d_render::{FaceColors, Finish};
use craft3d_tess::Mesh;
use std::collections::BTreeMap;
use std::sync::Arc;

/// What the bytes of a scene start with, with the form's version.
const MAGIC: &[u8; 8] = b"C3DSCN\x00\x04";

/// The scene's bytes. The revision is the page's own and is left out.
pub(crate) fn encode(scene: &Scene) -> Vec<u8> {
    let mut w = Writer::default();
    w.bytes(MAGIC);
    w.f64(scene.tolerance);
    w.opt_f64(scene.max_angle);
    w.bool(scene.coarsened);
    w.bool(scene.preview);
    w.len(scene.bodies.len());
    for body in scene.bodies.iter() {
        w.text(&body.feature);
        w.text(&body.name);
        w.mesh(&body.mesh);
        w.u32(body.first_face_id);
        w.lines(&body.edges);
        w.facts(&body.facts);
    }
    if concatenated(&scene.bodies).is_some_and(|mesh| same_mesh(&mesh, &scene.mesh)) {
        w.u8(0);
    } else {
        w.u8(1);
        w.mesh(&scene.mesh);
    }
    let looks: Vec<(u32, ([u8; 3], Finish))> = scene.face_colors.looks().collect();
    w.len(looks.len());
    for (face, (rgb, finish)) in looks {
        w.u32(face);
        w.bytes(&rgb);
        w.u8(finish.metallic);
        match finish.roughness {
            Some(r) => {
                w.u8(1);
                w.u8(r);
            }
            None => w.u8(0),
        }
    }
    let optics: Vec<(u32, &Optics)> = scene.face_colors.optics_faces().collect();
    w.len(optics.len());
    for (face, o) in optics {
        w.u32(face);
        w.optics(o);
    }
    w.len(scene.sketches.len());
    for sketch in &scene.sketches {
        w.text(&sketch.feature);
        w.text(&sketch.name);
        for v in [sketch.origin, sketch.x_axis, sketch.y_axis, sketch.normal] {
            w.vec3(v);
        }
        w.len(sketch.polylines.len());
        for line in &sketch.polylines {
            w.text(&line.entity);
            w.u8(match line.curve {
                SketchCurve::Line => 0,
                SketchCurve::Circle => 1,
                SketchCurve::Arc => 2,
                SketchCurve::Spline => 3,
            });
            w.bool(line.construction);
            w.points(&line.points);
        }
        w.points(&sketch.points);
    }
    w.len(scene.datums.len());
    for datum in &scene.datums {
        w.text(&datum.feature);
        w.text(&datum.name);
        match datum.shape {
            SceneDatumShape::Plane {
                center,
                u,
                v,
                origin,
                normal,
            } => {
                w.u8(0);
                for p in [center, u, v, origin, normal] {
                    w.vec3(p);
                }
            }
            SceneDatumShape::Axis { from, to } => {
                w.u8(1);
                w.vec3(from);
                w.vec3(to);
            }
            SceneDatumShape::Point(p) => {
                w.u8(2);
                w.vec3(p);
            }
        }
    }
    w.len(scene.features.len());
    for f in &scene.features {
        w.text(&f.id);
        w.text(&f.name);
        w.text(f.kind);
        w.u8(match f.state {
            FeatureState::Ok => 0,
            FeatureState::Suppressed => 1,
            FeatureState::Inactive => 2,
            FeatureState::RolledBack => 3,
            FeatureState::Failed => 4,
        });
        w.opt_text(f.error.as_deref());
        match f.dof {
            Some(dof) => {
                w.u8(1);
                w.u64(dof as u64);
            }
            None => w.u8(0),
        }
        w.bool(f.is_sketch);
        w.opt_text(f.caused_by.as_deref());
        w.bool(f.hidden);
        w.bool(f.draws);
        w.opt_text(f.when.as_deref());
        w.opt_text(f.passes.as_deref());
    }
    w.texts(&scene.problems);
    w.texts(&scene.notices);
    w.bbox(&scene.bbox);
    w.bbox(&scene.bodies_box);
    w.out
}

/// The scene `bytes` hold ([`encode`]), for `doc`, under `revision`. An
/// error says why the bytes are not a scene of `doc`.
pub(crate) fn decode(bytes: &[u8], doc: &Document, revision: u64) -> Result<Scene, String> {
    let mut r = Reader { bytes, at: 0 };
    if r.take(MAGIC.len())? != MAGIC {
        return Err("not a scene of this version".into());
    }
    let tolerance = r.f64()?;
    let max_angle = r.opt_f64()?;
    let coarsened = r.bool()?;
    let preview = r.bool()?;
    let count = r.count(BODY)?;
    let mut bodies = Vec::with_capacity(r.room::<SceneBody>(count));
    for _ in 0..count {
        bodies.push(SceneBody {
            feature: r.text()?,
            name: r.text()?,
            mesh: Arc::new(r.mesh()?),
            first_face_id: r.u32()?,
            edges: Arc::new(r.lines()?),
            facts: Arc::new(r.facts()?),
            // Not written: the page uploads what it reads as it is.
            placement: None,
        });
    }
    let face_end = numbered(&bodies)?;
    let mesh = match r.u8()? {
        0 => concatenated(&bodies).ok_or("the scene's bodies cannot be joined into one mesh")?,
        1 => r.mesh()?,
        _ => return Err("the scene's mesh is not written as a mesh is".into()),
    };
    let mut face_colors = FaceColors::new();
    for _ in 0..r.count(FACE_COLOR)? {
        let face = r.u32()?;
        if face >= face_end {
            return Err("a face colour of the scene is of no face of its bodies".into());
        }
        let rgb = [r.u8()?, r.u8()?, r.u8()?];
        let metallic = r.u8()?;
        let roughness = if r.bool()? { Some(r.u8()?) } else { None };
        if !face_colors.set_look(
            face,
            Some((
                rgb,
                Finish {
                    metallic,
                    roughness,
                },
            )),
        ) {
            return Err("a face colour of the scene is out of range".into());
        }
    }
    for _ in 0..r.count(OPTICS)? {
        let face = r.u32()?;
        if face >= face_end {
            return Err("a face's layers of the scene are of no face of its bodies".into());
        }
        let optics = r.optics()?;
        if !face_colors.set_optics(face, Some(optics)) {
            return Err("a face's layers of the scene are out of range".into());
        }
    }
    let count = r.count(SKETCH)?;
    let mut sketches = Vec::with_capacity(r.room::<SceneSketch>(count));
    for _ in 0..count {
        let feature = r.text()?;
        let name = r.text()?;
        let (origin, x_axis, y_axis, normal) = (r.vec3()?, r.vec3()?, r.vec3()?, r.vec3()?);
        let lines = r.count(POLYLINE)?;
        let mut polylines = Vec::with_capacity(r.room::<ScenePolyline>(lines));
        for _ in 0..lines {
            let entity = r.text()?;
            let curve = match r.u8()? {
                0 => SketchCurve::Line,
                1 => SketchCurve::Circle,
                2 => SketchCurve::Arc,
                3 => SketchCurve::Spline,
                _ => return Err("a sketch curve of the scene is of no known kind".into()),
            };
            polylines.push(ScenePolyline {
                entity,
                curve,
                construction: r.bool()?,
                points: r.points()?,
            });
        }
        sketches.push(SceneSketch {
            feature,
            name,
            origin,
            x_axis,
            y_axis,
            normal,
            polylines,
            points: r.points()?,
        });
    }
    let count = r.count(DATUM)?;
    let mut datums = Vec::with_capacity(r.room::<SceneDatum>(count));
    for _ in 0..count {
        let feature = r.text()?;
        let name = r.text()?;
        let shape = match r.u8()? {
            0 => SceneDatumShape::Plane {
                center: r.vec3()?,
                u: r.vec3()?,
                v: r.vec3()?,
                origin: r.vec3()?,
                normal: r.vec3()?,
            },
            1 => SceneDatumShape::Axis {
                from: r.vec3()?,
                to: r.vec3()?,
            },
            2 => SceneDatumShape::Point(r.vec3()?),
            _ => return Err("a datum of the scene is of no known shape".into()),
        };
        datums.push(SceneDatum {
            feature,
            name,
            shape,
        });
    }
    // The kinds, as the page's document has them.
    let kinds: BTreeMap<String, &'static str> = doc
        .features
        .iter()
        .map(|f| (f.id.to_string(), f.kind.type_name()))
        .collect();
    let count = r.count(FEATURE)?;
    let mut features = Vec::with_capacity(r.room::<SceneFeature>(count));
    for _ in 0..count {
        let id = r.text()?;
        let name = r.text()?;
        let written = r.text()?;
        let kind = kinds
            .get(&id)
            .copied()
            .filter(|k| *k == written)
            .ok_or_else(|| format!("the scene's feature {id} is not this document's"))?;
        let state = match r.u8()? {
            0 => FeatureState::Ok,
            1 => FeatureState::Suppressed,
            2 => FeatureState::Inactive,
            3 => FeatureState::RolledBack,
            4 => FeatureState::Failed,
            _ => return Err("a feature of the scene is in no known state".into()),
        };
        let error = r.opt_text()?;
        let dof = if r.bool()? {
            Some(usize::try_from(r.u64()?).map_err(|_| "a sketch's freedom is out of range")?)
        } else {
            None
        };
        features.push(SceneFeature {
            id,
            name,
            kind,
            state,
            error,
            dof,
            is_sketch: r.bool()?,
            caused_by: r.opt_text()?,
            hidden: r.bool()?,
            draws: r.bool()?,
            when: r.opt_text()?,
            passes: r.opt_text()?,
        });
    }
    let problems = r.texts()?;
    let notices = r.texts()?;
    let bbox = r.bbox()?;
    let bodies_box = r.bbox()?;
    if r.at != bytes.len() {
        return Err("the scene has bytes past its end".into());
    }
    Ok(Scene {
        revision,
        tolerance,
        max_angle,
        coarsened,
        preview,
        bodies: Arc::new(bodies),
        mesh: Arc::new(mesh),
        face_colors: Arc::new(face_colors),
        sketches,
        datums,
        features,
        problems,
        notices,
        bbox,
        bodies_box,
    })
}

/// The least bytes of an entry of each list of a scene: what its fixed
/// parts and the counts of its lists take.
/// A body: two texts, a mesh (four counts), its first face id, its edges
/// (a count) and its facts (seven counts).
const BODY: usize = 4 + 4 + 16 + 4 + 4 + 28;
/// A face colour: the face, the colour, the metal, whether it is rough.
const FACE_COLOR: usize = 4 + 3 + 1 + 1;
/// A face's layers: the face, nine `f32`, the tangent field's kind and
/// axis, two flags, the scattering colour's flag and the solid texture's.
const OPTICS: usize = 4 + 9 * 4 + 1 + 12 + 1 + 1 + 1;
/// A sketch: two texts, four vectors and two counts.
const SKETCH: usize = 4 + 4 + 4 * 24 + 4 + 4;
/// A sketch's polyline: a text, its kind, construction and its points.
const POLYLINE: usize = 4 + 1 + 1 + 4;
/// A datum: two texts, its shape and at least one vector.
const DATUM: usize = 4 + 4 + 1 + 24;
/// A feature row: three texts and nine bytes of state and options.
const FEATURE: usize = 4 + 4 + 4 + 9;

/// One past the highest face id a face colour of `bodies` may have: the
/// end of the faces of the bodies, checked to be numbered as
/// `scene::build` numbers them (each body's first id one past the
/// highest of the one before, the first 0) and each body's face ids
/// within its faces. A body has as many faces as it has face names (or
/// lists of aliases), or, without names, no more than it has triangles
/// (every face the tessellator meshed has a triangle at least). An error
/// for bodies that are not numbered so: their colours could take room no
/// face of them has.
fn numbered(bodies: &[SceneBody]) -> Result<u32, String> {
    const WRONG: &str = "the scene's faces are not numbered as a scene numbers them";
    let mut next: u32 = 0;
    let mut end: u32 = 0;
    for body in bodies {
        if body.first_face_id != next {
            return Err(WRONG.into());
        }
        let faces = body
            .facts
            .face_names
            .len()
            .max(body.facts.face_aliases.len())
            .max(body.mesh.triangles.len());
        let faces = u32::try_from(faces).map_err(|_| WRONG)?;
        if let Some(&highest) = body.mesh.face_ids.iter().max() {
            if highest >= faces {
                return Err(WRONG.into());
            }
            next = next
                .checked_add(highest)
                .and_then(|n| n.checked_add(1))
                .ok_or(WRONG)?;
        }
        end = end.max(body.first_face_id.checked_add(faces).ok_or(WRONG)?);
    }
    Ok(end)
}

/// The bodies' meshes one after the other, as `scene::build` joins them:
/// vertex indices moved past the vertices before, face ids from each
/// body's first. `None` when the vertices do not fit the indices.
fn concatenated(bodies: &[SceneBody]) -> Option<Mesh> {
    let mut all = Mesh::default();
    for body in bodies {
        let base = u32::try_from(all.positions.len()).ok()?;
        let mesh = &body.mesh;
        all.positions.extend(&mesh.positions);
        all.normals.extend(&mesh.normals);
        all.triangles.extend(
            mesh.triangles
                .iter()
                .map(|t| t.map(|k| k.saturating_add(base))),
        );
        all.face_ids.extend(
            mesh.face_ids
                .iter()
                .map(|f| f.saturating_add(body.first_face_id)),
        );
    }
    Some(all)
}

/// The same meshes, bit for bit.
fn same_mesh(a: &Mesh, b: &Mesh) -> bool {
    let same_points = |p: &[Vec3], q: &[Vec3]| {
        p.len() == q.len()
            && p.iter().zip(q).all(|(a, b)| {
                a.x.to_bits() == b.x.to_bits()
                    && a.y.to_bits() == b.y.to_bits()
                    && a.z.to_bits() == b.z.to_bits()
            })
    };
    same_points(&a.positions, &b.positions)
        && same_points(&a.normals, &b.normals)
        && a.triangles == b.triangles
        && a.face_ids == b.face_ids
}

#[derive(Default)]
struct Writer {
    out: Vec<u8>,
}

impl Writer {
    fn bytes(&mut self, b: &[u8]) {
        self.out.extend_from_slice(b);
    }
    fn f32(&mut self, v: f32) {
        self.u32(v.to_bits());
    }
    /// A face's layers (see [`OPTICS`]).
    fn optics(&mut self, o: &Optics) {
        for v in [
            o.anisotropy,
            o.clearcoat,
            o.clearcoat_roughness,
            o.transmission,
            o.ior,
        ] {
            self.f32(v);
        }
        for v in o.absorption {
            self.f32(v);
        }
        self.f32(o.subsurface);
        match o.tangent {
            TangentField::Axis(axis) => {
                self.u8(0);
                axis.into_iter().for_each(|v| self.f32(v));
            }
            TangentField::Around { origin, axis } => {
                self.u8(1);
                axis.into_iter().for_each(|v| self.f32(v));
                origin.into_iter().for_each(|v| self.f64(v));
            }
        }
        self.bool(o.thin_walled);
        match o.subsurface_color {
            Some(c) => {
                self.u8(1);
                self.bytes(&c);
            }
            None => self.u8(0),
        }
        match &o.pattern {
            Some(t) => {
                // The kind's code is 1 to 7.
                self.u8(t.kind.code() as u8);
                self.u8(t.stops);
                for c in &t.ramp {
                    self.bytes(c);
                }
                match t.roughness {
                    Some([a, b]) => {
                        self.u8(1);
                        self.f32(a);
                        self.f32(b);
                    }
                    None => self.u8(0),
                }
                self.f32(t.scale);
                t.origin.into_iter().for_each(|v| self.f64(v));
                t.axis.into_iter().for_each(|v| self.f32(v));
                self.u32(t.seed);
                self.u32(t.detail);
                for v in [t.distortion, t.grain, t.rays, t.spot] {
                    self.f32(v);
                }
                self.bool(t.wrap);
            }
            None => self.u8(0),
        }
    }
    fn u8(&mut self, v: u8) {
        self.out.push(v);
    }
    fn bool(&mut self, v: bool) {
        self.u8(u8::from(v));
    }
    fn u32(&mut self, v: u32) {
        self.bytes(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.bytes(&v.to_le_bytes());
    }
    /// A count. Lists longer than `u32::MAX` do not occur in a scene (a
    /// mesh's indices are `u32`); one would be written short and fail to
    /// read, never read wrong.
    fn len(&mut self, n: usize) {
        self.u32(u32::try_from(n).unwrap_or(u32::MAX));
    }
    fn f64(&mut self, v: f64) {
        self.u64(v.to_bits());
    }
    fn opt_f64(&mut self, v: Option<f64>) {
        match v {
            Some(v) => {
                self.u8(1);
                self.f64(v);
            }
            None => self.u8(0),
        }
    }
    fn vec3(&mut self, v: Vec3) {
        self.f64(v.x);
        self.f64(v.y);
        self.f64(v.z);
    }
    fn points(&mut self, points: &[Vec3]) {
        self.len(points.len());
        for p in points {
            self.vec3(*p);
        }
    }
    fn lines(&mut self, lines: &EdgePolylines) {
        self.len(lines.len());
        for line in lines {
            self.points(line);
        }
    }
    fn text(&mut self, t: &str) {
        self.len(t.len());
        self.bytes(t.as_bytes());
    }
    fn opt_text(&mut self, t: Option<&str>) {
        match t {
            Some(t) => {
                self.u8(1);
                self.text(t);
            }
            None => self.u8(0),
        }
    }
    fn texts(&mut self, texts: &[String]) {
        self.len(texts.len());
        for t in texts {
            self.text(t);
        }
    }
    fn places(&mut self, lists: &[Vec<usize>]) {
        self.len(lists.len());
        for list in lists {
            self.len(list.len());
            for &k in list {
                self.u64(k as u64);
            }
        }
    }
    fn mesh(&mut self, mesh: &Mesh) {
        self.points(&mesh.positions);
        self.points(&mesh.normals);
        self.len(mesh.triangles.len());
        for t in &mesh.triangles {
            for k in t {
                self.u32(*k);
            }
        }
        self.len(mesh.face_ids.len());
        for f in &mesh.face_ids {
            self.u32(*f);
        }
    }
    fn facts(&mut self, facts: &BodyFacts) {
        self.texts(&facts.face_names);
        self.len(facts.face_aliases.len());
        for aliases in facts.face_aliases.iter() {
            self.texts(aliases);
        }
        self.texts(&facts.edge_names);
        self.places(&facts.edge_faces);
        self.points(&facts.vertices);
        self.texts(&facts.vertex_names);
        self.places(&facts.vertex_faces);
    }
    fn bbox(&mut self, b: &Bbox3) {
        match b.corners() {
            Some((lo, hi)) => {
                self.u8(1);
                self.vec3(lo);
                self.vec3(hi);
            }
            None => self.u8(0),
        }
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

const SHORT: &str = "the scene's bytes end too soon";

impl Reader<'_> {
    fn f32(&mut self) -> Result<f32, String> {
        Ok(f32::from_bits(self.u32()?))
    }
    /// A face's layers, as [`Writer::optics`] writes them.
    fn optics(&mut self) -> Result<Optics, String> {
        let mut head = [0.0_f32; 5];
        for v in &mut head {
            *v = self.f32()?;
        }
        let [
            anisotropy,
            clearcoat,
            clearcoat_roughness,
            transmission,
            ior,
        ] = head;
        let absorption = [self.f32()?, self.f32()?, self.f32()?];
        let subsurface = self.f32()?;
        let tangent = match self.u8()? {
            0 => TangentField::Axis([self.f32()?, self.f32()?, self.f32()?]),
            1 => {
                let axis = [self.f32()?, self.f32()?, self.f32()?];
                let origin = [self.f64()?, self.f64()?, self.f64()?];
                TangentField::Around { origin, axis }
            }
            _ => return Err("a face's tangent field is of no known kind".into()),
        };
        let thin_walled = self.bool()?;
        let subsurface_color = if self.bool()? {
            Some([self.u8()?, self.u8()?, self.u8()?])
        } else {
            None
        };
        let pattern = match self.u8()? {
            0 => None,
            code => {
                let kind = PatternKind::ALL
                    .into_iter()
                    .find(|k| k.code() == u32::from(code))
                    .ok_or("a face's solid texture is of no known kind")?;
                let stops = self.u8()?;
                let mut ramp = [[0_u8; 3]; 4];
                for c in &mut ramp {
                    *c = [self.u8()?, self.u8()?, self.u8()?];
                }
                let roughness = if self.bool()? {
                    Some([self.f32()?, self.f32()?])
                } else {
                    None
                };
                let scale = self.f32()?;
                let origin = [self.f64()?, self.f64()?, self.f64()?];
                let axis = [self.f32()?, self.f32()?, self.f32()?];
                let seed = self.u32()?;
                let detail = self.u32()?;
                let [distortion, grain, rays, spot] =
                    [self.f32()?, self.f32()?, self.f32()?, self.f32()?];
                let wrap = self.bool()?;
                Some(
                    Pattern {
                        kind,
                        ramp,
                        stops,
                        roughness,
                        scale,
                        origin,
                        axis,
                        seed,
                        distortion,
                        detail,
                        grain,
                        rays,
                        spot,
                        wrap,
                    }
                    .sanitized(),
                )
            }
        };
        Ok(Optics {
            anisotropy,
            tangent,
            clearcoat,
            clearcoat_roughness,
            transmission,
            ior,
            thin_walled,
            absorption,
            subsurface,
            subsurface_color,
            pattern,
        })
    }
    fn take(&mut self, n: usize) -> Result<&[u8], String> {
        let end = self.at.checked_add(n).ok_or(SHORT)?;
        let got = self.bytes.get(self.at..end).ok_or(SHORT)?;
        self.at = end;
        Ok(got)
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(*self.take(1)?.first().ok_or(SHORT)?)
    }
    fn bool(&mut self) -> Result<bool, String> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err("the scene has a yes or no that is neither".into()),
        }
    }
    fn u32(&mut self) -> Result<u32, String> {
        let b: [u8; 4] = self.take(4)?.try_into().map_err(|_| SHORT)?;
        Ok(u32::from_le_bytes(b))
    }
    fn u64(&mut self) -> Result<u64, String> {
        let b: [u8; 8] = self.take(8)?.try_into().map_err(|_| SHORT)?;
        Ok(u64::from_le_bytes(b))
    }
    /// The bytes not read yet.
    fn left(&self) -> usize {
        self.bytes.len().saturating_sub(self.at)
    }
    /// The room to give a list of `n` entries of `T` up front: no more
    /// than the bytes left would fill (a list longer than that grows as
    /// its entries are read, each of which takes bytes of its own).
    fn room<T>(&self, n: usize) -> usize {
        n.min(self.left() / std::mem::size_of::<T>().max(1))
    }
    /// A count of things of at least `min` bytes each: never more than
    /// the bytes left can hold.
    fn count(&mut self, min: usize) -> Result<usize, String> {
        let n = usize::try_from(self.u32()?).map_err(|_| SHORT)?;
        if n.saturating_mul(min.max(1)) > self.left() {
            return Err(SHORT.into());
        }
        Ok(n)
    }
    fn f64(&mut self) -> Result<f64, String> {
        Ok(f64::from_bits(self.u64()?))
    }
    fn opt_f64(&mut self) -> Result<Option<f64>, String> {
        Ok(if self.bool()? {
            Some(self.f64()?)
        } else {
            None
        })
    }
    fn vec3(&mut self) -> Result<Vec3, String> {
        Ok(Vec3::new(self.f64()?, self.f64()?, self.f64()?))
    }
    fn points(&mut self) -> Result<Vec<Vec3>, String> {
        let n = self.count(24)?;
        let mut points = Vec::with_capacity(self.room::<Vec3>(n));
        for _ in 0..n {
            points.push(self.vec3()?);
        }
        Ok(points)
    }
    fn lines(&mut self) -> Result<EdgePolylines, String> {
        let n = self.count(4)?;
        let mut lines = Vec::with_capacity(self.room::<Vec<Vec3>>(n));
        for _ in 0..n {
            lines.push(self.points()?);
        }
        Ok(lines)
    }
    fn text(&mut self) -> Result<String, String> {
        let n = self.count(1)?;
        String::from_utf8(self.take(n)?.to_vec())
            .map_err(|_| "a text of the scene is not UTF-8".into())
    }
    fn opt_text(&mut self) -> Result<Option<String>, String> {
        Ok(if self.bool()? {
            Some(self.text()?)
        } else {
            None
        })
    }
    fn texts(&mut self) -> Result<Vec<String>, String> {
        let n = self.count(4)?;
        let mut texts = Vec::with_capacity(self.room::<String>(n));
        for _ in 0..n {
            texts.push(self.text()?);
        }
        Ok(texts)
    }
    fn places(&mut self) -> Result<Vec<Vec<usize>>, String> {
        let n = self.count(4)?;
        let mut lists = Vec::with_capacity(self.room::<Vec<usize>>(n));
        for _ in 0..n {
            let m = self.count(8)?;
            let mut list = Vec::with_capacity(self.room::<usize>(m));
            for _ in 0..m {
                list.push(usize::try_from(self.u64()?).map_err(|_| "a place is out of range")?);
            }
            lists.push(list);
        }
        Ok(lists)
    }
    fn mesh(&mut self) -> Result<Mesh, String> {
        let positions = self.points()?;
        let normals = self.points()?;
        let n = self.count(12)?;
        let mut triangles = Vec::with_capacity(self.room::<[u32; 3]>(n));
        for _ in 0..n {
            triangles.push([self.u32()?, self.u32()?, self.u32()?]);
        }
        let n = self.count(4)?;
        let mut face_ids = Vec::with_capacity(self.room::<u32>(n));
        for _ in 0..n {
            face_ids.push(self.u32()?);
        }
        // What a renderer and a pick take for granted of a mesh.
        let corners = positions.len();
        if normals.len() != corners
            || face_ids.len() != triangles.len()
            || triangles
                .iter()
                .flatten()
                .any(|&k| !usize::try_from(k).is_ok_and(|k| k < corners))
        {
            return Err("a mesh of the scene cannot be read: its triangles, normals or face ids do not match its corners".into());
        }
        Ok(Mesh {
            positions,
            normals,
            triangles,
            face_ids,
        })
    }
    fn facts(&mut self) -> Result<BodyFacts, String> {
        let face_names = self.texts()?;
        let n = self.count(4)?;
        let mut face_aliases = Vec::with_capacity(self.room::<Vec<String>>(n));
        for _ in 0..n {
            face_aliases.push(self.texts()?);
        }
        Ok(BodyFacts {
            face_names: Arc::new(face_names),
            face_aliases: Arc::new(face_aliases),
            edge_names: Arc::new(self.texts()?),
            edge_faces: Arc::new(self.places()?),
            vertices: self.points()?,
            vertex_names: Arc::new(self.texts()?),
            vertex_faces: Arc::new(self.places()?),
        })
    }
    fn bbox(&mut self) -> Result<Bbox3, String> {
        Ok(if self.bool()? {
            Bbox3::from_corners(self.vec3()?, self.vec3()?)
        } else {
            Bbox3::EMPTY
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]

    use super::*;
    use crate::{SceneQuality, Session, Workspace};
    use serde_json::json;

    fn scene() -> (Session, Arc<Scene>) {
        let mut s = Session::new(Workspace::none());
        for (id, params) in [
            ("file.new", json!({"units": "mm"})),
            ("feature.block", json!({"size": [10, 10, 10], "name": "B"})),
            (
                "feature.cylinder",
                json!({"radius": 2, "height": 20, "position": [5, 5, -5], "name": "P"}),
            ),
            (
                "feature.boolean",
                json!({"op": "subtract", "target": "B", "tool": "P"}),
            ),
            (
                "feature.sphere",
                json!({"radius": 3, "position": [30, 0, 0], "name": "S"}),
            ),
            ("datum.plane", json!({"from": "xy", "distance": 4})),
            ("material.preset", json!({"body": "S", "preset": "glass"})),
            (
                "body.appearance",
                json!({"body": "B", "color": "#808080", "metallic": 1, "anisotropy": 0.6, "clearcoat": 0.5, "subsurface": 0.2}),
            ),
        ] {
            s.execute(id, &params).unwrap();
        }
        let scene = s.scene_with(SceneQuality::display()).unwrap().unwrap();
        (s, scene)
    }

    #[test]
    fn a_scene_reads_back_bit_for_bit() {
        let (mut s, scene) = scene();
        let bytes = encode(&scene);
        let doc = Arc::clone(s.document_mut().unwrap().history.current());
        let back = decode(&bytes, &doc, scene.revision).unwrap();
        assert_eq!(encode(&back), bytes);
        assert_eq!(back, *scene);
        assert_eq!(back.datums.len(), 1);
        // The layers cross too: the glass sphere's and the brushed block's.
        assert!(back.face_colors.has_transmission());
        assert!(back.face_colors.optics_table().len() >= 2);
    }

    #[test]
    fn bytes_that_are_not_a_scene_are_an_error() {
        let (mut s, scene) = scene();
        let doc = Arc::clone(s.document_mut().unwrap().history.current());
        let bytes = encode(&scene);
        // Every cut short, and garbage.
        for end in (0..bytes.len()).step_by(7) {
            assert!(decode(&bytes[..end], &doc, 0).is_err());
        }
        let mut long = bytes.clone();
        long.push(0);
        assert!(decode(&long, &doc, 0).is_err());
        let mut huge = MAGIC.to_vec();
        huge.extend([0; 17]);
        huge.extend(u32::MAX.to_le_bytes());
        assert!(decode(&huge, &doc, 0).is_err());
        // Another document: its features are not these.
        let other = Document::default();
        assert!(decode(&bytes, &other, 0).is_err());
    }

    /// The bytes of a scene without bodies that colours face `face`.
    fn colour_without_bodies(face: u32) -> Vec<u8> {
        let mut b = MAGIC.to_vec();
        b.extend(1.0f64.to_bits().to_le_bytes());
        b.extend([0, 0, 0]); // no angle, not coarsened, no preview
        b.extend(0u32.to_le_bytes()); // no bodies
        b.push(0); // the mesh is theirs
        b.extend(1u32.to_le_bytes());
        b.extend(face.to_le_bytes());
        b.extend([200, 100, 50, 0, 0]);
        for _ in 0..5 {
            b.extend(0u32.to_le_bytes()); // sketches .. notices
        }
        b.extend([0, 0]); // two empty boxes
        b
    }

    /// A face colour of a face no body has is refused, however small its
    /// bytes: kept by face id, it would take room up to that id (16
    /// million entries for 2^24 - 1 from 60 bytes).
    #[test]
    fn a_face_colour_of_no_face_is_refused() {
        let doc = Document::default();
        for face in [0, 1 << 20, (1 << 24) - 1] {
            let error = decode(&colour_without_bodies(face), &doc, 0).unwrap_err();
            assert!(error.contains("no face of its bodies"), "{face}: {error}");
        }
        // Past the faces of real bodies too.
        let (mut s, scene) = scene();
        let doc = Arc::clone(s.document_mut().unwrap().history.current());
        let mut scene = (*scene).clone();
        let end = scene
            .bodies
            .iter()
            .map(|b| b.first_face_id + b.facts.face_names.len().max(b.mesh.triangles.len()) as u32)
            .max()
            .unwrap();
        Arc::make_mut(&mut scene.face_colors).set(end, Some([1, 2, 3]));
        assert!(decode(&encode(&scene), &doc, 0).is_err());
        // Within them it reads.
        Arc::make_mut(&mut scene.face_colors).set(end, None);
        Arc::make_mut(&mut scene.face_colors).set(end - 1, Some([1, 2, 3]));
        let back = decode(&encode(&scene), &doc, scene.revision).unwrap();
        assert_eq!(back.face_colors.get(end - 1), Some([1, 2, 3]));
    }

    /// Bodies numbered otherwise than a scene numbers them do not read:
    /// a first face id moved on, a face id past the body's faces.
    #[test]
    fn bodies_numbered_otherwise_do_not_read() {
        let (mut s, scene) = scene();
        let doc = Arc::clone(s.document_mut().unwrap().history.current());
        assert!(scene.bodies.len() >= 2);
        let mut moved = (*scene).clone();
        Arc::make_mut(&mut moved.bodies)[1].first_face_id += 1;
        let error = decode(&encode(&moved), &doc, 0).unwrap_err();
        assert!(error.contains("numbered"), "{error}");
        let mut far = (*scene).clone();
        let body = &mut Arc::make_mut(&mut far.bodies)[0];
        Arc::make_mut(&mut body.mesh).face_ids[0] = (1 << 24) - 2;
        let error = decode(&encode(&far), &doc, 0).unwrap_err();
        assert!(error.contains("numbered"), "{error}");
    }

    /// A mesh a renderer could not draw does not read: a corner it does
    /// not have, a normal short, a face id short; in a body's mesh and in
    /// the scene's own (written when it is not the bodies' joined).
    #[test]
    fn a_mesh_that_does_not_hold_together_does_not_read() {
        let (mut s, scene) = scene();
        let doc = Arc::clone(s.document_mut().unwrap().history.current());
        let breaks: [fn(&mut Mesh); 3] = [
            |m| m.triangles[0][1] = m.positions.len() as u32,
            |m| {
                m.normals.pop();
            },
            |m| {
                m.face_ids.pop();
            },
        ];
        for (k, broken) in breaks.iter().enumerate() {
            let mut own = (*scene).clone();
            broken(Arc::make_mut(&mut own.mesh));
            let error = decode(&encode(&own), &doc, 0).unwrap_err();
            assert!(error.contains("cannot be read"), "{k}: {error}");
            let mut body = (*scene).clone();
            let first = &mut Arc::make_mut(&mut body.bodies)[0];
            broken(Arc::make_mut(&mut first.mesh));
            let error = decode(&encode(&body), &doc, 0).unwrap_err();
            assert!(error.contains("cannot be read"), "{k}: {error}");
        }
    }

    /// A scene whose own mesh is not its bodies' joined (written whole,
    /// the tag 1) reads back bit for bit too.
    #[test]
    fn a_scene_mesh_written_whole_reads_back() {
        let (mut s, scene) = scene();
        let doc = Arc::clone(s.document_mut().unwrap().history.current());
        let joined = encode(&scene);
        let mut own = (*scene).clone();
        own.mesh = Arc::clone(&scene.bodies[0].mesh);
        let bytes = encode(&own);
        assert_ne!(bytes, joined);
        let back = decode(&bytes, &doc, own.revision).unwrap();
        assert_eq!(back, own);
        assert_eq!(encode(&back), bytes);
    }

    /// No list is given more room up front than the bytes left would fill.
    #[test]
    fn the_room_given_up_front_is_within_the_bytes() {
        let bytes = [0u8; 100];
        let r = Reader {
            bytes: &bytes,
            at: 10,
        };
        assert!(r.room::<SceneFeature>(1_000_000) * std::mem::size_of::<SceneFeature>() <= 90);
        assert!(r.room::<u32>(1_000_000) <= 22);
        assert_eq!(r.room::<u8>(5), 5);
    }
}
