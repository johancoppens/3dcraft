//! STEP export without an external reader: what is written parses back,
//! is a consistent B-rep when read by a small independent interpreter,
//! follows each convention of `craft3d_io::step`, and is pinned as text
//! for two small bodies. What only another kernel can tell (that the file
//! means the solid we made) is in `step_freecad.rs`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

#[path = "common/bodies.rs"]
mod bodies;

use craft3d_geom::{Cone, Curve3, Surface};
use craft3d_io::{AngleUnit, IoError, StepBody, StepOptions, StepSummary, export_step, step_model};
use craft3d_kernel::primitives;
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_meshio::Unit;
use craft3d_step::{Entity, Instance, Param, Record, StepFile};
use craft3d_testkit::{solids, trimmed};
use craft3d_topo::{Body, BodyKind};
use std::collections::BTreeMap;
use std::f64::consts::{FRAC_PI_2, PI};

fn tilted(scale: f64) -> Frame {
    Frame::new(
        Vec3::new(1.0, -2.0, 0.5) * scale,
        Vec3::new(0.3, -0.4, 1.0),
        Vec3::new(1.0, 0.2, 0.0),
    )
    .unwrap()
}

fn options(scale: f64) -> StepOptions {
    StepOptions {
        tolerance: Tolerance::new(1e-6 * scale.min(1.0), 1e-9).unwrap(),
        ..StepOptions::default()
    }
}

/// Every test body, with a name: the reference solids of the test kit
/// and the hand-built ones, at `scale`.
fn test_bodies(scale: f64) -> Vec<(String, Body)> {
    let frame = tilted(scale);
    let mut all = solids::all(&frame, scale).unwrap();
    all.extend(trimmed::all(&frame, scale).unwrap());
    all.push(trimmed::sphere_octant(&frame, 2.0 * scale).unwrap());
    all.push(trimmed::pipe_elbow(&frame, 3.0 * scale, 1.0 * scale, 1.2).unwrap());
    let mut out: Vec<(String, Body)> = all
        .into_iter()
        .map(|r| (r.name.to_string(), r.body))
        .collect();
    out.extend([
        (
            "cavity sphere".to_string(),
            bodies::block_with_spherical_cavity(scale),
        ),
        (
            "cavities box and cone".to_string(),
            bodies::block_with_two_cavities(scale),
        ),
        ("two lumps".to_string(), bodies::two_lumps(scale)),
        ("nested".to_string(), bodies::nested(scale)),
        (
            "nurbs can".to_string(),
            bodies::nurbs_can(&frame, 1.5 * scale, 2.0 * scale),
        ),
        (
            "quarter round".to_string(),
            bodies::quarter_round(&frame, 1.5 * scale, 2.0 * scale),
        ),
        ("open box".to_string(), bodies::open_box(scale)),
        (
            "washer".to_string(),
            bodies::washer(&frame, 3.0 * scale, scale),
        ),
        (
            "ellipse wide".to_string(),
            bodies::elliptic_disc(&frame, 3.0 * scale, scale),
        ),
        (
            "ellipse tall".to_string(),
            bodies::elliptic_disc(&frame, scale, 3.0 * scale),
        ),
    ]);
    out
}

fn model(name: &str, body: &Body, options: &StepOptions) -> (StepFile, StepSummary) {
    step_model(&[StepBody { name, body }], options).unwrap_or_else(|e| panic!("{name}: {e}"))
}

// ------------------------------------------------- a small interpreter

/// The file as a graph, read with nothing of the exporter.
struct Graph<'a> {
    by_id: BTreeMap<u64, &'a Instance>,
    file: &'a StepFile,
}

type P3 = [f64; 3];

fn sub(a: P3, b: P3) -> P3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: P3, b: P3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: P3, b: P3) -> P3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn norm(a: P3) -> f64 {
    dot(a, a).sqrt()
}

impl<'a> Graph<'a> {
    fn new(file: &'a StepFile) -> Self {
        Self {
            by_id: file.index(),
            file,
        }
    }

