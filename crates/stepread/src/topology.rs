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

use crate::geometry::{Read, Reader, extruded};
use crate::{ImportError, Imported, Item, ItemKind, Options, Report, boolean, err, keywords};
use craft3d_geom::{Curve3, Surface};
use craft3d_math::Vec3;
use craft3d_step::Param;
use craft3d_topo::{Body, BodyKind, Coedge, Defect, EdgeId, FaceId, LoopId, VertexId};
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
            } => (self.extrusion_over(swept, &curve, along, &loops)?, false),
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

    /// The B-spline surface of an extrusion over as much of its direction
    /// as the face's edges reach: `v` of a point `X = C(u) + v V` lies
    /// between `(X·V − max P·V) / |V|²` and `(X·V − min P·V) / |V|²` over
    /// the control points `P` of the curve (their hull holds the curve),
    /// taken over points along every edge and widened by a quarter and
    /// one unit of `V` each way (past its ends the surface goes on as the
    /// straight lines it is, so the width only keeps projections inside).
    fn extrusion_over(
        &self,
        swept: u64,
        curve: &craft3d_geom::NurbsCurve,
        along: Vec3,
        loops: &[LoopId],
    ) -> Result<Surface, ImportError> {
        let length = along.length_squared();
        let (mut low, mut high) = (f64::INFINITY, f64::NEG_INFINITY);
        for p in curve.control_points() {
            let x = p.dot(along);
            (low, high) = (low.min(x), high.max(x));
        }
        let (mut v0, mut v1) = (f64::INFINITY, f64::NEG_INFINITY);
        for l in loops {
            let Some(l) = self.body.loop_(*l) else {
                continue;
            };
            for c in &l.coedges {
                let Some(edge) = self.body.edge(c.edge) else {
                    continue;
                };
                let (t0, t1) = edge.range;
                for k in 0..=32 {
                    let x = edge
                        .curve
                        .eval(t0 + (t1 - t0) * f64::from(k) / 32.0)
                        .dot(along);
                    v0 = v0.min((x - high) / length);
                    v1 = v1.max((x - low) / length);
                }
            }
        }
        if !(v0.is_finite() && v1.is_finite() && v1 >= v0 && length > 0.0) {
            return self.bad(
                swept,
                "an extrusion without bounding edges, or edges that are not finite",
            );
        }
        let margin = 0.25 * (v1 - v0) + 1.0;
        extruded(curve, along, (v0 - margin, v1 + margin))
            .map(Surface::Nurbs)
            .map_err(|e| ImportError(format!("{}: {e}", self.describe(swept))))
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

    /// Joins edges used once each that run between the same two vertices
    /// with their midpoints within the merge distance.
    fn sew_edges(&mut self) {
        let mut uses = vec![0usize; self.body.edges.len()];
        for l in &self.body.loops {
            for c in &l.coedges {
                if let Some(n) = uses.get_mut(c.edge.0 as usize) {
                    *n += 1;
                }
            }
        }
        // Open edges by the pair of vertices they join.
        let mut open: BTreeMap<(u32, u32), Vec<EdgeId>> = BTreeMap::new();
        for (i, edge) in self.body.edges.iter().enumerate() {
            if uses.get(i) == Some(&1) {
                let pair = (edge.start.0.min(edge.end.0), edge.start.0.max(edge.end.0));
                open.entry(pair).or_default().push(EdgeId(i as u32));
            }
        }
        let mid = |body: &Body, e: EdgeId| {
            body.edge(e)
                .map(|edge| edge.curve.eval(0.5 * (edge.range.0 + edge.range.1)))
        };
        // Edge → (edge it becomes, whether it then runs the other way).
        let mut into: BTreeMap<u32, (EdgeId, bool)> = BTreeMap::new();
        for group in open.values() {
            let mut taken = vec![false; group.len()];
            for (i, &first) in group.iter().enumerate() {
                if taken.get(i).copied().unwrap_or(true) {
                    continue;
                }
                for (j, &second) in group.iter().enumerate().skip(i + 1) {
                    if taken.get(j).copied().unwrap_or(true) {
                        continue;
                    }
                    let (Some(a), Some(b)) = (mid(&self.body, first), mid(&self.body, second))
                    else {
                        continue;
                    };
                    let gap = a.distance(b);
                    if gap <= self.merge {
                        let same_way = self
                            .body
                            .edge(first)
                            .zip(self.body.edge(second))
                            .is_some_and(|(x, y)| {
                                if x.start != x.end {
                                    return x.start == y.start;
                                }
                                // A closed edge: by the way the curves run
                                // at the midpoint.
                                let along = |e: &craft3d_topo::Edge| {
                                    e.curve.derivatives(0.5 * (e.range.0 + e.range.1)).1
                                };
                                along(x).dot(along(y)) > 0.0
                            });
                        into.insert(second.0, (first, !same_way));
                        if let Some(t) = taken.get_mut(j) {
                            *t = true;
                        }
                        if let Some(t) = taken.get_mut(i) {
                            *t = true;
                        }
                        self.report.merged_edges += 1;
                        self.report.edge_gap = self.report.edge_gap.max(gap);
                        break;
                    }
                }
            }
        }
        if into.is_empty() {
            return;
        }
        for l in &mut self.body.loops {
            for c in &mut l.coedges {
                if let Some(&(edge, flip)) = into.get(&c.edge.0) {
                    c.edge = edge;
                    c.reversed ^= flip;
                }
            }
        }
        // Drop the edges that went into others, keeping the order.
        let mut renumber = vec![u32::MAX; self.body.edges.len()];
        let mut kept = Vec::with_capacity(self.body.edges.len());
        let mut kept_ids = Vec::with_capacity(self.body.edges.len());
        for (i, edge) in std::mem::take(&mut self.body.edges).into_iter().enumerate() {
            if into.contains_key(&(i as u32)) {
                continue;
            }
            if let Some(slot) = renumber.get_mut(i) {
                *slot = kept.len() as u32;
            }
            kept.push(edge);
            kept_ids.push(self.edge_ids.get(i).copied().unwrap_or(0));
        }
        self.body.edges = kept;
        self.edge_ids = kept_ids;
        for l in &mut self.body.loops {
            for c in &mut l.coedges {
                if let Some(&n) = renumber.get(c.edge.0 as usize) {
                    c.edge = EdgeId(n);
                }
            }
        }
    }

    /// Sews the edges still used once to free edges that run within the
    /// cap of them (`topo::sew`: faces the file does not join, or joins
    /// only within a gap): the file's own sharing first (`sew_edges`),
    /// then this, at the document's cap on an entity's tolerance. Nothing
    /// is moved; what it joins is measured and recorded. A solid left
    /// with an edge used once is refused naming it and how far the
    /// nearest free edge is.
    fn sew_loose(
        &mut self,
        name: &str,
        tolerance: &craft3d_math::Tolerance,
        cap: f64,
    ) -> Result<(), ImportError> {
        let step_id = |ids: &[u64], k: u32| ids.get(k as usize).copied().unwrap_or(0);
        let sewn = match craft3d_topo::sew(&self.body, tolerance, cap, cap) {
            Ok(sewn) => sewn,
            Err(craft3d_topo::SewError::Gap {
                edge,
                vertex,
                measured,
                cap,
            }) => {
                let what = match (edge, vertex) {
                    (Some(e), _) => format!("a sewn edge (from edge {})", e.0),
                    (None, Some(v)) => format!("a sewn vertex (from vertex {})", v.0),
                    (None, None) => "an entity".to_string(),
                };
                let how = measured.map_or_else(
                    || "could not be proven within".to_string(),
                    |m| format!("lies {m:e} off what it joins, more than"),
                );
                return err(format!(
                    "`{name}`: {what} {how} the cap {cap:e} on an entity's own tolerance (the document's `loosest`); the body is refused rather than moved"
                ));
            }
            Err(e) => return err(format!("`{name}`: the faces could not be sewn: {e}")),
        };
        if sewn.joined > 0 {
            // The file's ids of the entities kept: the first that became
            // each (the lowest, kept with its curve or point).
            let mut edge_ids = vec![0u64; sewn.body.edges.len()];
            let mut edge_seen = vec![false; sewn.body.edges.len()];
            for (old, (new, _)) in sewn.edges.iter().enumerate() {
                let k = new.0 as usize;
                if let Some(slot) = edge_seen.get_mut(k)
                    && !*slot
                {
                    *slot = true;
                    if let Some(id) = edge_ids.get_mut(k) {
                        *id = step_id(&self.edge_ids, u32::try_from(old).unwrap_or(u32::MAX));
                    }
                }
            }
            let mut vertex_ids = vec![0u64; sewn.body.vertices.len()];
            let mut seen = vec![false; sewn.body.vertices.len()];
            for (old, new) in sewn.vertices.iter().enumerate() {
                let k = new.0 as usize;
                if let Some(slot) = seen.get_mut(k)
                    && !*slot
                {
                    *slot = true;
                    if let Some(id) = vertex_ids.get_mut(k) {
                        *id = step_id(&self.vertex_ids, u32::try_from(old).unwrap_or(u32::MAX));
                    }
                }
            }
            // The pieces of an edge split at a T-junction carry the
            // file's id of the edge they are part of, where no edge of the
            // file became them already.
            for (old, pieces) in &sewn.split {
                for (new, _) in pieces {
                    let k = new.0 as usize;
                    if let Some(slot) = edge_seen.get_mut(k)
                        && !*slot
                    {
                        *slot = true;
                        if let Some(id) = edge_ids.get_mut(k) {
                            *id = step_id(&self.edge_ids, old.0);
                        }
                    }
                }
            }
            self.edge_ids = edge_ids;
            self.vertex_ids = vertex_ids;
            self.report.split_edges += sewn.split.len();
            self.report.sewn_edges += sewn.joined;
            self.report.sewn_vertices += sewn.merged_vertices;
            self.report.sewn_gap = self.report.sewn_gap.max(sewn.largest_gap);
        }
        let open = sewn.worst_open();
        self.body = sewn.body;
        if self.body.kind == BodyKind::Solid
            && let Some((edge, nearest)) = open
        {
            let id = step_id(&self.edge_ids, edge.0);
            let nearest = nearest.map_or_else(
                || "no other free edge".to_string(),
                |d| format!("the nearest free edge is {d:e} away"),
            );
            return err(format!(
                "`{name}`: edge #{id} bounds only one face and no free edge runs within the cap {cap:e} of it ({nearest}; the document's `loosest`): the solid is not closed and is refused rather than moved"
            ));
        }
        Ok(())
    }

    /// Turns a shell over: every face's normal and every loop.
    fn turn_shell(&mut self, shell: usize) {
        let faces = self
            .body
            .shells
            .get(shell)
            .map(|s| s.faces.clone())
            .unwrap_or_default();
        for face in faces {
            let loops = match self.body.faces.get_mut(face.0 as usize) {
                Some(f) => {
                    f.reversed = !f.reversed;
                    f.loops.clone()
                }
                None => continue,
            };
            for l in loops {
                if let Some(l) = self.body.loops.get_mut(l.0 as usize) {
                    reverse(&mut l.coedges);
                }
            }
        }
    }
}

