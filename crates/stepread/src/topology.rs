//! Shells, faces, loops, edges and vertices (ISO 10303-42, the topology
//! and geometric model schemas) as a `craft3d-topo` body.
//!
//! Orientation, as the export writes it (`craft3d-io`, module
//! documentation): a face's outward normal is its surface's when
//! `same_sense`, and its bounds run with the face on their left seen from
//! that normal; so `same_sense` is `!Face::reversed` and a bound with
//! `orientation` `.T.` is the loop as it is. An `ORIENTED_EDGE` with
//! `.T.` uses its edge from start to end. An `EDGE_CURVE` whose
//! `same_sense` is `.F.` runs against its curve: here every edge runs with
//! its curve, so it becomes an edge from the file's end vertex to its
//! start vertex, used the other way round by every loop. An oriented face
//! or shell with `.F.` turns its faces over (the normal and every loop).

use crate::finish::reverse;
use crate::geometry::{Read, Reader};
use crate::{ImportError, Imported, Item, ItemKind, Options, Report, boolean, err, keywords};
use craft3d_geom::{Curve3, Surface};
use craft3d_math::Vec3;
use craft3d_step::Param;
use craft3d_topo::{Body, BodyKind, Coedge, EdgeId, FaceId, LoopId, VertexId};
use std::collections::BTreeMap;

/// The edge of the body a file's `EDGE_CURVE` is, and whether it runs
/// against the file's edge.
#[derive(Clone, Copy)]
struct EdgeOf {
    edge: EdgeId,
    flipped: bool,
}

struct Builder<'a, 'r> {
    reader: &'r Reader<'a>,
    body: Body,
    /// The distance within which two vertices of the file are one.
    merge: f64,
    /// The document's linear tolerance.
    tolerance: f64,
    vertices: BTreeMap<u64, VertexId>,
    /// Vertices by cell of a grid `2 merge` wide.
    grid: BTreeMap<(i64, i64, i64), Vec<VertexId>>,
    edges: BTreeMap<u64, EdgeOf>,
    faces: BTreeMap<u64, FaceId>,
    /// Curves and surfaces read, by instance id: read once however many
    /// edges and faces use them (`crate::budget` bounds the copies).
    curves: BTreeMap<u64, Curve3>,
    surfaces: BTreeMap<u64, Read>,
    face_ids: Vec<u64>,
    edge_ids: Vec<u64>,
    vertex_ids: Vec<u64>,
    report: &'r mut Report,
}

fn cell(p: Vec3, size: f64) -> Option<(i64, i64, i64)> {
    let index = |x: f64| {
        let k = (x / size).floor();
        (k.is_finite() && k.abs() < 1e15).then_some(k as i64)
    };
    Some((index(p.x)?, index(p.y)?, index(p.z)?))
}

impl Builder<'_, '_> {
    fn describe(&self, id: u64) -> String {
        self.reader.parsed.describe(id)
    }

    fn bad<T>(&self, id: u64, what: &str) -> Result<T, ImportError> {
        err(format!("{}: {what}", self.describe(id)))
    }

    fn reference(&self, id: u64, param: Option<&Param>, what: &str) -> Result<u64, ImportError> {
        param
            .and_then(Param::as_ref)
            .map_or_else(|| self.bad(id, &format!("{what} is not a reference")), Ok)
    }

    fn flag(&self, id: u64, param: Option<&Param>, what: &str) -> Result<bool, ImportError> {
        param
            .and_then(boolean)
            .map_or_else(|| self.bad(id, &format!("{what} is not .T. or .F.")), Ok)
    }