    fn all(&self, keyword: &'a str) -> impl Iterator<Item = (u64, &'a Record)> + '_ {
        self.file
            .data
            .iter()
            .filter_map(move |i| Some((i.id, i.entity.record(keyword)?)))
    }

    /// The record `keyword` of instance `id`, which must be of that type.
    fn record(&self, id: u64, keyword: &str) -> &'a Record {
        self.by_id[&id]
            .entity
            .record(keyword)
            .unwrap_or_else(|| panic!("#{id} is not a {keyword}: {:?}", self.by_id[&id]))
    }

    fn simple(&self, id: u64) -> &'a Record {
        match &self.by_id[&id].entity {
            Entity::Simple(record) => record,
            Entity::Complex(_) => panic!("#{id} is a complex instance"),
        }
    }

    fn reference(param: &Param) -> u64 {
        param.as_ref().unwrap_or_else(|| panic!("{param:?}"))
    }

    fn references(param: &Param) -> Vec<u64> {
        param
            .as_list()
            .unwrap()
            .iter()
            .map(Self::reference)
            .collect()
    }

    fn flag(param: &Param) -> bool {
        match param.as_enum() {
            Some("T") => true,
            Some("F") => false,
            _ => panic!("{param:?}"),
        }
    }

    fn triple(&self, id: u64, keyword: &str) -> P3 {
        let values = self.record(id, keyword).params[1].as_list().unwrap();
        [0, 1, 2].map(|i| values[i].as_real().unwrap())
    }

    /// `(origin, z, x)` of an `AXIS2_PLACEMENT_3D`.
    fn placement(&self, id: u64) -> (P3, P3, P3) {
        let p = &self.record(id, "AXIS2_PLACEMENT_3D").params;
        (
            self.triple(Self::reference(&p[1]), "CARTESIAN_POINT"),
            self.triple(Self::reference(&p[2]), "DIRECTION"),
            self.triple(Self::reference(&p[3]), "DIRECTION"),
        )
    }

    fn vertex(&self, id: u64) -> P3 {
        let point = Self::reference(&self.record(id, "VERTEX_POINT").params[1]);
        self.triple(point, "CARTESIAN_POINT")
    }

    /// How far `p` is from the curve `id`, for the kinds this interpreter
    /// knows; `None` for splines.
    fn off_curve(&self, id: u64, p: P3) -> Option<f64> {
        let entity = &self.by_id[&id].entity;
        if let Some(line) = entity.record("LINE") {
            let origin = self.triple(Self::reference(&line.params[1]), "CARTESIAN_POINT");
            let vector = self.record(Self::reference(&line.params[2]), "VECTOR");
            let direction = self.triple(Self::reference(&vector.params[1]), "DIRECTION");
            assert_eq!(vector.params[2], Param::Real(1.0), "unit speed");
            assert!((norm(direction) - 1.0).abs() < 1e-12);
            return Some(norm(cross(sub(p, origin), direction)));
        }
        let (keyword, radii) = if let Some(circle) = entity.record("CIRCLE") {
            let r = circle.params[2].as_real().unwrap();
            (circle, (r, r))
        } else if let Some(ellipse) = entity.record("ELLIPSE") {
            (
                ellipse,
                (
                    ellipse.params[2].as_real().unwrap(),
                    ellipse.params[3].as_real().unwrap(),
                ),
            )
        } else {
            return None;
        };
        let (origin, z, x) = self.placement(Self::reference(&keyword.params[1]));
        assert!(dot(z, x).abs() < 1e-12, "placement axes are perpendicular");
        let y = cross(z, x);
        let d = sub(p, origin);
        let (px, py, pz) = (dot(d, x), dot(d, y), dot(d, z));
        // Out of the plane, and off the conic in it (as a length).
        let conic = ((px / radii.0).powi(2) + (py / radii.1).powi(2)).sqrt() - 1.0;
        Some(pz.abs().max(conic.abs() * radii.0.min(radii.1)))
    }

    /// The signed distance of `p` from the surface `id` along its normal
    /// `∂u × ∂v`, for the elementary kinds; `None` for splines.
    fn above_surface(&self, id: u64, p: P3, radians_per_unit: f64) -> Option<f64> {
        let entity = &self.by_id[&id].entity;
        let (record, kind) = [
            "PLANE",
            "CYLINDRICAL_SURFACE",
            "CONICAL_SURFACE",
            "SPHERICAL_SURFACE",
            "TOROIDAL_SURFACE",
        ]
        .iter()
        .find_map(|k| Some((entity.record(k)?, *k)))?;
        let (origin, z, x) = self.placement(Self::reference(&record.params[1]));
        let y = cross(z, x);
        let d = sub(p, origin);
        let (px, py, pz) = (dot(d, x), dot(d, y), dot(d, z));
        let rho = px.hypot(py);
        let real = |i: usize| record.params[i].as_real().unwrap();
        Some(match kind {
            "PLANE" => pz,
            "CYLINDRICAL_SURFACE" => rho - real(2),
            "CONICAL_SURFACE" => {
                let angle = real(3) * radians_per_unit;
                assert!(angle > 0.0 && angle < FRAC_PI_2, "a positive semi-angle");
                assert!(real(2) >= 0.0);
                // Distance from the line rho = R + z tan(a), outward.
                (rho - real(2) - pz * angle.tan()) * angle.cos()
            }
            "SPHERICAL_SURFACE" => norm(d) - real(2),
            _ => {
                assert!(real(2) > real(3), "major radius first");
                (rho - real(2)).hypot(pz) - real(3)
            }
        })
    }

    /// The plane angle unit of the file in radians.
    fn radians_per_unit(&self) -> f64 {
        let context = self.all("GLOBAL_UNIT_ASSIGNED_CONTEXT").next().unwrap().1;
        for unit in Self::references(&context.params[0]) {
            let entity = &self.by_id[&unit].entity;
            if !entity.is("PLANE_ANGLE_UNIT") {
                continue;
            }
            return match entity.record("CONVERSION_BASED_UNIT") {
                None => 1.0,
                Some(converted) => {
                    let measure = self.simple(Self::reference(&converted.params[1]));
                    match &measure.params[0] {
                        Param::Typed(_, value) => value.as_real().unwrap(),
                        other => panic!("{other:?}"),
                    }
                }
            };
        }
        panic!("no plane angle unit")
    }

    /// Checks the topology of every shell as a reader would follow it,
    /// and that the geometry is where the topology says: loops are closed
    /// chains, vertices lie on their edges' curves and on their faces'
    /// surfaces, and in a closed shell every edge is used once each way.
    fn check_shells(&self, linear: f64) {
        let radians = self.radians_per_unit();
        for keyword in ["CLOSED_SHELL", "OPEN_SHELL"] {
            for (shell_id, shell) in self.all(keyword) {
                // Per edge: the directions it is used in, as seen through
                // the bound's and the oriented edge's orientation.
                let mut uses: BTreeMap<u64, Vec<bool>> = BTreeMap::new();
                for face_id in Self::references(&shell.params[1]) {
                    let face = self.record(face_id, "ADVANCED_FACE");
                    let surface = Self::reference(&face.params[2]);
                    let bounds = Self::references(&face.params[1]);
                    assert!(!bounds.is_empty(), "#{face_id}: a face needs a bound");
                    let mut outer = 0;
                    for (at, bound_id) in bounds.iter().enumerate() {
                        let bound = self.simple(*bound_id);
                        match bound.keyword.as_str() {
                            "FACE_OUTER_BOUND" => {
                                outer += 1;
                                assert_eq!(at, 0, "#{face_id}: the outer bound comes first");
                            }
                            "FACE_BOUND" => {}
                            other => panic!("#{bound_id}: {other}"),
                        }
                        let bound_sense = Self::flag(&bound.params[2]);
                        let loop_id = Self::reference(&bound.params[1]);
                        let loop_ = self.simple(loop_id);
                        if loop_.keyword == "VERTEX_LOOP" {
                            let p = self.vertex(Self::reference(&loop_.params[1]));
                            if let Some(off) = self.above_surface(surface, p, radians) {
                                assert!(off.abs() <= linear, "#{loop_id}: {off}");
                            }
                            continue;
                        }
                        assert_eq!(loop_.keyword, "EDGE_LOOP");
                        let mut ends: Vec<(u64, u64)> = Vec::new();
                        for oriented_id in Self::references(&loop_.params[1]) {
                            let oriented = self.record(oriented_id, "ORIENTED_EDGE");
                            assert_eq!(oriented.params[1], Param::Derived);
                            assert_eq!(oriented.params[2], Param::Derived);
                            let edge_id = Self::reference(&oriented.params[3]);
                            let sense = Self::flag(&oriented.params[4]);
                            let edge = self.record(edge_id, "EDGE_CURVE");
                            assert_eq!(edge.params[4], Param::boolean(true));
                            let (start, end) = (
                                Self::reference(&edge.params[1]),
                                Self::reference(&edge.params[2]),
                            );
                            let curve = Self::reference(&edge.params[3]);
                            for vertex in [start, end] {
                                let p = self.vertex(vertex);
                                if let Some(off) = self.off_curve(curve, p) {
                                    assert!(
                                        off <= linear,
                                        "#{edge_id}: vertex {off} off its curve"
                                    );
                                }
                                if let Some(off) = self.above_surface(surface, p, radians) {
                                    assert!(
                                        off.abs() <= linear,
                                        "#{face_id}: vertex of #{edge_id} is {off} off the surface"
                                    );
                                }
                            }
                            ends.push(if sense { (start, end) } else { (end, start) });
                            uses.entry(edge_id).or_default().push(sense == bound_sense);
                        }
                        assert!(!ends.is_empty(), "#{loop_id}: an empty loop");
                        for (i, (_, end)) in ends.iter().enumerate() {
                            let next = ends[(i + 1) % ends.len()].0;
                            assert_eq!(*end, next, "#{loop_id}: not a closed chain at {i}");
                        }
                    }
                    assert!(outer <= 1, "#{face_id}: one outer bound at most");
                }
                if keyword == "CLOSED_SHELL" {
                    for (edge, uses) in &uses {
                        assert!(
                            matches!(uses.as_slice(), [a, b] if a != b),
                            "shell #{shell_id}: edge #{edge} is used {uses:?}"
                        );
                    }
                }
            }
        }
    }
}