/// Points along an edge, per face that uses it, all within `tolerance` of
/// the face's surface: 1025 places along the edge (the ends included).
fn densely_within(body: &Body, edge: EdgeId, tolerance: f64) -> bool {
    const PLACES: u32 = 1024;
    let Some(e) = body.edge(edge) else {
        return false;
    };
    let (t0, t1) = e.range;
    let mut any = false;
    for face in &body.faces {
        let uses = face.loops.iter().any(|l| {
            body.loop_(*l)
                .is_some_and(|l| l.coedges.iter().any(|c| c.edge == edge))
        });
        if !uses {
            continue;
        }
        any = true;
        for k in 0..=PLACES {
            let p = e
                .curve
                .eval(t0 + (t1 - t0) * f64::from(k) / f64::from(PLACES));
            let near = face
                .surface
                .project(p)
                .ok()
                .map(|(u, v)| face.surface.eval(u, v).distance(p));
            if !near.is_some_and(|d| d <= tolerance) {
                return false;
            }
        }
    }
    any
}

/// A loop run the other way.
fn reverse(coedges: &mut [Coedge]) {
    coedges.reverse();
    for c in coedges {
        c.reversed = !c.reversed;
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

/// The worst distance among the defects that say an entity lies off
/// another (a gap a tolerance per entity would hold), with the defect.
fn worst_gap(defects: &[Defect]) -> Option<(f64, &Defect)> {
    let mut worst: Option<(f64, &Defect)> = None;
    for defect in defects {
        let distance = match defect {
            Defect::VertexOffEdge { distance, .. } | Defect::EdgeOffSurface { distance, .. } => {
                *distance
            }
            _ => continue,
        };
        let distance = if distance.is_finite() {
            distance
        } else {
            f64::INFINITY
        };
        if worst.is_none_or(|(d, _)| distance > d) {
            worst = Some((distance, defect));
        }
    }
    worst
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
        vertices: BTreeMap::new(),
        grid: BTreeMap::new(),
        edges: BTreeMap::new(),
        faces: BTreeMap::new(),
        curves: BTreeMap::new(),
        surfaces: BTreeMap::new(),
        face_ids: Vec::new(),
        edge_ids: Vec::new(),
        vertex_ids: Vec::new(),
        report,
    };
    let mut cavities = Vec::new();
    for (shell, turned, cavity) in shells {
        builder.shell(shell, turned)?;
        cavities.push(cavity);
    }
    builder.sew_edges();
    let name = if item.name.is_empty() {
        format!("#{}", item.id)
    } else {
        item.name.clone()
    };
    builder.sew_loose(&name, &options.tolerance, cap)?;
    // A solid's outside encloses a positive volume and a cavity a
    // negative one; a shell the other way round is turned over (a cavity
    // the file gives as a plain closed shell).
    if kind == BodyKind::Solid {
        let volumes = craft3d_topo::shell_volumes(&builder.body);
        for (shell, (measured, cavity)) in volumes.iter().zip(&cavities).enumerate() {
            if let Ok(measured) = measured
                && measured.volume.abs() > measured.bound
                && (measured.volume < 0.0) != *cavity
            {
                builder.turn_shell(shell);
                builder.report.turned_shells += 1;
            }
        }
    }
    // Every edge proven on the surfaces of its faces over its whole
    // length (not sampled), and every vertex measured against the ends of
    // its edges: what lies off by more than the tolerance gets what was
    // proven as its own tolerance, up to the cap; beyond it, or when no
    // proof within the cap is found, the body is refused naming the
    // entity. Geometry is never moved.
    let step_id = |ids: &[u64], k: u32| ids.get(k as usize).copied().unwrap_or(0);
    for k in 0..builder.body.edges.len() {
        let edge = EdgeId(u32::try_from(k).unwrap_or(u32::MAX));
        match craft3d_topo::record_edge(&mut builder.body, edge, &options.tolerance, cap) {
            Ok(Some(t)) => {
                builder.report.loose_edges += 1;
                builder.report.largest_tolerance = builder.report.largest_tolerance.max(t);
            }
            Ok(None) => {}
            // No proof within the cap was found. Where the prover gives
            // out on an edge that is exact (an arc through a sphere's
            // pole), a dense sampling within the tolerance accepts it,
            // counted; anything sampled off by more is refused.
            Err(craft3d_topo::ToleranceError::EdgeNotMeasured { .. })
                if densely_within(&builder.body, edge, tolerance) =>
            {
                builder.report.unproven_edges += 1;
            }
            Err(e) => {
                let what = match e {
                    craft3d_topo::ToleranceError::AboveCap { measured, .. } => {
                        format!(
                            "lies {:e} off the surfaces of its faces",
                            measured / craft3d_topo::RECORD_MARGIN
                        )
                    }
                    craft3d_topo::ToleranceError::EdgeNotMeasured { .. } => {
                        "could not be proven within the cap of its faces' surfaces".to_string()
                    }
                    other => other.to_string(),
                };
                return err(format!(
                    "`{name}`: edge #{} {what}, more than the cap {cap:e} on an entity's own tolerance (the document's `loosest`); the body is refused rather than moved",
                    step_id(&builder.edge_ids, edge.0)
                ));
            }
        }
    }
    // Every vertex, with one pass over the edges.
    let every: Vec<VertexId> = (0..builder.body.vertices.len())
        .map(|k| VertexId(u32::try_from(k).unwrap_or(u32::MAX)))
        .collect();
    match craft3d_topo::record_vertices(&mut builder.body, &every, &options.tolerance, cap) {
        Ok(recorded) => {
            for t in recorded.into_iter().filter_map(|(_, t)| t) {
                builder.report.loose_vertices += 1;
                builder.report.largest_tolerance = builder.report.largest_tolerance.max(t);
            }
        }
        Err((vertex, e)) => {
            let what = match e {
                craft3d_topo::ToleranceError::AboveCap { measured, .. } => {
                    format!(
                        "lies {:e} from the ends of its edges",
                        measured / craft3d_topo::RECORD_MARGIN
                    )
                }
                other => other.to_string(),
            };
            return err(format!(
                "`{name}`: vertex #{} {what}, more than the cap {cap:e} on an entity's own tolerance (the document's `loosest`); the body is refused rather than moved",
                step_id(&builder.vertex_ids, vertex.0)
            ));
        }
    }
    let defects = craft3d_topo::check_capped(&builder.body, &options.tolerance, cap);
    let face_of = |f: FaceId| {
        builder
            .face_ids
            .get(f.0 as usize)
            .map_or_else(|| format!("face {}", f.0), |id| format!("face #{id}"))
    };
    let edge_of = |e: EdgeId| {
        builder
            .edge_ids
            .get(e.0 as usize)
            .map_or_else(|| format!("edge {}", e.0), |id| format!("edge #{id}"))
    };
    let vertex_of = |v: VertexId| {
        builder
            .vertex_ids
            .get(v.0 as usize)
            .map_or_else(|| format!("vertex {}", v.0), |id| format!("vertex #{id}"))
    };
    if let Some((distance, defect)) = worst_gap(&defects) {
        let what = match defect {
            Defect::VertexOffEdge { edge, vertex, .. } => format!(
                "{} lies {distance:e} from the end of {}",
                vertex_of(*vertex),
                edge_of(*edge)
            ),
            Defect::EdgeOffSurface { face, edge, .. } => format!(
                "{} lies {distance:e} off the surface of {}",
                edge_of(*edge),
                face_of(*face)
            ),
            other => other.to_string(),
        };
        let gaps = defects
            .iter()
            .filter(|d| {
                matches!(
                    d,
                    Defect::VertexOffEdge { .. } | Defect::EdgeOffSurface { .. }
                )
            })
            .count();
        return err(format!(
            "`{name}`: {what}, more than its tolerance ({gaps} such gaps, this the largest); the body is refused rather than moved"
        ));
    }
    if let Some(first) = defects.first() {
        let shown: Vec<String> = defects.iter().take(4).map(ToString::to_string).collect();
        return err(format!(
            "`{name}` is not a valid body ({} defects): {}; faces, edges and vertices are numbered in the order the file's shells reach them (first: {first})",
            defects.len(),
            shown.join("; ")
        ));
    }
    let Builder {
        body,
        face_ids,
        edge_ids,
        vertex_ids,
        ..
    } = builder;
    Ok(Imported {
        body,
        name,
        path: String::new(),
        color: None,
        report: report.clone(),
        face_ids,
        edge_ids,
        vertex_ids,
    })
}