    fn vertex(&mut self, id: u64) -> Result<VertexId, ImportError> {
        if let Some(&vertex) = self.vertices.get(&id) {
            return Ok(vertex);
        }
        let record = self.reader.parsed.record(id, "VERTEX_POINT")?;
        let point =
            self.reader
                .point(self.reference(id, record.params.get(1), "vertex_geometry")?)?;
        // A vertex of the file at the place of an earlier one is that one.
        let size = 2.0 * self.merge;
        let mut found: Option<(VertexId, f64)> = None;
        if let Some((cx, cy, cz)) = cell(point, size) {
            for dx in -1..=1 {
                for dy in -1..=1 {
                    for dz in -1..=1 {
                        let key = (cx + dx, cy + dy, cz + dz);
                        for &other in self.grid.get(&key).map_or(&[][..], Vec::as_slice) {
                            let Some(at) = self.body.vertex(other) else {
                                continue;
                            };
                            let gap = at.point.distance(point);
                            if gap <= self.merge && found.is_none_or(|(_, best)| gap < best) {
                                found = Some((other, gap));
                            }
                        }
                    }
                }
            }
        }
        let vertex = match found {
            Some((other, gap)) => {
                self.report.merged_vertices += 1;
                self.report.vertex_gap = self.report.vertex_gap.max(gap);
                other
            }
            None => {
                let vertex = self.body.add_vertex(point);
                self.vertex_ids.push(id);
                if let Some(key) = cell(point, size) {
                    self.grid.entry(key).or_default().push(vertex);
                }
                vertex
            }
        };
        self.vertices.insert(id, vertex);
        Ok(vertex)
    }

    /// The parameters of the part of `curve` from `a` to `b` (one vertex
    /// for a closed edge), with the start below the end.
    fn range(
        &self,
        id: u64,
        curve: &Curve3,
        a: VertexId,
        b: VertexId,
    ) -> Result<(f64, f64), ImportError> {
        let point = |v: VertexId| {
            self.body
                .vertex(v)
                .map(|v| v.point)
                .ok_or_else(|| ImportError(format!("{}: a vertex is missing", self.describe(id))))
        };
        let (pa, pb) = (point(a)?, point(b)?);
        let project = |p: Vec3| {
            curve
                .project(p)
                .map_err(|e| ImportError(format!("{}: {e}", self.describe(id))))
        };
        let (d0, d1) = curve.domain();
        // A curve that comes back to where it starts at the ends of its
        // domain (a closed B-spline).
        let closes = d0.is_finite()
            && d1.is_finite()
            && curve.eval(d0).distance(curve.eval(d1)) <= self.merge;
        if let Some(period) = curve.period() {
            let ta = project(pa)?;
            if a == b {
                return Ok((ta, ta + period));
            }
            let mut tb = project(pb)?;
            while tb <= ta {
                tb += period;
            }
            return Ok((ta, tb));
        }
        if a == b {
            if closes {
                return Ok((d0, d1));
            }
            return self.bad(
                id,
                "an edge that starts where it ends, on a curve that does not close",
            );
        }
        let mut ta = project(pa)?;
        let mut tb = project(pb)?;
        if ta >= tb && closes {
            let span = (d1 - d0).abs();
            if (tb - d0).abs() <= 1e-9 * span {
                tb = d1;
            } else if (ta - d1).abs() <= 1e-9 * span {
                ta = d0;
            }
        }
        if ta < tb {
            Ok((ta, tb))
        } else {
            self.bad(
                id,
                "the edge runs against its curve although same_sense says it runs with it",
            )
        }
    }