// ------------------------------------------------------------ the tests

#[test]
fn every_test_body_parses_back_and_is_a_consistent_b_rep() {
    let mut exported = 0;
    for scale in [1e-3, 1.0, 1e6] {
        for (name, body) in test_bodies(scale) {
            let options = options(scale);
            let (file, summary) = model(&name, &body, &options);
            let text = craft3d_step::write(&file).unwrap();
            assert!(text.is_ascii(), "{name}");
            let back =
                craft3d_step::read(text.as_bytes()).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(back, file, "{name}: the instance graph reads back");
            assert_eq!(craft3d_step::write(&back).unwrap(), text, "{name}");
            assert_eq!(back.reference_cycle(), None, "{name}");
            // Ids count up from 1, and nothing refers forward: a reader
            // can build every instance as it meets it.
            for (i, instance) in back.data.iter().enumerate() {
                assert_eq!(instance.id, i as u64 + 1, "{name}");
                instance.entity.for_each_ref(&mut |to| {
                    assert!(to < instance.id, "{name}: #{} -> #{to}", instance.id)
                });
            }
            // Every instance but the roots is used by another.
            let mut used = vec![false; back.data.len() + 1];
            for instance in &back.data {
                instance
                    .entity
                    .for_each_ref(&mut |to| used[to as usize] = true);
            }
            for instance in &back.data {
                let root = [
                    "SHAPE_DEFINITION_REPRESENTATION",
                    "PRODUCT_RELATED_PRODUCT_CATEGORY",
                    "APPLICATION_PROTOCOL_DEFINITION",
                ]
                .iter()
                .any(|k| instance.entity.is(k));
                assert!(
                    used[instance.id as usize] || root,
                    "{name}: #{} is unused",
                    instance.id
                );
            }
            // The export is the same every time.
            assert_eq!(
                export_step(
                    &[StepBody {
                        name: &name,
                        body: &body
                    }],
                    &options
                )
                .unwrap()
                .0,
                text.as_bytes()
            );

            let graph = Graph::new(&back);
            graph.check_shells(options.tolerance.linear * 4.0);
            // Counts: the summary, the file and the body agree.
            assert_eq!(summary.instances, back.data.len(), "{name}");
            assert_eq!(back.count("ADVANCED_FACE"), body.faces.len(), "{name}");
            assert_eq!(back.count("EDGE_CURVE"), body.edges.len(), "{name}");
            assert_eq!(back.count("EDGE_LOOP"), body.loops.len(), "{name}");
            assert_eq!(
                back.count("VERTEX_POINT"),
                body.vertices.len() + back.count("VERTEX_LOOP"),
                "{name}"
            );
            assert_eq!(
                back.count("CLOSED_SHELL") + back.count("OPEN_SHELL"),
                body.shells.len(),
                "{name}"
            );
            assert_eq!(
                back.count("ORIENTED_EDGE"),
                body.loops.iter().map(|l| l.coedges.len()).sum::<usize>(),
                "{name}"
            );
            assert_eq!(
                (
                    summary.faces,
                    summary.edges,
                    summary.shells,
                    summary.vertices,
                    summary.vertex_loops
                ),
                (
                    body.faces.len(),
                    body.edges.len(),
                    body.shells.len(),
                    back.count("VERTEX_POINT"),
                    back.count("VERTEX_LOOP")
                ),
                "{name}"
            );
            assert_eq!(back.count("PRODUCT"), 1);
            let solid = body.kind == BodyKind::Solid;
            assert_eq!(
                back.count("ADVANCED_BREP_SHAPE_REPRESENTATION"),
                usize::from(solid)
            );
            assert_eq!(
                back.count("MANIFOLD_SURFACE_SHAPE_REPRESENTATION"),
                usize::from(!solid)
            );
            exported += 1;
        }
    }
    assert_eq!(exported, 3 * 26);
}

fn text(name: &str, body: &Body, file_name: &str) -> String {
    let options = StepOptions {
        file_name: file_name.to_string(),
        ..StepOptions::default()
    };
    String::from_utf8(export_step(&[StepBody { name, body }], &options).unwrap().0).unwrap()
}

/// The exact text of two small files, so that a change of the mapping
/// shows up in review. Set `CRAFT3D_UPDATE_GOLDEN=1` to rewrite them
/// after a deliberate change.
#[test]
fn golden_text_of_a_block_and_a_cylinder() {
    let block = primitives::block(&Frame::WORLD, 4.0, 2.0, 1.0).unwrap();
    let cylinder = primitives::cylinder(
        &Frame::WORLD.at(Vec3::new(10.0, 0.0, 0.0)).unwrap(),
        1.5,
        4.0,
    )
    .unwrap();
    for (name, body, file) in [
        ("Block", &block, "block.step"),
        ("Cylinder", &cylinder, "cylinder.step"),
    ] {
        let actual = text(name, body, file);
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/golden")
            .join(file);
        if std::env::var_os("CRAFT3D_UPDATE_GOLDEN").is_some() {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, &actual).unwrap();
        }
        let expected = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
        if actual != expected {
            let out = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(file);
            std::fs::write(&out, &actual).unwrap();
            panic!(
                "{file} differs from tests/golden/{file}; the new text is in {}",
                out.display()
            );
        }
    }
}

