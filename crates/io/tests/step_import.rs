//! STEP import, oracle (a) of docs/design/step-import.md: every body the
//! export writes reads back (`craft3d-stepread`) as bodies of the same
//! volume, area, face, edge and vertex counts and genus, at the scales
//! 1e-3, 1 and 1e6, and in another length unit than it was written in.
//! The bodies are those `step.rs` exports: the test kit's reference
//! solids, trimmed solids, cavities, several lumps, rational splines and a
//! sheet.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

#[path = "common/bodies.rs"]
mod bodies;

use craft3d_io::{StepBody, StepOptions, export_step};
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_meshio::Unit;
use craft3d_stepread::{Options, parse};
use craft3d_testkit::{solids, trimmed};
use craft3d_topo::{Body, BodyKind};

fn tilted(scale: f64) -> Frame {
    Frame::new(
        Vec3::new(1.0, -2.0, 0.5) * scale,
        Vec3::new(0.3, -0.4, 1.0),
        Vec3::new(1.0, 0.2, 0.0),
    )
    .unwrap()
}

fn tolerance(scale: f64) -> Tolerance {
    Tolerance::new(1e-6 * scale.min(1.0), 1e-9).unwrap()
}

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

/// What is compared: counts, and the measures.
#[derive(Debug, Default)]
struct Measures {
    faces: usize,
    edges: usize,
    vertices: usize,
    shells: usize,
    volume: f64,
    area: f64,
}

fn measure(bodies: &[&Body]) -> Measures {
    let mut m = Measures::default();
    for body in bodies {
        m.faces += body.faces.len();
        m.edges += body.edges.len();
        m.vertices += body.vertices.len();
        m.shells += body.shells.len();
        let mass = craft3d_topo::mass_properties(body).unwrap_or_else(|e| {
            panic!(
                "{e:?}: {:?}",
                body.faces
                    .iter()
                    .map(|f| (f.surface.kind(), f.loops.len(), f.reversed))
                    .collect::<Vec<_>>()
            )
        });
        m.volume += mass.volume;
        m.area += mass.area;
    }
    m
}

fn close(a: f64, b: f64, relative: f64, size: f64) -> bool {
    (a - b).abs() <= relative * a.abs().max(b.abs()) + 1e-12 * size
}

/// Exports `body` in `unit`, reads it back into millimetres (or `into`
/// metres per unit) and compares.
fn round_trip(name: &str, body: &Body, scale: f64, unit: Unit, problems: &mut Vec<String>) {
    let options = StepOptions {
        length_unit: unit,
        tolerance: tolerance(scale),
        ..StepOptions::default()
    };
    let (bytes, _) = match export_step(&[StepBody::new(name, body)], &options) {
        Ok(done) => done,
        Err(e) => {
            problems.push(format!("{name} @ {scale}: export: {e}"));
            return;
        }
    };
    let parsed = parse(&bytes).unwrap();
    // The file is in `unit`, the bodies read into the unit they were
    // made in: the length unit of the export says which that was.
    let metres = match unit {
        Unit::Micron => 1e-6,
        Unit::Millimeter => 1e-3,
        Unit::Centimeter => 1e-2,
        Unit::Meter => 1.0,
        Unit::Inch => 0.0254,
        Unit::Foot => 0.3048,
    };
    let read = Options {
        metres_per_unit: metres,
        tolerance: tolerance(scale),
    };
    let mut imported = Vec::new();
    for (index, item) in parsed.items().iter().enumerate() {
        match parsed.build(index, &read) {
            Ok(done) => {
                assert_eq!(
                    done.body.kind,
                    match item.kind {
                        craft3d_stepread::ItemKind::Solid => BodyKind::Solid,
                        craft3d_stepread::ItemKind::Sheet => BodyKind::Sheet,
                    }
                );
                assert_eq!(done.face_ids.len(), done.body.faces.len());
                assert_eq!(done.edge_ids.len(), done.body.edges.len());
                assert_eq!(done.vertex_ids.len(), done.body.vertices.len());
                imported.push(done);
            }
            Err(e) => {
                problems.push(format!("{name} @ {scale} {unit:?}: item {index}: {e}"));
                return;
            }
        }
    }
    let ours = measure(&[body]);
    let theirs = measure(&imported.iter().map(|i| &i.body).collect::<Vec<_>>());
    let size = scale * scale * scale;
    // Faces without loops (a whole sphere or torus) are written with a
    // vertex loop, whose vertex is not one of the body's: the counts are
    // compared on the body as it is.
    let same_counts = ours.faces == theirs.faces
        && ours.edges == theirs.edges
        && ours.vertices == theirs.vertices
        && ours.shells == theirs.shells;
    if !same_counts {
        problems.push(format!(
            "{name} @ {scale} {unit:?}: counts {:?} became {:?}",
            (ours.faces, ours.edges, ours.vertices, ours.shells),
            (theirs.faces, theirs.edges, theirs.vertices, theirs.shells)
        ));
    }
    if !close(ours.volume, theirs.volume, 1e-7, size)
        || !close(ours.area, theirs.area, 1e-7, scale * scale)
    {
        problems.push(format!(
            "{name} @ {scale} {unit:?}: volume {} became {}, area {} became {}",
            ours.volume, theirs.volume, ours.area, theirs.area
        ));
    }
    if let [one] = imported.as_slice()
        && craft3d_topo::genus(body) != craft3d_topo::genus(&one.body)
    {
        problems.push(format!(
            "{name} @ {scale} {unit:?}: genus {:?} became {:?}",
            craft3d_topo::genus(body),
            craft3d_topo::genus(&one.body)
        ));
    }
    // Nothing merged: our files share every vertex and edge.
    for done in &imported {
        let r = &done.report;
        if r.merged_vertices + r.merged_edges + r.turned_shells != 0 {
            problems.push(format!(
                "{name} @ {scale} {unit:?}: sewing changed what the file shares: {r:?}"
            ));
        }
    }
}