    fn edge(&mut self, id: u64) -> Result<EdgeOf, ImportError> {
        if let Some(&found) = self.edges.get(&id) {
            return Ok(found);
        }
        let record = self.reader.parsed.record(id, "EDGE_CURVE")?;
        if record.params.len() < 5 {
            return self.bad(id, "EDGE_CURVE has too few attributes");
        }
        let start = self.reference(id, record.params.get(1), "edge_start")?;
        let end = self.reference(id, record.params.get(2), "edge_end")?;
        let geometry = self.reference(id, record.params.get(3), "edge_geometry")?;
        let same_sense = self.flag(id, record.params.get(4), "same_sense")?;
        let curve = match self.curves.get(&geometry) {
            Some(curve) => curve.clone(),
            None => {
                let curve = self.reader.curve(geometry)?;
                self.curves.insert(geometry, curve.clone());
                curve
            }
        };
        let (start, end) = (self.vertex(start)?, self.vertex(end)?);
        let (a, b) = if same_sense {
            (start, end)
        } else {
            (end, start)
        };
        let range = self.range(id, &curve, a, b)?;
        let edge = self.body.add_edge(curve, range, a, b);
        self.edge_ids.push(id);
        let found = EdgeOf {
            edge,
            flipped: !same_sense,
        };
        self.edges.insert(id, found);
        Ok(found)
    }

    /// A bound's loop as coedges in the direction of the bound (`None`
    /// for a vertex loop, which faces here do not need).
    fn bound(&mut self, id: u64) -> Result<Option<Vec<Coedge>>, ImportError> {
        let instance = self.reader.parsed.instance(id)?;
        let entity = &instance.entity;
        let keyword = if entity.is("FACE_OUTER_BOUND") {
            "FACE_OUTER_BOUND"
        } else if entity.is("FACE_BOUND") {
            "FACE_BOUND"
        } else {
            return self.bad(id, "a face's bound that is not a FACE_BOUND");
        };
        let record = self.reader.parsed.record(id, keyword)?;
        let loop_id = self.reference(id, record.params.get(1), "bound")?;
        let forward = self.flag(id, record.params.get(2), "orientation")?;
        let loop_entity = &self.reader.parsed.instance(loop_id)?.entity;
        if loop_entity.is("VERTEX_LOOP") {
            return Ok(None);
        }
        if !loop_entity.is("EDGE_LOOP") {
            return err(format!(
                "#{loop_id} {}: this kind of loop is not read",
                keywords(loop_entity)
            ));
        }
        let loop_record = self.reader.parsed.record(loop_id, "EDGE_LOOP")?;
        let list = loop_record
            .params
            .get(1)
            .and_then(Param::as_list)
            .unwrap_or(&[]);
        if list.is_empty() {
            return self.bad(loop_id, "an edge loop without edges");
        }
        let mut coedges = Vec::with_capacity(list.len());
        for item in list {
            let oriented = self.reference(loop_id, Some(item), "edge_list")?;
            let record = self.reader.parsed.record(oriented, "ORIENTED_EDGE")?;
            let element = self.reference(oriented, record.params.get(3), "edge_element")?;
            let along = self.flag(oriented, record.params.get(4), "orientation")?;
            let found = self.edge(element)?;
            coedges.push(Coedge {
                edge: found.edge,
                reversed: along == found.flipped,
            });
        }
        if !forward {
            reverse(&mut coedges);
        }
        Ok(Some(coedges))
    }