#[test]
fn senses_and_orientations_follow_the_body() {
    // A block: every face has the sense of its plane, every bound runs
    // as written, half of the edge uses are reversed.
    let block = primitives::block(&Frame::WORLD, 4.0, 2.0, 1.0).unwrap();
    let (file, _) = model("b", &block, &StepOptions::default());
    let graph = Graph::new(&file);
    for (_, face) in graph.all("ADVANCED_FACE") {
        assert_eq!(face.params[3], Param::boolean(true));
    }
    for (_, bound) in graph.all("FACE_OUTER_BOUND") {
        assert_eq!(bound.params[2], Param::boolean(true));
    }
    assert_eq!(file.count("FACE_OUTER_BOUND"), 6);
    assert_eq!(file.count("FACE_BOUND"), 0);
    let reversed = graph
        .all("ORIENTED_EDGE")
        .filter(|(_, r)| r.params[4] == Param::boolean(false))
        .count();
    assert_eq!(reversed, 12);
    // Coedge by coedge, in order: the oriented edge is `.T.` exactly
    // when the coedge is not reversed.
    let written: Vec<bool> = graph
        .all("ORIENTED_EDGE")
        .map(|(_, r)| Graph::flag(&r.params[4]))
        .collect();
    let expected: Vec<bool> = block.shells[0]
        .faces
        .iter()
        .flat_map(|f| &block.face(*f).unwrap().loops)
        .flat_map(|l| &block.loop_(*l).unwrap().coedges)
        .map(|c| !c.reversed)
        .collect();
    assert_eq!(written, expected);

    // The same block inside out, as the cavity of a larger one: the
    // cavity's faces are written turned over, behind an oriented shell
    // that turns them back.
    let cavity = bodies::block_with_two_cavities(1.0);
    let (file, summary) = model("c", &cavity, &StepOptions::default());
    let graph = Graph::new(&file);
    assert_eq!((summary.solids, summary.voids, summary.shells), (1, 2, 3));
    assert_eq!(file.count("MANIFOLD_SOLID_BREP"), 0);
    let (_, brep) = graph.all("BREP_WITH_VOIDS").next().unwrap();
    let outer = graph.record(Graph::reference(&brep.params[1]), "CLOSED_SHELL");
    let voids = Graph::references(&brep.params[2]);
    assert_eq!(voids.len(), 2);
    let face_sense = |face: u64| Graph::flag(&graph.record(face, "ADVANCED_FACE").params[3]);
    let bound_senses = |face: u64| -> Vec<bool> {
        Graph::references(&graph.record(face, "ADVANCED_FACE").params[1])
            .iter()
            .map(|b| Graph::flag(&graph.simple(*b).params[2]))
            .collect()
    };
    for face in Graph::references(&outer.params[1]) {
        assert!(face_sense(face));
        assert_eq!(bound_senses(face), [true]);
    }
    for void in voids {
        let oriented = graph.record(void, "ORIENTED_CLOSED_SHELL");
        assert_eq!(oriented.params[1], Param::Derived);
        assert_eq!(oriented.params[3], Param::boolean(false));
        let shell = graph.record(Graph::reference(&oriented.params[2]), "CLOSED_SHELL");
        for face in Graph::references(&shell.params[1]) {
            // In the body the cavity's faces are reversed planes and a
            // reversed cone; turned over they have the surface's sense.
            assert!(
                face_sense(face),
                "the surface normal points out of the cavity's volume"
            );
            let senses = bound_senses(face);
            let surface = Graph::reference(&graph.record(face, "ADVANCED_FACE").params[2]);
            if graph.by_id[&surface].entity.is("CONICAL_SURFACE") {
                // The rim, turned; and the apex, which has no direction.
                assert_eq!(senses, [false, true]);
            } else {
                assert_eq!(senses, [false]);
            }
        }
    }
    // A reversed face that is not in a cavity keeps `.F.`: mirror a
    // block, whose planes then face inward and are marked reversed.
    let mirrored = block
        .transformed(&craft3d_math::Transform::mirror(Vec3::ZERO, Vec3::X).unwrap())
        .unwrap();
    assert!(mirrored.faces.iter().all(|f| f.reversed));
    let (file, _) = model("m", &mirrored, &StepOptions::default());
    for (_, face) in Graph::new(&file).all("ADVANCED_FACE") {
        assert_eq!(face.params[3], Param::boolean(false));
    }
}

#[test]
fn shells_are_sorted_into_lumps_and_their_cavities() {
    let body = bodies::nested(1.0);
    let (file, summary) = model("n", &body, &StepOptions::default());
    let graph = Graph::new(&file);
    assert_eq!((summary.solids, summary.voids, summary.shells), (3, 2, 5));
    // Each lump with what its cavity is made of: the big cube holds the
    // cubic cavity, the floating cube the spherical one, the cylinder
    // none.
    let mut found = Vec::new();
    for (_, brep) in graph.all("BREP_WITH_VOIDS") {
        let outer = graph.record(Graph::reference(&brep.params[1]), "CLOSED_SHELL");
        let corner = Graph::references(&outer.params[1])
            .iter()
            .flat_map(|f| Graph::references(&graph.record(*f, "ADVANCED_FACE").params[1]))
            .count();
        assert_eq!(corner, 6);
        let voids = Graph::references(&brep.params[2]);
        assert_eq!(voids.len(), 1);
        let shell = graph.record(
            Graph::reference(&graph.record(voids[0], "ORIENTED_CLOSED_SHELL").params[2]),
            "CLOSED_SHELL",
        );
        let faces = Graph::references(&shell.params[1]);
        // The size of the outer cube, from its first plane's points.
        let size = Graph::references(&outer.params[1])
            .iter()
            .map(|f| {
                let surface = Graph::reference(&graph.record(*f, "ADVANCED_FACE").params[2]);
                let (origin, ..) =
                    graph.placement(Graph::reference(&graph.record(surface, "PLANE").params[1]));
                origin[0].abs().max(origin[1].abs()).max(origin[2].abs())
            })
            .fold(0.0, f64::max);
        found.push((size, faces.len()));
    }
    found.sort_by(|a, b| a.partial_cmp(b).unwrap());
    // The floating cube reaches to 7 and has the sphere (one face); the
    // big cube reaches to 10 and has the cubic cavity (six faces).
    assert_eq!(found, [(7.0, 1), (10.0, 6)]);
    assert_eq!(file.count("MANIFOLD_SOLID_BREP"), 1, "the cylinder");
    let (_, representation) = graph
        .all("ADVANCED_BREP_SHAPE_REPRESENTATION")
        .next()
        .unwrap();
    assert_eq!(Graph::references(&representation.params[1]).len(), 3);

    // A cavity outside every lump, and a body that is all cavity.
    let stray = craft3d_topo::merge(&[
        &primitives::block(&Frame::WORLD, 1.0, 1.0, 1.0).unwrap(),
        &primitives::block(
            &Frame::WORLD.at(Vec3::new(5.0, 0.0, 0.0)).unwrap(),
            1.0,
            1.0,
            1.0,
        )
        .unwrap(),
        &bodies::inside_out(
            &primitives::sphere(&Frame::WORLD.at(Vec3::new(20.0, 0.0, 0.0)).unwrap(), 1.0).unwrap(),
        ),
    ])
    .unwrap()
    .0;
    let error = step_model(
        &[StepBody {
            name: "stray",
            body: &stray,
        }],
        &StepOptions::default(),
    )
    .unwrap_err();
    assert!(error.to_string().contains("lies in no part"), "{error}");
    let hollow = craft3d_topo::merge(&[
        &bodies::inside_out(&primitives::sphere(&Frame::WORLD, 1.0).unwrap()),
        &bodies::inside_out(
            &primitives::sphere(&Frame::WORLD.at(Vec3::new(5.0, 0.0, 0.0)).unwrap(), 1.0).unwrap(),
        ),
    ])
    .unwrap()
    .0;
    let error = step_model(
        &[StepBody {
            name: "hollow",
            body: &hollow,
        }],
        &StepOptions::default(),
    )
    .unwrap_err();
    assert!(error.to_string().contains("lies in no part"), "{error}");
}