#[test]
fn every_exported_body_reads_back_the_same_at_three_scales() {
    let mut problems = Vec::new();
    let mut count = 0;
    for scale in [1e-3, 1.0, 1e6] {
        for (name, body) in test_bodies(scale) {
            round_trip(&name, &body, scale, Unit::Millimeter, &mut problems);
            count += 1;
        }
    }
    println!("{count} bodies read back");
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn a_file_in_inches_reads_back_in_millimetres() {
    // The body is in inches when written in inches; read into
    // millimetres, every length is 25.4 times as long.
    let body = craft3d_kernel::primitives::block(&Frame::WORLD, 1.0, 2.0, 3.0).unwrap();
    let options = StepOptions {
        length_unit: Unit::Inch,
        ..StepOptions::default()
    };
    let (bytes, _) = export_step(&[StepBody::new("inch block", &body)], &options).unwrap();
    let parsed = parse(&bytes).unwrap();
    let done = parsed
        .build(
            0,
            &Options {
                metres_per_unit: 1e-3,
                tolerance: Tolerance::default(),
            },
        )
        .unwrap();
    let volume = craft3d_topo::mass_properties(&done.body).unwrap().volume;
    let expected = 6.0 * 25.4f64.powi(3);
    assert!((volume - expected).abs() < 1e-9 * expected, "{volume}");
    assert_eq!(done.report.length_unit, "INCH");
    assert_eq!(done.name, "inch block");
    // Same unit both ways in other tests: the round trip in other units.
    let mut problems = Vec::new();
    for unit in [Unit::Inch, Unit::Meter, Unit::Micron] {
        round_trip("unit block", &body, 1.0, unit, &mut problems);
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

// ------------------------------------------- other writers' conventions

use craft3d_io::step_model;
use craft3d_step::{Entity, Instance, Param, Record, StepFile};

fn file_of(name: &str, body: &Body) -> StepFile {
    let options = StepOptions {
        tolerance: tolerance(1.0),
        ..StepOptions::default()
    };
    step_model(&[StepBody::new(name, body)], &options)
        .unwrap()
        .0
}

fn read_all(file: &StepFile) -> Result<Vec<craft3d_stepread::Imported>, String> {
    let text = craft3d_step::write(file).unwrap();
    let parsed = parse(text.as_bytes()).map_err(|e| e.to_string())?;
    let options = Options {
        metres_per_unit: 1e-3,
        tolerance: tolerance(1.0),
    };
    (0..parsed.items().len())
        .map(|i| parsed.build(i, &options).map_err(|e| e.to_string()))
        .collect()
}

fn simple_mut<'a>(instance: &'a mut Instance, keyword: &str) -> Option<&'a mut Record> {
    match &mut instance.entity {
        Entity::Simple(record) if record.keyword == keyword => Some(record),
        _ => None,
    }
}

fn next_id(file: &StepFile) -> u64 {
    file.data.iter().map(|i| i.id).max().unwrap_or(0) + 1
}

/// The bodies these variants are tried on: planes, a cylinder with its
/// circles, a cone with its apex, cavities, rational splines, a sheet.
fn variant_bodies() -> Vec<(String, Body)> {
    let wanted = [
        "cavities box and cone",
        "cavity sphere",
        "nurbs can",
        "washer",
        "ellipse tall",
        "open box",
    ];
    let mut out: Vec<(String, Body)> = test_bodies(1.0)
        .into_iter()
        .filter(|(name, _)| wanted.contains(&name.as_str()))
        .collect();
    let frame = tilted(1.0);
    out.push((
        "block".to_string(),
        craft3d_kernel::primitives::block(&frame, 1.0, 2.0, 3.0).unwrap(),
    ));
    out.push((
        "cylinder".to_string(),
        craft3d_kernel::primitives::cylinder(&frame, 1.0, 2.0).unwrap(),
    ));
    out.push((
        "cone".to_string(),
        craft3d_kernel::primitives::cone(&frame, 1.0, 2.0).unwrap(),
    ));
    out
}

fn same_as(name: &str, body: &Body, imported: &[craft3d_stepread::Imported]) -> Result<(), String> {
    let ours = measure(&[body]);
    let theirs = measure(&imported.iter().map(|i| &i.body).collect::<Vec<_>>());
    if (ours.faces, ours.edges, ours.vertices, ours.shells)
        != (theirs.faces, theirs.edges, theirs.vertices, theirs.shells)
    {
        return Err(format!("{name}: counts {ours:?} became {theirs:?}"));
    }
    if !close(ours.volume, theirs.volume, 1e-7, 1.0) || !close(ours.area, theirs.area, 1e-7, 1.0) {
        return Err(format!("{name}: {ours:?} became {theirs:?}"));
    }
    Ok(())
}

#[test]
fn edges_written_against_their_curves_read_as_the_same_body() {
    // EDGE_CURVE(start, end, curve, .T.) and the same edge written as
    // EDGE_CURVE(end, start, curve, .F.) used the other way by every loop.
    let mut problems = Vec::new();
    let mut seen = 0;
    for (name, body) in variant_bodies() {
        let mut file = file_of(&name, &body);
        for instance in &mut file.data {
            if let Some(record) = simple_mut(instance, "EDGE_CURVE") {
                record.params.swap(1, 2);
                record.params[4] = Param::boolean(false);
                seen += 1;
            } else if let Some(record) = simple_mut(instance, "ORIENTED_EDGE") {
                let along = record.params[4] == Param::boolean(true);
                record.params[4] = Param::boolean(!along);
            }
        }
        match read_all(&file) {
            Ok(imported) => {
                if let Err(e) = same_as(&name, &body, &imported) {
                    problems.push(e);
                }
            }
            Err(e) => problems.push(format!("{name}: {e}")),
        }
    }
    assert!(seen > 20);
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn faces_written_turned_over_inside_oriented_faces_read_as_the_same_body() {
    // An ADVANCED_FACE with its normal and bounds the other way, used
    // through an ORIENTED_FACE(.F.) that turns it back.
    let mut problems = Vec::new();
    for (name, body) in variant_bodies() {
        let mut file = file_of(&name, &body);
        let mut wrap = Vec::new();
        for instance in &mut file.data {
            if let Some(record) = simple_mut(instance, "ADVANCED_FACE") {
                let same = record.params[3] == Param::boolean(true);
                record.params[3] = Param::boolean(!same);
                wrap.push(instance.id);
            }
        }
        let bounds: Vec<u64> = file
            .data
            .iter()
            .filter(|i| wrap.contains(&i.id))
            .flat_map(|i| {
                i.entity.records()[0].params[1]
                    .as_list()
                    .unwrap()
                    .iter()
                    .filter_map(Param::as_ref)
                    .collect::<Vec<_>>()
            })
            .collect();
        for instance in &mut file.data {
            if bounds.contains(&instance.id)
                && let Entity::Simple(record) = &mut instance.entity
            {
                let along = record.params[2] == Param::boolean(true);
                record.params[2] = Param::boolean(!along);
            }
        }
        let mut id = next_id(&file);
        let mut replaced = std::collections::BTreeMap::new();
        for &face in &wrap {
            file.data.push(Instance {
                id,
                entity: Entity::Simple(Record::new(
                    "ORIENTED_FACE",
                    vec![
                        Param::String(String::new()),
                        Param::Derived,
                        Param::Ref(face),
                        Param::boolean(false),
                    ],
                )),
            });
            replaced.insert(face, id);
            id += 1;
        }
        for instance in &mut file.data {
            for keyword in ["CLOSED_SHELL", "OPEN_SHELL"] {
                if let Some(record) = simple_mut(instance, keyword)
                    && let Param::List(items) = &mut record.params[1]
                {
                    for item in items {
                        if let Param::Ref(face) = item
                            && let Some(&to) = replaced.get(face)
                        {
                            *item = Param::Ref(to);
                        }
                    }
                }
            }
        }
        match read_all(&file) {
            Ok(imported) => {
                if let Err(e) = same_as(&name, &body, &imported) {
                    problems.push(e);
                }
            }
            Err(e) => problems.push(format!("{name}: {e}")),
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn repeated_vertices_and_edges_are_sewn_and_the_gap_measured() {
    // The second use of every edge gets an EDGE_CURVE of its own, with
    // VERTEX_POINTs of its own at the same places: the faces share
    // nothing, as some writers leave them. Sewing makes them one again.
    let mut problems = Vec::new();
    for (name, body) in variant_bodies() {
        let mut file = file_of(&name, &body);
        let mut id = next_id(&file);
        let mut added = Vec::new();
        let mut edge_seen = std::collections::BTreeSet::new();
        let index: std::collections::BTreeMap<u64, Instance> =
            file.data.iter().map(|i| (i.id, i.clone())).collect();
        for instance in &mut file.data {
            let Some(record) = simple_mut(instance, "ORIENTED_EDGE") else {
                continue;
            };
            let Param::Ref(edge) = record.params[3] else {
                continue;
            };
            if edge_seen.insert(edge) {
                continue;
            }
            let Entity::Simple(edge_record) = &index[&edge].entity else {
                continue;
            };
            let mut copy = edge_record.clone();
            for at in [1, 2] {
                let Param::Ref(vertex) = copy.params[at] else {
                    continue;
                };
                added.push(Instance {
                    id,
                    entity: index[&vertex].entity.clone(),
                });
                copy.params[at] = Param::Ref(id);
                id += 1;
            }
            // A closed edge keeps one vertex at both ends.
            if edge_record.params[1] == edge_record.params[2] {
                copy.params[2] = copy.params[1].clone();
            }
            added.push(Instance {
                id,
                entity: Entity::Simple(copy),
            });
            record.params[3] = Param::Ref(id);
            id += 1;
        }
        file.data.extend(added);
        match read_all(&file) {
            Ok(imported) => {
                if let Err(e) = same_as(&name, &body, &imported) {
                    problems.push(e);
                }
                let merged: usize = imported.iter().map(|i| i.report.merged_edges).sum();
                let vertices: usize = imported.iter().map(|i| i.report.merged_vertices).sum();
                let open = body.edges.iter().filter(|e| e.start != e.end).count();
                if body.kind == BodyKind::Solid && (merged < open || vertices == 0) {
                    problems.push(format!(
                        "{name}: {merged} edges of {open} and {vertices} vertices merged"
                    ));
                }
                for i in &imported {
                    if i.report.vertex_gap != 0.0 {
                        problems.push(format!("{name}: a gap where there is none"));
                    }
                }
            }
            Err(e) => problems.push(format!("{name}: {e}")),
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn a_cavity_given_as_a_plain_shell_is_turned_and_said_so() {
    // BREP_WITH_VOIDS lists ORIENTED_CLOSED_SHELL(.F.); a file that lists
    // the closed shell itself gets it turned by the sign of its volume.
    let body = bodies::block_with_spherical_cavity(1.0);
    let mut file = file_of("cavity", &body);
    let oriented: std::collections::BTreeMap<u64, Param> = file
        .data
        .iter()
        .filter(|i| i.entity.is("ORIENTED_CLOSED_SHELL"))
        .map(|i| (i.id, i.entity.records()[0].params[2].clone()))
        .collect();
    for instance in &mut file.data {
        if let Some(record) = simple_mut(instance, "BREP_WITH_VOIDS")
            && let Param::List(voids) = &mut record.params[2]
        {
            for void in voids {
                if let Param::Ref(id) = void {
                    *void = oriented[id].clone();
                }
            }
        }
    }
    let imported = read_all(&file).unwrap();
    same_as("cavity", &body, &imported).unwrap();
    assert_eq!(imported[0].report.turned_shells, 1);
}

#[test]
fn a_vertex_off_its_edges_is_refused_with_the_gap() {
    // One corner of a block moved by 1e-3 mm: the edges at it no longer
    // end at it. Nothing is moved back; the body is refused, naming the
    // vertex, the edge and the distance.
    let body = craft3d_kernel::primitives::block(&Frame::WORLD, 10.0, 20.0, 30.0).unwrap();
    let mut file = file_of("block", &body);
    let vertex_point = file
        .data
        .iter()
        .find(|i| i.entity.is("VERTEX_POINT"))
        .map(|i| i.entity.records()[0].params[1].as_ref().unwrap())
        .unwrap();
    for instance in &mut file.data {
        if instance.id == vertex_point
            && let Some(record) = simple_mut(instance, "CARTESIAN_POINT")
            && let Param::List(xyz) = &mut record.params[1]
        {
            let x = xyz[0].as_real().unwrap();
            xyz[0] = Param::Real(x + 1e-3);
        }
    }
    let error = read_all(&file).unwrap_err();
    assert!(error.contains("vertex #"), "{error}");
    assert!(error.contains("e-3") || error.contains("e-4"), "{error}");
    assert!(error.contains("tolerance per edge and vertex"), "{error}");
}

// --------------------------------------------------- hostile variations

/// A seeded generator, so a failure can be run again.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
}

#[test]
fn damaged_files_are_errors_never_panics() {
    // Files of a few bodies cut short at many places, with bytes
    // replaced, with numbers made absurd, with references pointed
    // elsewhere: reading ends in a body or an error.
    let mut rng = Lcg(0x5eed);
    let mut tried = 0;
    let options = Options {
        metres_per_unit: 1e-3,
        tolerance: tolerance(1.0),
    };
    let read = |bytes: &[u8]| {
        if let Ok(parsed) = parse(bytes) {
            for i in 0..parsed.items().len() {
                let _ = parsed.build(i, &options);
            }
        }
    };
    for (name, body) in variant_bodies() {
        let file = file_of(&name, &body);
        let text = craft3d_step::write(&file).unwrap();
        let bytes = text.as_bytes();
        for k in 1..40 {
            read(&bytes[..bytes.len() * k / 40]);
            tried += 1;
        }
        for _ in 0..40 {
            let mut damaged = bytes.to_vec();
            for _ in 0..1 + rng.next() % 4 {
                let at = (rng.next() as usize) % damaged.len();
                damaged[at] = b"#0123456789.,()'$*E-+ ;=TF"[(rng.next() % 26) as usize];
            }
            read(&damaged);
            tried += 1;
        }
        let ids: Vec<u64> = file.data.iter().map(|i| i.id).collect();
        for _ in 0..60 {
            let mut changed = file.clone();
            let at = (rng.next() as usize) % changed.data.len();
            let records = match &mut changed.data[at].entity {
                Entity::Simple(r) => std::slice::from_mut(r),
                Entity::Complex(rs) => rs.as_mut_slice(),
            };
            for record in records {
                for param in &mut record.params {
                    match param {
                        Param::Ref(r) => *r = ids[(rng.next() as usize) % ids.len()],
                        Param::Real(x) => {
                            *x = [1e308, -1e308, 0.0, 1e-320, 1e15][(rng.next() % 5) as usize] * *x
                                + 1.0;
                        }
                        Param::Integer(i) => {
                            *i = [i64::MAX, -1, 0, 1_000_000_000][(rng.next() % 4) as usize];
                        }
                        Param::Enum(e) if e == "T" || e == "F" => {
                            *e = if e == "T" { "F" } else { "U" }.to_string();
                        }
                        Param::List(items) if rng.next() % 3 == 0 => items.clear(),
                        _ => {}
                    }
                }
            }
            if let Ok(text) = craft3d_step::write(&changed) {
                read(text.as_bytes());
                tried += 1;
            }
        }
    }
    assert!(tried > 500, "{tried}");
}

// ------------------------------------------------------- swept surfaces

fn reals_of(file: &StepFile, id: u64) -> Vec3 {
    let instance = file.data.iter().find(|i| i.id == id).unwrap();
    let list = instance.entity.records()[0].params[1].as_list().unwrap();
    Vec3::new(
        list[0].as_real().unwrap(),
        list[1].as_real().unwrap(),
        list[2].as_real().unwrap(),
    )
}

/// The frame of an `AXIS2_PLACEMENT_3D` of the file.
fn frame_of(file: &StepFile, id: u64) -> Frame {
    let instance = file.data.iter().find(|i| i.id == id).unwrap();
    let p = &instance.entity.records()[0].params;
    Frame::new(
        reals_of(file, p[1].as_ref().unwrap()),
        reals_of(file, p[2].as_ref().unwrap()),
        reals_of(file, p[3].as_ref().unwrap()),
    )
    .unwrap()
}

struct Adder {
    file: StepFile,
    next: u64,
}

impl Adder {
    fn add(&mut self, keyword: &str, params: Vec<Param>) -> u64 {
        let id = self.next;
        self.next += 1;
        self.file.data.push(Instance {
            id,
            entity: Entity::Simple(Record::new(keyword, params)),
        });
        id
    }
    fn point(&mut self, p: Vec3) -> u64 {
        self.add(
            "CARTESIAN_POINT",
            vec![Param::string(""), Param::reals([p.x, p.y, p.z])],
        )
    }
    fn direction(&mut self, d: Vec3) -> u64 {
        self.add(
            "DIRECTION",
            vec![Param::string(""), Param::reals([d.x, d.y, d.z])],
        )
    }
    fn vector(&mut self, d: Vec3) -> u64 {
        let direction = self.direction(d);
        self.add(
            "VECTOR",
            vec![Param::string(""), Param::Ref(direction), Param::Real(1.0)],
        )
    }
    fn line(&mut self, at: Vec3, d: Vec3) -> u64 {
        let point = self.point(at);
        let vector = self.vector(d);
        self.add(
            "LINE",
            vec![Param::string(""), Param::Ref(point), Param::Ref(vector)],
        )
    }
    fn placement(&mut self, f: &Frame) -> u64 {
        let origin = self.point(f.origin());
        let z = self.direction(f.z());
        let x = self.direction(f.x());
        self.add(
            "AXIS2_PLACEMENT_3D",
            vec![
                Param::string(""),
                Param::Ref(origin),
                Param::Ref(z),
                Param::Ref(x),
            ],
        )
    }
    fn circle(&mut self, f: &Frame, r: f64) -> u64 {
        let placement = self.placement(f);
        self.add(
            "CIRCLE",
            vec![Param::string(""), Param::Ref(placement), Param::Real(r)],
        )
    }
    fn axis1(&mut self, at: Vec3, d: Vec3) -> u64 {
        let point = self.point(at);
        let direction = self.direction(d);
        self.add(
            "AXIS1_PLACEMENT",
            vec![Param::string(""), Param::Ref(point), Param::Ref(direction)],
        )
    }
}

/// Every elementary surface of the file written as the swept surface
/// that is the same point set: `turn` sweeps it the other way, so that
/// its normal is the opposite one, and the faces on it say `same_sense`
/// the other way round.
fn as_swept(file: &StepFile, turn: bool) -> (StepFile, usize) {
    let mut adder = Adder {
        next: next_id(file),
        file: file.clone(),
    };
    let sign = if turn { -1.0 } else { 1.0 };
    let mut replaced = 0;
    let mut swept = Vec::new();
    let originals: Vec<Instance> = file.data.clone();
    for instance in &originals {
        let Entity::Simple(record) = &instance.entity else {
            continue;
        };
        let p = &record.params;
        let real = |k: usize| p[k].as_real().unwrap();
        let entity = match record.keyword.as_str() {
            "CYLINDRICAL_SURFACE" => {
                let f = frame_of(file, p[1].as_ref().unwrap());
                if turn {
                    // The circle extruded along the axis, backwards.
                    let circle = adder.circle(&f, real(2));
                    let vector = adder.vector(-f.z());
                    Record::new(
                        "SURFACE_OF_LINEAR_EXTRUSION",
                        vec![Param::string(""), Param::Ref(circle), Param::Ref(vector)],
                    )
                } else {
                    // A line parallel to the axis, turned about it.
                    let line = adder.line(f.point(real(2), 0.0, 0.0), f.z());
                    let axis = adder.axis1(f.origin(), f.z());
                    Record::new(
                        "SURFACE_OF_REVOLUTION",
                        vec![Param::string(""), Param::Ref(line), Param::Ref(axis)],
                    )
                }
            }
            "CONICAL_SURFACE" => {
                let f = frame_of(file, p[1].as_ref().unwrap());
                let (r, a) = (real(2), real(3));
                let along = f.vector(a.sin(), 0.0, a.cos()) * sign;
                let line = adder.line(f.point(r, 0.0, 0.0), along);
                let axis = adder.axis1(f.origin(), f.z());
                Record::new(
                    "SURFACE_OF_REVOLUTION",
                    vec![Param::string(""), Param::Ref(line), Param::Ref(axis)],
                )
            }
            "SPHERICAL_SURFACE" | "TOROIDAL_SURFACE" => {
                let f = frame_of(file, p[1].as_ref().unwrap());
                let (centre, radius) = if record.keyword == "SPHERICAL_SURFACE" {
                    (f.origin(), real(2))
                } else {
                    (f.point(real(2), 0.0, 0.0), real(3))
                };
                // A circle in the plane of x and z, run either way.
                let plane = Frame::new(centre, f.y() * -sign, f.x()).unwrap();
                let circle = adder.circle(&plane, radius);
                let axis = adder.axis1(f.origin(), f.z());
                Record::new(
                    "SURFACE_OF_REVOLUTION",
                    vec![Param::string(""), Param::Ref(circle), Param::Ref(axis)],
                )
            }
            "PLANE" if turn => {
                let f = frame_of(file, p[1].as_ref().unwrap());
                // The x axis extruded along -y: normal x × -y = -z.
                let line = adder.line(f.origin(), f.x());
                let vector = adder.vector(-f.y());
                Record::new(
                    "SURFACE_OF_LINEAR_EXTRUSION",
                    vec![Param::string(""), Param::Ref(line), Param::Ref(vector)],
                )
            }
            _ => continue,
        };
        replaced += 1;
        let at = adder
            .file
            .data
            .iter()
            .position(|i| i.id == instance.id)
            .unwrap();
        adder.file.data[at].entity = Entity::Simple(entity);
        swept.push(instance.id);
    }
    // A surface whose normal is the opposite one: the faces on it say
    // `same_sense` the other way, so that they mean the same face.
    if turn {
        for instance in &mut adder.file.data {
            if let Some(record) = simple_mut(instance, "ADVANCED_FACE")
                && swept.contains(&record.params[2].as_ref().unwrap())
            {
                let same = record.params[3] == Param::boolean(true);
                record.params[3] = Param::boolean(!same);
            }
        }
    }
    (adder.file, replaced)
}

#[test]
fn swept_surfaces_read_as_the_elementary_surfaces_they_are() {
    let frame = tilted(1.0);
    let mut cases: Vec<(String, Body)> = vec![
        (
            "cylinder".to_string(),
            craft3d_kernel::primitives::cylinder(&frame, 1.0, 2.0).unwrap(),
        ),
        (
            "cone".to_string(),
            craft3d_kernel::primitives::cone(&frame, 1.0, 2.0).unwrap(),
        ),
        (
            "sphere".to_string(),
            craft3d_kernel::primitives::sphere(&frame, 1.5).unwrap(),
        ),
        (
            "torus".to_string(),
            craft3d_kernel::primitives::torus(&frame, 3.0, 1.0).unwrap(),
        ),
        (
            "block".to_string(),
            craft3d_kernel::primitives::block(&frame, 1.0, 2.0, 3.0).unwrap(),
        ),
    ];
    cases.extend(
        test_bodies(1.0)
            .into_iter()
            .filter(|(name, _)| name == "cavities box and cone" || name == "washer"),
    );
    let mut problems = Vec::new();
    for (name, body) in cases {
        for turn in [false, true] {
            let (file, replaced) = as_swept(&file_of(&name, &body), turn);
            // Planes are swept only when turned.
            assert!(replaced > 0 || !turn, "{name}");
            match read_all(&file) {
                Ok(imported) => {
                    if let Err(e) = same_as(&name, &body, &imported) {
                        problems.push(format!("turned {turn}: {e}"));
                    }
                }
                Err(e) => problems.push(format!("{name}, turned {turn}: {e}")),
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}