    /// A face of a shell; `turned` when the face is used turned over.
    fn face(&mut self, id: u64, turned: bool) -> Result<FaceId, ImportError> {
        let entity = &self.reader.parsed.instance(id)?.entity;
        if entity.is("ORIENTED_FACE") {
            let record = self.reader.parsed.record(id, "ORIENTED_FACE")?;
            let element = self.reference(id, record.params.get(2), "face_element")?;
            let along = self.flag(id, record.params.get(3), "orientation")?;
            if self
                .reader
                .parsed
                .instance(element)?
                .entity
                .is("ORIENTED_FACE")
            {
                return self.bad(id, "an oriented face of an oriented face");
            }
            return self.face(element, turned != !along);
        }
        if self.faces.contains_key(&id) {
            return self.bad(id, "the face is used twice");
        }
        let keyword = if entity.is("ADVANCED_FACE") {
            "ADVANCED_FACE"
        } else if entity.is("FACE_SURFACE") {
            "FACE_SURFACE"
        } else {
            return err(format!(
                "#{id} {}: this kind of face is not read",
                keywords(entity)
            ));
        };
        let record = self.reader.parsed.record(id, keyword)?;
        let bounds = record.params.get(1).and_then(Param::as_list).unwrap_or(&[]);
        let surface_id = self.reference(id, record.params.get(2), "face_geometry")?;
        let same_sense = self.flag(id, record.params.get(3), "same_sense")?;
        let read = match self.surfaces.get(&surface_id) {
            Some(read) => read.clone(),
            None => {
                let read = self.reader.surface(surface_id)?;
                self.surfaces.insert(surface_id, read.clone());
                read
            }
        };
        let mut loops: Vec<LoopId> = Vec::with_capacity(bounds.len());
        for bound in bounds {
            let bound = self.reference(id, Some(bound), "bounds")?;
            if let Some(mut coedges) = self.bound(bound)? {
                if turned {
                    reverse(&mut coedges);
                }
                loops.push(self.body.add_loop(coedges));
            }
        }
        let (surface, opposite) = match read {
            Read::Done(surface, opposite) => (surface, opposite),
            Read::Extrusion {
                id: swept,
                curve,
                along,
            } => (
                crate::swept::bounded_extrusion(&self.body, &curve, along, &loops)
                    .or_else(|e| self.bad(swept, &e))?,
                false,
            ),
            Read::Spindle(spindle) => {
                crate::swept::spindle_part(&self.body, &spindle, &loops, self.tolerance)
                    .or_else(|e| self.bad(surface_id, &e))?
            }
        };
        // A face without an edge loop is its whole surface: only a surface
        // that closes all around is one (a sphere, a torus, a spline
        // surface closed both ways).
        let closed = match &surface {
            Surface::Sphere(_) | Surface::Torus(_) => true,
            other => other.period_u().is_some() && other.period_v().is_some(),
        };
        if loops.is_empty() && !closed {
            return self.bad(
                id,
                &format!(
                    "a face without an edge loop on a {}, which does not close all around",
                    surface.kind()
                ),
            );
        }
        let reversed = !same_sense ^ opposite ^ turned;
        let face = self.body.add_face(surface, reversed, loops);
        self.faces.insert(id, face);
        self.face_ids.push(id);
        Ok(face)
    }

    /// A shell's faces; `turned` when the shell is used turned over.
    fn shell(&mut self, id: u64, turned: bool) -> Result<(), ImportError> {
        let entity = &self.reader.parsed.instance(id)?.entity;
        for keyword in ["ORIENTED_CLOSED_SHELL", "ORIENTED_OPEN_SHELL"] {
            if entity.is(keyword) {
                let record = self.reader.parsed.record(id, keyword)?;
                let element = self.reference(id, record.params.get(2), "shell_element")?;
                let along = self.flag(id, record.params.get(3), "orientation")?;
                let inner = &self.reader.parsed.instance(element)?.entity;
                if inner.is("ORIENTED_CLOSED_SHELL") || inner.is("ORIENTED_OPEN_SHELL") {
                    return self.bad(id, "an oriented shell of an oriented shell");
                }
                return self.shell(element, turned != !along);
            }
        }
        let keyword = if entity.is("CLOSED_SHELL") {
            "CLOSED_SHELL"
        } else if entity.is("OPEN_SHELL") {
            "OPEN_SHELL"
        } else {
            return err(format!(
                "#{id} {}: this kind of shell is not read",
                keywords(entity)
            ));
        };
        let record = self.reader.parsed.record(id, keyword)?;
        let list = record.params.get(1).and_then(Param::as_list).unwrap_or(&[]);
        if list.is_empty() {
            return self.bad(id, "a shell without faces");
        }
        let mut faces = Vec::with_capacity(list.len());
        for item in list {
            let face = self.reference(id, Some(item), "cfs_faces")?;
            faces.push(self.face(face, turned)?);
        }
        self.body.add_shell(faces);
        Ok(())
    }
}