#[test]
fn surfaces_are_written_as_the_standard_parametrises_them() {
    let frame = tilted(1.0);
    let axis = [frame.z().x, frame.z().y, frame.z().z];
    let near = |a: P3, b: P3| norm(sub(a, b)) < 1e-15;
    // A cone that narrows along its frame's z (as `primitives::cone`
    // makes it) is written on the frame seen from the other side, with a
    // positive semi-angle; one that widens, as it is.
    let narrowing = primitives::cone(&frame, 2.0, 3.0).unwrap();
    for (angle_unit, expected) in [
        (AngleUnit::Radian, (2.0_f64 / 3.0).atan()),
        (AngleUnit::Degree, (2.0_f64 / 3.0).atan().to_degrees()),
    ] {
        let options = StepOptions {
            angle_unit,
            ..StepOptions::default()
        };
        let (file, _) = model("cone", &narrowing, &options);
        let graph = Graph::new(&file);
        let (_, cone) = graph.all("CONICAL_SURFACE").next().unwrap();
        assert_eq!(
            cone.params[2],
            Param::Real(2.0),
            "the radius at the placement"
        );
        assert_eq!(cone.params[3], Param::Real(expected));
        let (_, z, x) = graph.placement(Graph::reference(&cone.params[1]));
        assert!(near(z, axis.map(|c| -c)), "the axis is turned: {z:?}");
        assert!(near(x, [frame.x().x, frame.x().y, frame.x().z]));
        assert!((graph.radians_per_unit() * expected - (2.0_f64 / 3.0).atan()).abs() < 1e-15);
        // The apex is where the vertex loop is.
        let (_, vertex_loop) = graph.all("VERTEX_LOOP").next().unwrap();
        let apex = frame.point(0.0, 0.0, 3.0);
        assert!(near(
            graph.vertex(Graph::reference(&vertex_loop.params[1])),
            [apex.x, apex.y, apex.z]
        ));
        graph.check_shells(1e-9);
    }
    let mut widening = Body::new(BodyKind::Sheet);
    let surface = Surface::Cone(Cone::new(frame, 0.5, 0.3).unwrap());
    let mut rim = |v: f64| {
        let radius = 0.5 + v * 0.3_f64.tan();
        let at = frame.at(frame.point(0.0, 0.0, v)).unwrap();
        let vertex = widening.add_vertex(at.point(radius, 0.0, 0.0));
        widening.add_edge(
            Curve3::Circle(craft3d_geom::Circle::new(at, radius).unwrap()),
            (0.0, 2.0 * PI),
            vertex,
            vertex,
        )
    };
    let (low, high) = (rim(1.0), rim(2.0));
    let coedge = |edge, reversed| craft3d_topo::Coedge { edge, reversed };
    let loops = vec![
        widening.add_loop(vec![coedge(low, false)]),
        widening.add_loop(vec![coedge(high, true)]),
    ];
    let face = widening.add_face(surface, false, loops);
    widening.add_shell(vec![face]);
    let (file, summary) = model("frustum", &widening, &StepOptions::default());
    let graph = Graph::new(&file);
    let (_, cone) = graph.all("CONICAL_SURFACE").next().unwrap();
    assert_eq!(
        (&cone.params[2], &cone.params[3]),
        (&Param::Real(0.5), &Param::Real(0.3))
    );
    assert!(near(
        graph.placement(Graph::reference(&cone.params[1])).1,
        axis
    ));
    // Between two circles: no apex, no outer bound.
    assert_eq!(summary.vertex_loops, 0);
    assert_eq!(
        (file.count("FACE_OUTER_BOUND"), file.count("FACE_BOUND")),
        (0, 2)
    );
    graph.check_shells(1e-9);

    // Torus: major radius, then minor; a whole one has a vertex loop at
    // (u, v) = (0, 0), a whole sphere at its north pole.
    let (file, _) = model(
        "torus",
        &primitives::torus(&frame, 3.0, 1.0).unwrap(),
        &StepOptions::default(),
    );
    let graph = Graph::new(&file);
    let (_, torus) = graph.all("TOROIDAL_SURFACE").next().unwrap();
    assert_eq!(
        (&torus.params[2], &torus.params[3]),
        (&Param::Real(3.0), &Param::Real(1.0))
    );
    let (_, vertex_loop) = graph.all("VERTEX_LOOP").next().unwrap();
    let at = frame.point(4.0, 0.0, 0.0);
    assert!(
        norm(sub(
            graph.vertex(Graph::reference(&vertex_loop.params[1])),
            [at.x, at.y, at.z]
        )) < 1e-14
    );
    assert_eq!(
        (
            file.count("FACE_BOUND"),
            file.count("FACE_OUTER_BOUND"),
            file.count("EDGE_CURVE")
        ),
        (1, 0, 0)
    );
    let (file, _) = model(
        "sphere",
        &primitives::sphere(&frame, 2.5).unwrap(),
        &StepOptions::default(),
    );
    let graph = Graph::new(&file);
    let (_, vertex_loop) = graph.all("VERTEX_LOOP").next().unwrap();
    let pole = frame.point(0.0, 0.0, 2.5);
    assert!(
        norm(sub(
            graph.vertex(Graph::reference(&vertex_loop.params[1])),
            [pole.x, pole.y, pole.z]
        )) < 1e-14
    );
    // Faces whose boundary passes through an apex or a pole, or that are
    // bounded by one circle, have no vertex loop.
    for reference in [
        trimmed::sphere_octant(&frame, 2.0).unwrap(),
        trimmed::spherical_cap(&frame, 2.5, 1.0).unwrap(),
        trimmed::cone_frustum(&frame, 2.0, 0.8, 2.5).unwrap(),
        solids::hemisphere(&frame, 2.0).unwrap(),
    ] {
        let (file, _) = model(reference.name, &reference.body, &StepOptions::default());
        assert_eq!(file.count("VERTEX_LOOP"), 0, "{}", reference.name);
    }

    // An ellipse with the longer semi-axis along y is written with that
    // axis first, on a placement turned a quarter: the same points.
    for (a, b) in [(3.0, 1.0), (1.0, 3.0)] {
        let body = bodies::elliptic_disc(&frame, a, b);
        let (file, _) = model("ellipse", &body, &StepOptions::default());
        let graph = Graph::new(&file);
        let (id, ellipse) = graph.all("ELLIPSE").next().unwrap();
        assert_eq!(
            (&ellipse.params[2], &ellipse.params[3]),
            (&Param::Real(3.0), &Param::Real(1.0))
        );
        let (_, z, x) = graph.placement(Graph::reference(&ellipse.params[1]));
        assert!(near(z, axis));
        let long = if a > b { frame.x() } else { frame.y() };
        assert!(near(x, [long.x, long.y, long.z]));
        let Curve3::Ellipse(_) = &body.edges[0].curve else {
            panic!()
        };
        for i in 0..16 {
            let p = body.edges[0].curve.eval(f64::from(i) * 0.4);
            assert!(graph.off_curve(id, [p.x, p.y, p.z]).unwrap() < 1e-14);
        }
    }

    // The outer bound of a planar face is the loop that runs
    // counter-clockwise, wherever it is in the face's list.
    let (file, _) = model(
        "washer",
        &bodies::washer(&frame, 3.0, 1.0),
        &StepOptions::default(),
    );
    let graph = Graph::new(&file);
    let (_, face) = graph.all("ADVANCED_FACE").next().unwrap();
    let radii: Vec<(String, f64)> = Graph::references(&face.params[1])
        .iter()
        .map(|b| {
            let bound = graph.simple(*b);
            let edge_loop = graph.record(Graph::reference(&bound.params[1]), "EDGE_LOOP");
            let oriented =
                graph.record(Graph::references(&edge_loop.params[1])[0], "ORIENTED_EDGE");
            let edge = graph.record(Graph::reference(&oriented.params[3]), "EDGE_CURVE");
            let circle = graph.record(Graph::reference(&edge.params[3]), "CIRCLE");
            (bound.keyword.clone(), circle.params[2].as_real().unwrap())
        })
        .collect();
    assert_eq!(
        radii,
        [
            ("FACE_OUTER_BOUND".to_string(), 3.0),
            ("FACE_BOUND".to_string(), 1.0)
        ]
    );
}

