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
//! Reading never panics and never allocates more than the bytes it is
//! given can hold: every count is checked against what is left before
//! anything is made of it. A feature's kind is the page's own (the kind of
//! the feature of that id in the document the scene is read for), so the
//! scene of another document does not read.

use crate::meshes::{BodyFacts, EdgePolylines};
use crate::scene::{
    FeatureState, Scene, SceneBody, SceneDatum, SceneDatumShape, SceneFeature, ScenePolyline,
    SceneSketch, SketchCurve,
};
use craft3d_doc::Document;
use craft3d_math::{Bbox3, Vec3};
use craft3d_render::{FaceColors, Finish};
use craft3d_tess::Mesh;
use std::collections::BTreeMap;
use std::sync::Arc;

/// What the bytes of a scene start with, with the form's version.
const MAGIC: &[u8; 8] = b"C3DSCN\x00\x01";

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
            SceneDatumShape::Plane { center, u, v } => {
                w.u8(0);
                w.vec3(center);
                w.vec3(u);
                w.vec3(v);
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
    let count = r.count(4)?;
    let mut bodies = Vec::with_capacity(count);
    for _ in 0..count {
        bodies.push(SceneBody {
            feature: r.text()?,
            name: r.text()?,
            mesh: Arc::new(r.mesh()?),
            first_face_id: r.u32()?,
            edges: Arc::new(r.lines()?),
            facts: Arc::new(r.facts()?),
        });
    }
    let mesh = match r.u8()? {
        0 => concatenated(&bodies).ok_or("the scene's bodies cannot be joined into one mesh")?,
        1 => r.mesh()?,
        _ => return Err("the scene's mesh is not written as a mesh is".into()),
    };
    let mut face_colors = FaceColors::new();
    for _ in 0..r.count(5)? {
        let face = r.u32()?;
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
    let count = r.count(4)?;
    let mut sketches = Vec::with_capacity(count);
    for _ in 0..count {
        let feature = r.text()?;
        let name = r.text()?;
        let (origin, x_axis, y_axis, normal) = (r.vec3()?, r.vec3()?, r.vec3()?, r.vec3()?);
        let lines = r.count(4)?;
        let mut polylines = Vec::with_capacity(lines);
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
    let count = r.count(4)?;
    let mut datums = Vec::with_capacity(count);
    for _ in 0..count {
        let feature = r.text()?;
        let name = r.text()?;
        let shape = match r.u8()? {
            0 => SceneDatumShape::Plane {
                center: r.vec3()?,
                u: r.vec3()?,
                v: r.vec3()?,
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
    let count = r.count(4)?;
    let mut features = Vec::with_capacity(count);
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
        for aliases in &facts.face_aliases {
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
    /// A count of things of at least `min` bytes each: never more than
    /// the bytes left can hold.
    fn count(&mut self, min: usize) -> Result<usize, String> {
        let n = usize::try_from(self.u32()?).map_err(|_| SHORT)?;
        let left = self.bytes.len().saturating_sub(self.at);
        if n.saturating_mul(min.max(1)) > left {
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
        let mut points = Vec::with_capacity(n);
        for _ in 0..n {
            points.push(self.vec3()?);
        }
        Ok(points)
    }
    fn lines(&mut self) -> Result<EdgePolylines, String> {
        let n = self.count(4)?;
        let mut lines = Vec::with_capacity(n);
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
        let mut texts = Vec::with_capacity(n);
        for _ in 0..n {
            texts.push(self.text()?);
        }
        Ok(texts)
    }
    fn places(&mut self) -> Result<Vec<Vec<usize>>, String> {
        let n = self.count(4)?;
        let mut lists = Vec::with_capacity(n);
        for _ in 0..n {
            let m = self.count(8)?;
            let mut list = Vec::with_capacity(m);
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
        let mut triangles = Vec::with_capacity(n);
        for _ in 0..n {
            triangles.push([self.u32()?, self.u32()?, self.u32()?]);
        }
        let n = self.count(4)?;
        let mut face_ids = Vec::with_capacity(n);
        for _ in 0..n {
            face_ids.push(self.u32()?);
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
        let mut face_aliases = Vec::with_capacity(n);
        for _ in 0..n {
            face_aliases.push(self.texts()?);
        }
        Ok(BodyFacts {
            face_names,
            face_aliases,
            edge_names: self.texts()?,
            edge_faces: self.places()?,
            vertices: self.points()?,
            vertex_names: self.texts()?,
            vertex_faces: self.places()?,
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
                json!({"radius": 3, "position": [30, 0, 0]}),
            ),
            ("datum.plane", json!({"from": "xy", "distance": 4})),
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
}