/// The shells of an item: (shell, used turned over, a cavity).
fn shells_of(reader: &Reader<'_>, item: &Item) -> Result<Vec<(u64, bool, bool)>, ImportError> {
    let parsed = reader.parsed;
    let id = item.id;
    let entity = &parsed.instance(id)?.entity;
    let reference = |param: Option<&Param>, what: &str| {
        param.and_then(Param::as_ref).ok_or_else(|| {
            ImportError(format!(
                "{}: {what} is not a reference",
                parsed.describe(id)
            ))
        })
    };
    match item.kind {
        ItemKind::Solid => {
            let mut out = Vec::new();
            // Simple BREP_WITH_VOIDS(name, outer, voids), simple
            // MANIFOLD_SOLID_BREP(name, outer), or a complex instance with
            // the voids in a record of their own.
            let (outer, voids) = match entity.record("BREP_WITH_VOIDS") {
                Some(record) if record.params.len() >= 3 => {
                    (record.params.get(1), record.params.get(2))
                }
                Some(record) => {
                    let solid = parsed.record(id, "MANIFOLD_SOLID_BREP")?;
                    (solid.params.get(1), record.params.first())
                }
                None => (
                    parsed.record(id, "MANIFOLD_SOLID_BREP")?.params.get(1),
                    None,
                ),
            };
            out.push((reference(outer, "outer")?, false, false));
            if let Some(voids) = voids {
                for void in voids.as_list().unwrap_or(&[]) {
                    out.push((reference(Some(void), "voids")?, false, true));
                }
            }
            Ok(out)
        }
        ItemKind::Sheet => {
            let record = parsed.record(id, "SHELL_BASED_SURFACE_MODEL")?;
            let list = record.params.get(1).and_then(Param::as_list).unwrap_or(&[]);
            if list.is_empty() {
                return err(format!(
                    "{}: a surface model without shells",
                    parsed.describe(id)
                ));
            }
            list.iter()
                .map(|s| Ok((reference(Some(s), "sbsm_boundary")?, false, false)))
                .collect()
        }
    }
}

pub(crate) fn build(
    reader: &Reader<'_>,
    item: &Item,
    options: &Options,
    report: &mut Report,
) -> Result<Imported, ImportError> {
    let tolerance = options.tolerance.linear;
    // The file's uncertainty, within the tolerance and the document's cap
    // on an entity's tolerance (docs/decisions.md, 2026-10-18).
    let cap = options.loosest;
    let merge = report
        .uncertainty
        .unwrap_or(tolerance)
        .clamp(tolerance, cap.max(tolerance));
    let kind = match item.kind {
        ItemKind::Solid => BodyKind::Solid,
        ItemKind::Sheet => BodyKind::Sheet,
    };
    let shells = shells_of(reader, item)?;
    let mut builder = Builder {
        reader,
        body: Body::new(kind),
        merge,
        tolerance,
        vertices: BTreeMap::new(),
        grid: BTreeMap::new(),
        edges: BTreeMap::new(),
        faces: BTreeMap::new(),
        curves: BTreeMap::new(),
        surfaces: BTreeMap::new(),
        face_ids: Vec::new(),
        edge_ids: Vec::new(),
        vertex_ids: Vec::new(),
        report: &mut *report,
    };
    let mut cavities = Vec::new();
    for (shell, turned, cavity) in shells {
        builder.shell(shell, turned)?;
        cavities.push(cavity);
    }
    let name = if item.name.is_empty() {
        format!("#{}", item.id)
    } else {
        item.name.clone()
    };
    let Builder {
        body,
        face_ids,
        edge_ids,
        vertex_ids,
        ..
    } = builder;
    crate::finish::finish(
        crate::finish::Assembled {
            body,
            cavities,
            face_ids,
            edge_ids,
            vertex_ids,
        },
        name,
        merge,
        options,
        report,
    )
}