#[test]
fn splines_keep_their_knots_points_and_weights() {
    let frame = Frame::WORLD;
    let body = bodies::nurbs_can(&frame, 1.5, 2.0);
    let (file, _) = model("can", &body, &StepOptions::default());
    let graph = Graph::new(&file);
    // The wall: rational, so a complex instance, its records in
    // alphabetical order.
    let wall = file
        .data
        .iter()
        .find(|i| i.entity.is("RATIONAL_B_SPLINE_SURFACE"))
        .unwrap();
    let keywords: Vec<&str> = wall
        .entity
        .records()
        .iter()
        .map(|r| r.keyword.as_str())
        .collect();
    assert_eq!(
        keywords,
        [
            "BOUNDED_SURFACE",
            "B_SPLINE_SURFACE",
            "B_SPLINE_SURFACE_WITH_KNOTS",
            "GEOMETRIC_REPRESENTATION_ITEM",
            "RATIONAL_B_SPLINE_SURFACE",
            "REPRESENTATION_ITEM",
            "SURFACE"
        ]
    );
    let Surface::Nurbs(surface) = &body.faces[0].surface else {
        panic!()
    };
    let spline = wall.entity.record("B_SPLINE_SURFACE").unwrap();
    assert_eq!(
        (&spline.params[0], &spline.params[1]),
        (&Param::Integer(2), &Param::Integer(1))
    );
    let rows = spline.params[2].as_list().unwrap();
    assert_eq!(rows.len(), 9, "one list per control point along u");
    for (i, row) in rows.iter().enumerate() {
        let row = Graph::references(row);
        assert_eq!(row.len(), 2);
        for (j, point) in row.iter().enumerate() {
            let p = surface.control_points()[i * 2 + j];
            assert_eq!(
                graph.triple(*point, "CARTESIAN_POINT"),
                [p.x + 0.0, p.y + 0.0, p.z + 0.0]
            );
        }
    }
    let knots = wall.entity.record("B_SPLINE_SURFACE_WITH_KNOTS").unwrap();
    assert_eq!(
        knots.params,
        [
            Param::List([3, 2, 2, 2, 3].map(Param::Integer).to_vec()),
            Param::List([2, 2].map(Param::Integer).to_vec()),
            Param::reals([0.0, 0.25, 0.5, 0.75, 1.0]),
            Param::reals([0.0, 1.0]),
            Param::enumeration("UNSPECIFIED"),
        ]
    );
    let weights = wall
        .entity
        .record("RATIONAL_B_SPLINE_SURFACE")
        .unwrap()
        .params[0]
        .as_list()
        .unwrap();
    let s = std::f64::consts::FRAC_1_SQRT_2;
    for (i, row) in weights.iter().enumerate() {
        let w = if i % 2 == 0 { 1.0 } else { s };
        assert_eq!(*row, Param::reals([w, w]));
    }
    // The rims: rational curves, nine points, the same knots.
    let curves: Vec<&Instance> = file
        .data
        .iter()
        .filter(|i| i.entity.is("RATIONAL_B_SPLINE_CURVE"))
        .collect();
    assert_eq!(curves.len(), 2);
    for curve in curves {
        let keywords: Vec<&str> = curve
            .entity
            .records()
            .iter()
            .map(|r| r.keyword.as_str())
            .collect();
        assert_eq!(
            keywords,
            [
                "BOUNDED_CURVE",
                "B_SPLINE_CURVE",
                "B_SPLINE_CURVE_WITH_KNOTS",
                "CURVE",
                "GEOMETRIC_REPRESENTATION_ITEM",
                "RATIONAL_B_SPLINE_CURVE",
                "REPRESENTATION_ITEM"
            ]
        );
        assert_eq!(
            curve.entity.record("B_SPLINE_CURVE").unwrap().params[0],
            Param::Integer(2)
        );
        assert_eq!(
            curve
                .entity
                .record("B_SPLINE_CURVE_WITH_KNOTS")
                .unwrap()
                .params[0],
            Param::List([3, 2, 2, 2, 3].map(Param::Integer).to_vec())
        );
        assert_eq!(
            curve
                .entity
                .record("RATIONAL_B_SPLINE_CURVE")
                .unwrap()
                .params[0],
            Param::reals([1.0, s, 1.0, s, 1.0, s, 1.0, s, 1.0])
        );
    }
    // Polynomial splines are simple instances: the domed block's roof and
    // its four cubic edges.
    let dome = trimmed::domed_block(&frame, 3.0, 2.0, 1.5, false).unwrap();
    let (file, _) = model("dome", &dome.body, &StepOptions::default());
    assert_eq!(file.count("RATIONAL_B_SPLINE_SURFACE"), 0);
    let graph = Graph::new(&file);
    let (_, roof) = graph.all("B_SPLINE_SURFACE_WITH_KNOTS").next().unwrap();
    assert_eq!(roof.params.len(), 13);
    assert_eq!(
        (&roof.params[1], &roof.params[2]),
        (&Param::Integer(3), &Param::Integer(3))
    );
    assert_eq!(
        roof.params[8],
        Param::List([4, 4].map(Param::Integer).to_vec())
    );
    assert_eq!(roof.params[10], Param::reals([0.0, 1.0]));
    assert_eq!(graph.all("B_SPLINE_CURVE_WITH_KNOTS").count(), 4);
    for (_, edge) in graph.all("B_SPLINE_CURVE_WITH_KNOTS") {
        assert_eq!(edge.params.len(), 9);
        assert_eq!(edge.params[1], Param::Integer(3));
        assert_eq!(Graph::references(&edge.params[2]).len(), 4);
    }
}

#[test]
fn units_uncertainty_and_names_come_from_the_options() {
    let block = primitives::block(&Frame::WORLD, 4.0, 2.0, 1.0).unwrap();
    for (unit, expected) in [
        (Unit::Micron, Ok(Some("MICRO"))),
        (Unit::Millimeter, Ok(Some("MILLI"))),
        (Unit::Centimeter, Ok(Some("CENTI"))),
        (Unit::Meter, Ok(None)),
        (Unit::Inch, Err(("INCH", 25.4))),
        (Unit::Foot, Err(("FOOT", 304.8))),
    ] {
        let options = StepOptions {
            length_unit: unit,
            tolerance: Tolerance::new(2.5e-5, 1e-9).unwrap(),
            ..StepOptions::default()
        };
        let (file, _) = model("b", &block, &options);
        let graph = Graph::new(&file);
        // The one context, with its three units and the uncertainty.
        let (_, context) = graph.all("GLOBAL_UNIT_ASSIGNED_CONTEXT").next().unwrap();
        let units = Graph::references(&context.params[0]);
        assert_eq!(units.len(), 3);
        let length = &graph.by_id[&units[0]].entity;
        assert!(length.is("LENGTH_UNIT") && length.is("NAMED_UNIT"));
        let millimetre = [Param::enumeration("MILLI"), Param::enumeration("METRE")];
        match expected {
            Ok(prefix) => {
                assert_eq!(
                    length.record("SI_UNIT").unwrap().params,
                    [
                        prefix.map_or(Param::Omitted, Param::enumeration),
                        Param::enumeration("METRE")
                    ]
                );
                assert_eq!(
                    length.record("NAMED_UNIT").unwrap().params,
                    [Param::Derived]
                );
            }
            Err((name, factor)) => {
                let converted = length.record("CONVERSION_BASED_UNIT").unwrap();
                assert_eq!(converted.params[0], Param::string(name));
                let measure = graph.record(
                    Graph::reference(&converted.params[1]),
                    "LENGTH_MEASURE_WITH_UNIT",
                );
                assert_eq!(
                    measure.params[0],
                    Param::typed("LENGTH_MEASURE", Param::Real(factor))
                );
                let base = &graph.by_id[&Graph::reference(&measure.params[1])].entity;
                assert_eq!(base.record("SI_UNIT").unwrap().params, millimetre);
                let dimensions = graph.record(
                    Graph::reference(&length.record("NAMED_UNIT").unwrap().params[0]),
                    "DIMENSIONAL_EXPONENTS",
                );
                assert_eq!(dimensions.params[0], Param::Real(1.0));
                assert!(
                    dimensions.params[1..]
                        .iter()
                        .all(|p| *p == Param::Real(0.0))
                );
                assert_eq!(dimensions.params.len(), 7);
            }
        }
        assert!(graph.by_id[&units[1]].entity.is("PLANE_ANGLE_UNIT"));
        assert!(graph.by_id[&units[2]].entity.is("SOLID_ANGLE_UNIT"));
        let (_, assigned) = graph
            .all("GLOBAL_UNCERTAINTY_ASSIGNED_CONTEXT")
            .next()
            .unwrap();
        let uncertainty = graph.record(
            Graph::references(&assigned.params[0])[0],
            "UNCERTAINTY_MEASURE_WITH_UNIT",
        );
        assert_eq!(
            uncertainty.params[0],
            Param::typed("LENGTH_MEASURE", Param::Real(2.5e-5))
        );
        assert_eq!(
            Graph::reference(&uncertainty.params[1]),
            units[0],
            "in the file's length unit"
        );
        assert_eq!(
            uncertainty.params[2],
            Param::string("distance_accuracy_value")
        );
        let (_, dimension) = graph
            .all("GEOMETRIC_REPRESENTATION_CONTEXT")
            .next()
            .unwrap();
        assert_eq!(dimension.params, [Param::Integer(3)]);
        // Coordinates are not converted: the far corner is (4, 2, 1).
        let far = graph
            .all("CARTESIAN_POINT")
            .map(|(id, _)| graph.triple(id, "CARTESIAN_POINT"))
            .fold([0.0_f64; 3], |m, p| {
                [m[0].max(p[0]), m[1].max(p[1]), m[2].max(p[2])]
            });
        assert_eq!(far, [4.0, 2.0, 1.0]);
    }

    // Names: as given, also with characters Part 21 has to escape; the
    // same name twice is numbered; no name is `Body`. Every body is a
    // product of its own, all in the one context.
    let names = [
        "Plate",
        "it's \\ \u{e9}\u{4e16}\u{1f600}",
        "Plate",
        "",
        "",
        "Plate",
    ];
    let ball = primitives::sphere(&Frame::WORLD, 1.0).unwrap();
    let items: Vec<StepBody> = names
        .iter()
        .enumerate()
        .map(|(i, name)| StepBody {
            name,
            body: if i % 2 == 0 { &block } else { &ball },
        })
        .collect();
    let (bytes, summary) = export_step(&items, &StepOptions::default()).unwrap();
    assert_eq!((summary.bodies, summary.solids), (6, 6));
    let file = craft3d_step::read(&bytes).unwrap();
    let graph = Graph::new(&file);
    let expected = [
        "Plate",
        names[1],
        "Plate (2)",
        "Body",
        "Body (2)",
        "Plate (3)",
    ];
    let products: Vec<&str> = graph
        .all("PRODUCT")
        .map(|(_, r)| r.params[1].as_str().unwrap())
        .collect();
    assert_eq!(products, expected);
    let ids: Vec<&str> = graph
        .all("PRODUCT")
        .map(|(_, r)| r.params[0].as_str().unwrap())
        .collect();
    assert_eq!(ids, expected, "product ids are unique");
    let solids: Vec<&str> = graph
        .all("MANIFOLD_SOLID_BREP")
        .map(|(_, r)| r.params[0].as_str().unwrap())
        .collect();
    assert_eq!(solids, expected);
    assert_eq!(file.count("GEOMETRIC_REPRESENTATION_CONTEXT"), 1);
    assert_eq!(file.count("APPLICATION_CONTEXT"), 1);
    // The chain from product to shape, per body.
    for (_, link) in graph.all("SHAPE_DEFINITION_REPRESENTATION") {
        let shape = graph.record(
            Graph::reference(&link.params[0]),
            "PRODUCT_DEFINITION_SHAPE",
        );
        let definition = graph.record(Graph::reference(&shape.params[2]), "PRODUCT_DEFINITION");
        let formation = graph.record(
            Graph::reference(&definition.params[2]),
            "PRODUCT_DEFINITION_FORMATION",
        );
        let product = graph.record(Graph::reference(&formation.params[2]), "PRODUCT");
        let representation = graph.record(
            Graph::reference(&link.params[1]),
            "ADVANCED_BREP_SHAPE_REPRESENTATION",
        );
        assert_eq!(product.params[1], representation.params[0]);
        let brep = graph.record(
            Graph::references(&representation.params[1])[0],
            "MANIFOLD_SOLID_BREP",
        );
        assert_eq!(product.params[1], brep.params[0]);
    }
    let header = file.header_values().unwrap();
    assert_eq!(header.schemas, [craft3d_io::step::SCHEMA]);
    assert_eq!(header.implementation_level, "2;1");
    assert_eq!((header.name.as_str(), header.time_stamp.as_str()), ("", ""));
    let (_, protocol) = graph.all("APPLICATION_PROTOCOL_DEFINITION").next().unwrap();
    assert_eq!(protocol.params[1], Param::string("automotive_design"));
    let dated = StepOptions {
        file_name: "a b.step".into(),
        time_stamp: "2026-10-13T12:00:00".into(),
        author: "J".into(),
        organization: "O".into(),
        ..StepOptions::default()
    };
    let (file, _) = model("b", &block, &dated);
    let header = file.header_values().unwrap();
    assert_eq!(
        (
            header.name.as_str(),
            header.time_stamp.as_str(),
            header.author,
            header.organization
        ),
        (
            "a b.step",
            "2026-10-13T12:00:00",
            vec!["J".to_string()],
            vec!["O".to_string()]
        )
    );
}

#[test]
fn what_cannot_be_exported_is_an_error_that_says_why() {
    let options = StepOptions::default();
    let export = |body: &Body| step_model(&[StepBody { name: "bad", body }], &options).map(|_| ());
    assert_eq!(
        step_model(&[], &options).map(|_| ()),
        Err(IoError::NoBodies)
    );
    let block = primitives::block(&Frame::WORLD, 4.0, 2.0, 1.0).unwrap();
    for tolerance in [
        Tolerance {
            linear: 0.0,
            angular: 1e-9,
        },
        Tolerance {
            linear: f64::NAN,
            angular: 1e-9,
        },
        Tolerance {
            linear: 1e-6,
            angular: f64::INFINITY,
        },
    ] {
        let options = StepOptions {
            tolerance,
            ..StepOptions::default()
        };
        let error = step_model(
            &[StepBody {
                name: "b",
                body: &block,
            }],
            &options,
        )
        .unwrap_err();
        assert!(error.to_string().contains("tolerance"), "{error}");
    }

    // Bodies that fail the checker: a missing face (an open "solid"), a
    // dangling loop, a vertex off its edge, NaN.
    let mut open = block.clone();
    open.shells[0].faces.pop();
    let mut dangling = block.clone();
    dangling.faces[0].loops[0].0 = 999;
    let mut off = block.clone();
    off.vertices[3].point.x += 0.5;
    let mut nan = block.clone();
    nan.vertices[0].point.y = f64::NAN;
    let mut backwards = block.clone();
    backwards.loops[2].coedges[1].reversed ^= true;
    for (what, body) in [
        ("open", &open),
        ("dangling", &dangling),
        ("off", &off),
        ("nan", &nan),
        ("backwards", &backwards),
    ] {
        match export(body) {
            Err(IoError::InvalidBody {
                body,
                defects,
                total,
            }) => {
                assert_eq!(body, "bad");
                assert!(
                    !defects.is_empty() && defects.len() <= 8 && total >= defects.len(),
                    "{what}"
                );
            }
            other => panic!("{what}: {other:?}"),
        }
    }
    let error = export(&open).unwrap_err().to_string();
    assert!(
        error.contains("body `bad` is not valid") && error.contains("face"),
        "{error}"
    );

    // Faces without loops on surfaces that have no end.
    for surface in [
        Surface::Plane(craft3d_geom::Plane::new(Frame::WORLD)),
        Surface::Cylinder(craft3d_geom::Cylinder::new(Frame::WORLD, 1.0).unwrap()),
        Surface::Cone(Cone::new(Frame::WORLD, 1.0, 0.3).unwrap()),
        match &trimmed::domed_block(&Frame::WORLD, 3.0, 2.0, 1.5, true)
            .unwrap()
            .body
            .faces
        {
            faces => faces
                .iter()
                .find(|f| matches!(f.surface, Surface::Nurbs(_)))
                .unwrap()
                .surface
                .clone(),
        },
    ] {
        let mut body = Body::new(BodyKind::Sheet);
        let kind = surface.kind();
        let face = body.add_face(surface, false, Vec::new());
        body.add_shell(vec![face]);
        let error = export(&body).unwrap_err().to_string();
        assert!(
            error.contains("has no boundary") && error.contains(kind),
            "{error}"
        );
    }
    // A planar face with two outer boundaries, or with none.
    let mut two = bodies::washer(&Frame::WORLD, 3.0, 1.0);
    two.loops[0].coedges[0].reversed = false;
    let error = export(&two).unwrap_err().to_string();
    assert!(error.contains("2 counter-clockwise loops"), "{error}");
    let mut none = bodies::washer(&Frame::WORLD, 3.0, 1.0);
    none.loops[1].coedges[0].reversed = true;
    let error = export(&none).unwrap_err().to_string();
    assert!(error.contains("0 counter-clockwise loops"), "{error}");
    // A solid without shells; a second body that is bad names itself.
    let empty = Body::new(BodyKind::Solid);
    assert!(export(&empty).is_err());
    let error = step_model(
        &[
            StepBody {
                name: "good",
                body: &block,
            },
            StepBody {
                name: "worse",
                body: &open,
            },
        ],
        &options,
    )
    .unwrap_err();
    assert!(error.to_string().contains("`worse`"), "{error}");
}
