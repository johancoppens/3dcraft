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

use craft3d_math::Portable;
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
        loosest: 1000.0 * (tolerance(scale)).linear,
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
                // Every edge proven, none accepted on samples alone
                // (slice 2: seams from poles, closed spline seams).
                if done.report.unproven_edges > 0 {
                    problems.push(format!(
                        "{name} @ {scale} {unit:?}: item {index}: {} edges not proven",
                        done.report.unproven_edges
                    ));
                }
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
        if r.merged_vertices + r.merged_edges + r.sewn_edges + r.turned_shells != 0 {
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
                loosest: 1000.0 * (Tolerance::default()).linear,
            },
        )
        .unwrap();
    let volume = craft3d_topo::mass_properties(&done.body).unwrap().volume;
    let expected = 6.0 * 25.4f64.ppowi(3);
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
        loosest: 1000.0 * (tolerance(1.0)).linear,
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
        let mut replaced = std::collections::BTreeMap::new();
        for (id, &face) in (next_id(&file)..).zip(&wrap) {
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

/// A block whose first corner is moved by `by` mm in x in the file.
fn block_with_a_moved_corner(by: f64) -> StepFile {
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
            xyz[0] = Param::Real(x + by);
        }
    }
    file
}

#[test]
fn a_vertex_off_its_edges_by_more_than_the_cap_is_refused_with_the_gap() {
    // One corner of a block moved by 1e-2 mm, ten times the cap: the
    // edges at it no longer end at it. Nothing is moved back; the body is
    // refused, naming the vertex and the distance.
    let error = read_all(&block_with_a_moved_corner(1e-2)).unwrap_err();
    assert!(
        error.contains("vertex #") || error.contains("edge #"),
        "{error}"
    );
    assert!(error.contains("refused rather than moved"), "{error}");
}

#[test]
fn a_vertex_off_its_edges_within_the_cap_gets_its_gap_as_its_tolerance() {
    // Moved by 1e-4 mm (a hundred times the tolerance, a tenth of the
    // cap): read, nothing moved, the vertex given a tolerance of its own
    // at least the gap, and the body checked with it.
    let imported = read_all(&block_with_a_moved_corner(1e-4)).unwrap();
    let one = &imported[0];
    assert!(one.report.loose_vertices >= 1, "{:?}", one.report);
    assert!(one.report.largest_tolerance >= 1e-4, "{:?}", one.report);
    assert!(one.report.largest_tolerance <= 1e-3, "{:?}", one.report);
    assert!(!one.body.tolerances.is_empty());
    let moved = one
        .body
        .vertices
        .iter()
        .any(|v| (v.point.x - 1e-4).abs() < 1e-12);
    assert!(moved, "the vertex stays where the file put it");
    let defects = craft3d_topo::check_capped(&one.body, &tolerance(1.0), 1e-3);
    assert!(defects.is_empty(), "{defects:?}");
}

const HEAD: &str = "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('','',(''),(''),'','','');\nFILE_SCHEMA(('AUTOMOTIVE_DESIGN'));\nENDSEC;\nDATA;\n#1=( LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT(.MILLI.,.METRE.) );\n#4=( GEOMETRIC_REPRESENTATION_CONTEXT(3) GLOBAL_UNIT_ASSIGNED_CONTEXT((#1)) REPRESENTATION_CONTEXT('','') );\n";
const TAIL: &str = "ENDSEC;\nEND-ISO-10303-21;\n";

/// A sheet on the plane z = 0 bounded by a polyline of 9 points whose odd
/// points are `lift` above the plane, closed by an arc of a parabola (a
/// quadratic B-spline) in the plane: five samples of the polyline (at
/// its even points) see nothing.
fn zigzag_sheet(lift: f64) -> String {
    let mut text = String::from(HEAD);
    let mut ids = Vec::new();
    for k in 0..9 {
        let id = 100 + k;
        let z = if k % 2 == 1 { lift } else { 0.0 };
        text.push_str(&format!(
            "#{id}=CARTESIAN_POINT('',({}.,0.,{z:?}));\n",
            k * 10
        ));
        ids.push(format!("#{id}"));
    }
    text.push_str(&format!(
        "#5=MANIFOLD_SURFACE_SHAPE_REPRESENTATION('zig',(#10),#4);\n\
         #10=SHELL_BASED_SURFACE_MODEL('zig',(#11));\n\
         #11=OPEN_SHELL('',(#12));\n\
         #12=ADVANCED_FACE('',(#13),#20,.T.);\n\
         #13=FACE_OUTER_BOUND('',#14,.T.);\n\
         #14=EDGE_LOOP('',(#15,#16));\n\
         #15=ORIENTED_EDGE('',*,*,#17,.T.);\n\
         #16=ORIENTED_EDGE('',*,*,#18,.T.);\n\
         #17=EDGE_CURVE('',#30,#31,#40,.T.);\n\
         #18=EDGE_CURVE('',#31,#30,#41,.T.);\n\
         #30=VERTEX_POINT('',#100);\n\
         #31=VERTEX_POINT('',#108);\n\
         #40=POLYLINE('',({}));\n\
         #41=B_SPLINE_CURVE_WITH_KNOTS('',2,(#108,#50,#100),.UNSPECIFIED.,.F.,.F.,(3,3),(0.,1.),.UNSPECIFIED.);\n\
         #50=CARTESIAN_POINT('',(40.,30.,0.));\n\
         #20=PLANE('',#21);\n\
         #21=AXIS2_PLACEMENT_3D('',#22,$,$);\n\
         #22=CARTESIAN_POINT('',(0.,0.,0.));\n{TAIL}",
        ids.join(",")
    ));
    text
}

#[test]
fn an_edge_off_its_face_between_the_samples_is_refused() {
    let options = Options {
        metres_per_unit: 1e-3,
        tolerance: tolerance(1.0),
        loosest: 1e-3,
    };
    // Flat: read, exact.
    let flat = parse(zigzag_sheet(0.0).as_bytes()).unwrap();
    let sheet = flat.build(0, &options).unwrap();
    assert_eq!(sheet.body.kind, BodyKind::Sheet);
    assert!(sheet.body.tolerances.is_empty(), "{:?}", sheet.report);
    // 1 mm off the plane at every other point: a million times the
    // tolerance, refused naming the edge.
    let zig = parse(zigzag_sheet(1.0).as_bytes()).unwrap();
    let error = zig.build(0, &options).unwrap_err().to_string();
    assert!(error.contains("edge #17"), "{error}");
    assert!(error.contains("refused rather than moved"), "{error}");
    // Within the cap (2e-4 mm): read, the edge given its proven gap.
    let small = parse(zigzag_sheet(2e-4).as_bytes()).unwrap();
    let sheet = small.build(0, &options).unwrap();
    assert_eq!(sheet.report.loose_edges, 1, "{:?}", sheet.report);
    assert!(sheet.report.largest_tolerance >= 2e-4, "{:?}", sheet.report);
}

#[test]
fn a_surface_shared_by_many_faces_is_refused_before_it_is_copied() {
    // The reviewer's probe: one B-spline surface of 1000 x 1000 control
    // points (all the same point) shared by many faces without bounds.
    // Each face would hold a copy (some 64 MB); the file is refused when
    // it is parsed, in bounded time and memory.
    let faces = 150;
    let mut text = String::from(HEAD);
    text.push_str("#30=CARTESIAN_POINT('',(0.,0.,0.));\n");
    let row = vec!["#30"; 1000].join(",");
    let net = vec![format!("({row})"); 1000].join(",");
    let values: Vec<String> = (0..1000).map(|k| format!("{k}.")).collect();
    let mut mults = vec!["1"; 1000];
    mults[0] = "2";
    mults[999] = "2";
    let (values, mults) = (values.join(","), mults.join(","));
    text.push_str(&format!(
        "#31=B_SPLINE_SURFACE_WITH_KNOTS('',1,1,({net}),.UNSPECIFIED.,.F.,.F.,.F.,({mults}),({mults}),({values}),({values}),.UNSPECIFIED.);\n"
    ));
    let face_ids: Vec<String> = (0..faces).map(|k| format!("#{}", 1000 + k)).collect();
    for f in &face_ids {
        text.push_str(&format!("{f}=ADVANCED_FACE('',(),#31,.T.);\n"));
    }
    let shell = format!("#11=OPEN_SHELL('',({}));\n", face_ids.join(","));
    text.push_str(&shell);
    text.push_str(
        "#10=SHELL_BASED_SURFACE_MODEL('probe',(#11));\n#5=MANIFOLD_SURFACE_SHAPE_REPRESENTATION('probe',(#10),#4);\n",
    );
    text.push_str(TAIL);
    let start = std::time::Instant::now();
    let error = parse(text.as_bytes()).unwrap_err().to_string();
    assert!(error.contains("control points"), "{error}");
    assert!(start.elapsed().as_secs() < 30, "{:?}", start.elapsed());
    // One such face is within the budget, and refused as a face without
    // an edge loop on a surface that does not close.
    let one = text.replace(&shell, "#11=OPEN_SHELL('',(#1000));\n");
    let parsed = parse(one.as_bytes()).unwrap();
    let options = Options {
        metres_per_unit: 1e-3,
        tolerance: tolerance(1.0),
        loosest: 1e-3,
    };
    let error = parsed.build(0, &options).unwrap_err().to_string();
    assert!(error.contains("without an edge loop"), "{error}");
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
        loosest: 1000.0 * (tolerance(1.0)).linear,
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
                        Param::List(items) if rng.next().is_multiple_of(3) => items.clear(),
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
                let along = f.vector(a.psin(), 0.0, a.pcos()) * sign;
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

// ------------------------------------------- swept splines, as splines

impl Adder {
    /// A rational B-spline curve, as the complex instance other writers
    /// use (`B_SPLINE_CURVE_WITH_KNOTS` with `RATIONAL_B_SPLINE_CURVE`).
    fn rational(&mut self, curve: &craft3d_geom::NurbsCurve) -> u64 {
        let points: Vec<u64> = curve
            .control_points()
            .iter()
            .map(|&p| self.point(p))
            .collect();
        let mut distinct: Vec<f64> = Vec::new();
        let mut multiplicities: Vec<Param> = Vec::new();
        for &k in curve.knots() {
            if distinct.last() == Some(&k) {
                if let Some(Param::Integer(m)) = multiplicities.last_mut() {
                    *m += 1;
                }
            } else {
                distinct.push(k);
                multiplicities.push(Param::Integer(1));
            }
        }
        let id = self.next;
        self.next += 1;
        let records = vec![
            Record::new("BOUNDED_CURVE", vec![]),
            Record::new(
                "B_SPLINE_CURVE",
                vec![
                    Param::Integer(curve.degree() as i64),
                    Param::refs(points),
                    Param::enumeration("UNSPECIFIED"),
                    Param::boolean(false),
                    Param::boolean(false),
                ],
            ),
            Record::new(
                "B_SPLINE_CURVE_WITH_KNOTS",
                vec![
                    Param::List(multiplicities),
                    Param::reals(distinct),
                    Param::enumeration("UNSPECIFIED"),
                ],
            ),
            Record::new("CURVE", vec![]),
            Record::new("GEOMETRIC_REPRESENTATION_ITEM", vec![]),
            Record::new(
                "RATIONAL_B_SPLINE_CURVE",
                vec![Param::reals(curve.weights().iter().copied())],
            ),
            Record::new("REPRESENTATION_ITEM", vec![Param::string("")]),
        ];
        self.file.data.push(Instance {
            id,
            entity: Entity::Complex(records),
        });
        id
    }

    /// The segment from `a` to `b` as a `QUASI_UNIFORM_CURVE` of degree
    /// one (its knots implied: 0, 0, 1, 1).
    fn quasi_uniform_segment(&mut self, a: Vec3, b: Vec3) -> u64 {
        let (a, b) = (self.point(a), self.point(b));
        self.add(
            "QUASI_UNIFORM_CURVE",
            vec![
                Param::string(""),
                Param::Integer(1),
                Param::refs([a, b]),
                Param::enumeration("POLYLINE_FORM"),
                Param::boolean(false),
                Param::boolean(false),
            ],
        )
    }
}

/// Every cylinder, sphere, torus and cone of the file written as a swept
/// surface of a curve that is a spline in the file (no elementary kind):
/// a cylinder as its circle, a rational B-spline, extruded along the axis
/// (`turn`: backwards); a sphere or torus as its meridian circle, a
/// rational B-spline, turned about the axis (`turn`: run the other way);
/// a cone as a quasi-uniform segment of its generator turned about the
/// axis. These read as exact B-spline surfaces, not as the elementary
/// kinds.
fn as_swept_splines(file: &StepFile, turn: bool) -> (StepFile, usize) {
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
                let circle = craft3d_geom::NurbsCurve::circle(&f, real(2)).unwrap();
                let curve = adder.rational(&circle);
                let vector = adder.vector(f.z() * sign);
                Record::new(
                    "SURFACE_OF_LINEAR_EXTRUSION",
                    vec![Param::string(""), Param::Ref(curve), Param::Ref(vector)],
                )
            }
            "SPHERICAL_SURFACE" | "TOROIDAL_SURFACE" => {
                let f = frame_of(file, p[1].as_ref().unwrap());
                let (centre, radius) = if record.keyword == "SPHERICAL_SURFACE" {
                    (f.origin(), real(2))
                } else {
                    (f.point(real(2), 0.0, 0.0), real(3))
                };
                let plane = Frame::new(centre, f.y() * -sign, f.x()).unwrap();
                let circle = if record.keyword == "SPHERICAL_SURFACE" {
                    // A sphere is a half circle turned: from pole to pole
                    // through the x axis, as two rational quarters (or
                    // back, turned).
                    let s = std::f64::consts::FRAC_1_SQRT_2;
                    let (top, bottom) = (radius * sign, -radius * sign);
                    let points = [
                        (0.0, bottom),
                        (radius, bottom),
                        (radius, 0.0),
                        (radius, top),
                        (0.0, top),
                    ]
                    .map(|(x, z)| centre + f.x() * x + f.z() * z)
                    .to_vec();
                    craft3d_geom::NurbsCurve::new(
                        2,
                        vec![0.0, 0.0, 0.0, 0.5, 0.5, 1.0, 1.0, 1.0],
                        points,
                        vec![1.0, s, 1.0, s, 1.0],
                    )
                    .unwrap()
                } else {
                    craft3d_geom::NurbsCurve::circle(&plane, radius).unwrap()
                };
                let curve = adder.rational(&circle);
                let axis = adder.axis1(f.origin(), f.z());
                Record::new(
                    "SURFACE_OF_REVOLUTION",
                    vec![Param::string(""), Param::Ref(curve), Param::Ref(axis)],
                )
            }
            "CONICAL_SURFACE" => {
                let f = frame_of(file, p[1].as_ref().unwrap());
                let (r, a) = (real(2), real(3));
                let up = f.vector(a.psin(), 0.0, a.pcos());
                // Long enough to hold every face on the cone: from the
                // axis (the apex) to well past the base, or back.
                let base = f.point(r, 0.0, 0.0);
                let reach = 20.0 * (r + 1.0);
                let apex = base - up * (r / a.psin());
                let (from, to) = if turn {
                    (base + up * reach, apex)
                } else {
                    (apex, base + up * reach)
                };
                let curve = adder.quasi_uniform_segment(from, to);
                let axis = adder.axis1(f.origin(), f.z());
                Record::new(
                    "SURFACE_OF_REVOLUTION",
                    vec![Param::string(""), Param::Ref(curve), Param::Ref(axis)],
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

/// Swept surfaces of curves that are no elementary kind (2026-10-21,
/// slice 2): the one FreeCAD file refused in slice 1 was a revolved
/// B-spline. Read as exact rational B-spline surfaces, every edge proven
/// on them, the same volume and area as the elementary surfaces they
/// are the same point sets as.
#[test]
fn swept_splines_read_as_exact_spline_surfaces() {
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
    ];
    // Whole spheres and tori are faces without loops, which a spline
    // surface (with no period) does not take: with their seams.
    cases.extend(
        seamed(&frame, 1.0)
            .into_iter()
            .map(|(name, body, _)| (name, body)),
    );
    let mut problems = Vec::new();
    for (name, body) in cases {
        for turn in [false, true] {
            let (file, replaced) = as_swept_splines(&file_of(&name, &body), turn);
            assert!(replaced > 0, "{name}");
            match read_all(&file) {
                Ok(imported) => {
                    let splines = imported
                        .iter()
                        .flat_map(|i| &i.body.faces)
                        .filter(|f| matches!(f.surface, craft3d_geom::Surface::Nurbs(_)))
                        .count();
                    if splines == 0 {
                        problems.push(format!("{name}, turned {turn}: no spline surface"));
                    }
                    for i in &imported {
                        let r = &i.report;
                        if r.unproven_edges + r.loose_edges + r.loose_vertices > 0 {
                            problems.push(format!("{name}, turned {turn}: {r:?}"));
                        }
                    }
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

// ------------------------------------------------------ offset surfaces

/// Every elementary surface of the file written as an `OFFSET_SURFACE`
/// of the surface of its kind `d` nearer (`outward`: the basis inside
/// and the offset positive; else the basis outside and the offset
/// negative): a plane moved back along its normal, a radius made smaller
/// or larger (a cone's by `d / cos α`, so that its offset is it again).
fn as_offsets(file: &StepFile, d: f64, outward: bool) -> (StepFile, usize) {
    let mut adder = Adder {
        next: next_id(file),
        file: file.clone(),
    };
    let sign = if outward { 1.0 } else { -1.0 };
    let mut replaced = 0;
    let originals: Vec<Instance> = file.data.clone();
    for instance in &originals {
        let Entity::Simple(record) = &instance.entity else {
            continue;
        };
        let p = record.params.clone();
        let real = |k: usize| p[k].as_real().unwrap();
        let basis = match record.keyword.as_str() {
            "PLANE" => {
                let f = frame_of(file, p[1].as_ref().unwrap());
                let moved = f.at(f.origin() - f.z() * (d * sign)).unwrap();
                let placement = adder.placement(&moved);
                adder.add("PLANE", vec![Param::string(""), Param::Ref(placement)])
            }
            "CYLINDRICAL_SURFACE" | "SPHERICAL_SURFACE" => adder.add(
                &record.keyword,
                vec![
                    Param::string(""),
                    p[1].clone(),
                    Param::Real(real(2) - d * sign),
                ],
            ),
            "TOROIDAL_SURFACE" => adder.add(
                "TOROIDAL_SURFACE",
                vec![
                    Param::string(""),
                    p[1].clone(),
                    p[2].clone(),
                    Param::Real(real(3) - d * sign),
                ],
            ),
            "CONICAL_SURFACE" => adder.add(
                "CONICAL_SURFACE",
                vec![
                    Param::string(""),
                    p[1].clone(),
                    Param::Real(real(2) - d * sign / real(3).pcos()),
                    p[3].clone(),
                ],
            ),
            _ => continue,
        };
        replaced += 1;
        let at = adder
            .file
            .data
            .iter()
            .position(|i| i.id == instance.id)
            .unwrap();
        adder.file.data[at].entity = Entity::Simple(Record::new(
            "OFFSET_SURFACE",
            vec![
                Param::string(""),
                Param::Ref(basis),
                Param::Real(d * sign),
                Param::boolean(false),
            ],
        ));
    }
    (adder.file, replaced)
}

/// Offsets of the elementary surfaces read as the surfaces they are
/// (slice 2), either way round; an offset of a spline surface is refused
/// naming it.
#[test]
fn offset_surfaces_read_as_the_surfaces_they_are() {
    let frame = tilted(1.0);
    let cases: Vec<(String, Body)> = vec![
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
    let mut problems = Vec::new();
    for (name, body) in cases {
        for outward in [true, false] {
            let (file, replaced) = as_offsets(&file_of(&name, &body), 0.125, outward);
            assert!(replaced > 0, "{name}");
            match read_all(&file) {
                Ok(imported) => {
                    if let Err(e) = same_as(&name, &body, &imported) {
                        problems.push(format!("outward {outward}: {e}"));
                    }
                }
                Err(e) => problems.push(format!("{name}, outward {outward}: {e}")),
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
    // An offset of a B-spline surface: refused, named.
    let (_, can) = test_bodies(1.0)
        .into_iter()
        .find(|(name, _)| name == "nurbs can")
        .unwrap();
    let mut file = file_of("can", &can);
    let mut adder = Adder {
        next: next_id(&file),
        file: file.clone(),
    };
    let spline = file
        .data
        .iter()
        .find(|i| i.entity.is("B_SPLINE_SURFACE_WITH_KNOTS"))
        .unwrap()
        .id;
    let offset = adder.add(
        "OFFSET_SURFACE",
        vec![
            Param::string(""),
            Param::Ref(spline),
            Param::Real(0.1),
            Param::boolean(false),
        ],
    );
    file = adder.file;
    for instance in &mut file.data {
        if let Some(record) = simple_mut(instance, "ADVANCED_FACE")
            && record.params[2] == Param::Ref(spline)
        {
            record.params[2] = Param::Ref(offset);
        }
    }
    let e = read_all(&file).unwrap_err();
    assert!(
        e.contains("OFFSET_SURFACE") && e.contains("B-spline"),
        "{e}"
    );
}

// ------------------------------------------------ seams, as others write

use craft3d_geom::{Circle, Curve3, Cylinder, Line, Surface};
use craft3d_topo::{Coedge, EdgeId, VertexId};
use std::f64::consts::TAU;

fn forward(edge: EdgeId) -> Coedge {
    Coedge {
        edge,
        reversed: false,
    }
}

fn backward(edge: EdgeId) -> Coedge {
    Coedge {
        edge,
        reversed: true,
    }
}

fn line(body: &mut Body, from: VertexId, to: VertexId) -> EdgeId {
    let (a, b) = (
        body.vertices[from.0 as usize].point,
        body.vertices[to.0 as usize].point,
    );
    let curve = Curve3::Line(Line::through(a, b).unwrap());
    body.add_edge(curve, (0.0, a.distance(b)), from, to)
}

/// A cylinder, a cone and a torus whose closed faces have their seams as
/// edges, as other modellers write them (`crates/tess/tests/periodic.rs`
/// builds the same), with their closed-form volumes.
fn seamed(frame: &Frame, k: f64) -> Vec<(String, Body, f64)> {
    let mut out = Vec::new();
    let (radius, height) = (1.5 * k, 2.0 * k);
    let mut body = Body::new(BodyKind::Solid);
    let above = frame.at(frame.point(0.0, 0.0, height)).unwrap();
    let low = body.add_vertex(frame.point(radius, 0.0, 0.0));
    let high = body.add_vertex(above.point(radius, 0.0, 0.0));
    let circle = |frame: Frame| Curve3::Circle(Circle::new(frame, radius).unwrap());
    let bottom = body.add_edge(circle(*frame), (0.0, TAU), low, low);
    let top = body.add_edge(circle(above), (0.0, TAU), high, high);
    let seam = line(&mut body, low, high);
    let wall = body.add_loop(vec![
        forward(bottom),
        forward(seam),
        backward(top),
        backward(seam),
    ]);
    let wall = body.add_face(
        Surface::Cylinder(Cylinder::new(*frame, radius).unwrap()),
        false,
        vec![wall],
    );
    let under = Frame::new(frame.origin(), -frame.z(), frame.x()).unwrap();
    let floor = body.add_loop(vec![backward(bottom)]);
    let floor = body.add_face(
        Surface::Plane(craft3d_geom::Plane::new(under)),
        false,
        vec![floor],
    );
    let lid = body.add_loop(vec![forward(top)]);
    let lid = body.add_face(
        Surface::Plane(craft3d_geom::Plane::new(above)),
        false,
        vec![lid],
    );
    body.add_shell(vec![wall, floor, lid]);
    out.push((
        "seamed cylinder".to_string(),
        body,
        std::f64::consts::PI * radius * radius * height,
    ));

    let cone = solids::cone(frame, radius, height).unwrap();
    let mut body = cone.body;
    let apex = body.add_vertex(frame.point(0.0, 0.0, height));
    let base = body.edges[0].start;
    let seam = line(&mut body, base, apex);
    let side = body.faces[0].loops[0];
    body.loops[side.0 as usize].coedges = vec![forward(EdgeId(0)), forward(seam), backward(seam)];
    out.push(("seamed cone".to_string(), body, cone.volume));

    let (major, minor) = (3.0 * k, k);
    let torus = solids::torus(frame, major, minor).unwrap();
    let mut body = torus.body;
    let vertex = body.add_vertex(frame.point(major + minor, 0.0, 0.0));
    let around_axis = Curve3::Circle(Circle::new(*frame, major + minor).unwrap());
    let tube = Frame::new(frame.point(major, 0.0, 0.0), -frame.y(), frame.x()).unwrap();
    let around_tube = Curve3::Circle(Circle::new(tube, minor).unwrap());
    let long = body.add_edge(around_axis, (0.0, TAU), vertex, vertex);
    let short = body.add_edge(around_tube, (0.0, TAU), vertex, vertex);
    let loop_ = body.add_loop(vec![
        forward(long),
        forward(short),
        backward(long),
        backward(short),
    ]);
    body.faces[0].loops = vec![loop_];
    out.push(("seamed torus".to_string(), body, torus.volume));

    // A sphere bounded by one meridian from pole to pole, used both ways:
    // its seam starts and ends at the poles, where the prover's feet
    // have no longitude.
    let radius = 1.5 * k;
    let mut body = Body::new(BodyKind::Solid);
    let south = body.add_vertex(frame.point(0.0, 0.0, -radius));
    let north = body.add_vertex(frame.point(0.0, 0.0, radius));
    let meridian = Frame::new(frame.origin(), -frame.y(), frame.x()).unwrap();
    let half = std::f64::consts::FRAC_PI_2;
    let seam = body.add_edge(
        Curve3::Circle(Circle::new(meridian, radius).unwrap()),
        (-half, half),
        south,
        north,
    );
    let loop_ = body.add_loop(vec![forward(seam), backward(seam)]);
    let face = body.add_face(
        Surface::Sphere(craft3d_geom::Sphere::new(*frame, radius).unwrap()),
        false,
        vec![loop_],
    );
    body.add_shell(vec![face]);
    out.push((
        "seamed sphere".to_string(),
        body,
        4.0 / 3.0 * std::f64::consts::PI * radius.ppowi(3),
    ));
    out
}

#[test]
fn faces_with_their_seams_as_edges_read_back_at_three_scales() {
    let mut problems = Vec::new();
    for k in [1e-3, 1.0, 1e6] {
        for (name, body, volume) in seamed(&tilted(k), k) {
            assert!(
                craft3d_topo::check(&body, &tolerance(k)).is_empty(),
                "{name}"
            );
            let ours = measure(&[&body]).volume;
            assert!(
                (ours - volume).abs() <= 1e-9 * volume,
                "{name}: {ours} {volume}"
            );
            round_trip(&name, &body, k, Unit::Millimeter, &mut problems);
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// `id` and everything it refers to, copied as new instances with every
/// `CARTESIAN_POINT` moved by `by` (directions and numbers kept): the
/// copy of a face, its loops, edges, vertices, curves and surface, moved
/// rigidly. `memo` maps what was copied already.
fn copy_moved(
    index: &std::collections::BTreeMap<u64, Instance>,
    id: u64,
    by: Vec3,
    next: &mut u64,
    memo: &mut std::collections::BTreeMap<u64, u64>,
    added: &mut Vec<Instance>,
) -> u64 {
    if let Some(&to) = memo.get(&id) {
        return to;
    }
    fn map(
        param: &Param,
        index: &std::collections::BTreeMap<u64, Instance>,
        by: Vec3,
        next: &mut u64,
        memo: &mut std::collections::BTreeMap<u64, u64>,
        added: &mut Vec<Instance>,
    ) -> Param {
        match param {
            Param::Ref(r) if index.contains_key(r) => {
                Param::Ref(copy_moved(index, *r, by, next, memo, added))
            }
            Param::List(items) => Param::List(
                items
                    .iter()
                    .map(|p| map(p, index, by, next, memo, added))
                    .collect(),
            ),
            other => other.clone(),
        }
    }
    let instance = index[&id].clone();
    let mut records = instance.entity.records().to_vec();
    for record in &mut records {
        if record.keyword == "CARTESIAN_POINT" {
            if let Param::List(xyz) = &mut record.params[1] {
                for (k, d) in [by.x, by.y, by.z].into_iter().enumerate() {
                    if let Some(Param::Real(x)) = xyz.get_mut(k) {
                        *x += d;
                    }
                }
            }
            continue;
        }
        record.params = record
            .params
            .iter()
            .map(|p| map(p, index, by, next, memo, added))
            .collect();
    }
    let to = *next;
    *next += 1;
    memo.insert(id, to);
    let entity = match instance.entity {
        Entity::Simple(_) => Entity::Simple(records.into_iter().next().unwrap()),
        Entity::Complex(_) => Entity::Complex(records),
    };
    added.push(Instance { id: to, entity });
    to
}

/// `file` with every face given its own loops, edges, vertices, curves
/// and surface, each face moved rigidly by `moves(k)` for its k-th face:
/// faces that share nothing and meet within a gap, as files from other
/// kernels have them.
fn faces_apart(file: &StepFile, moves: impl Fn(usize) -> Vec3) -> StepFile {
    let mut file = file.clone();
    let index: std::collections::BTreeMap<u64, Instance> =
        file.data.iter().map(|i| (i.id, i.clone())).collect();
    let mut next = next_id(&file);
    let mut added = Vec::new();
    let mut replaced = std::collections::BTreeMap::new();
    let faces: Vec<u64> = file
        .data
        .iter()
        .filter(|i| i.entity.is("ADVANCED_FACE"))
        .map(|i| i.id)
        .collect();
    for (k, face) in faces.iter().enumerate() {
        let mut memo = std::collections::BTreeMap::new();
        let to = copy_moved(&index, *face, moves(k), &mut next, &mut memo, &mut added);
        replaced.insert(*face, to);
    }
    file.data.extend(added);
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
    file
}

#[test]
fn faces_that_share_nothing_and_meet_within_a_gap_are_sewn() {
    // Every face its own edges and vertices, moved by up to k tolerances
    // (0: exactly where they were, sewn by the file's own vertex merge;
    // 30 and 300: beyond it, sewn by `topo::sew` within the cap): the same
    // body, nothing moved back, every gap recorded, valid with them.
    let tol = tolerance(1.0);
    let mut problems = Vec::new();
    let mut tally = (0usize, 0usize, 0.0_f64);
    for (name, body) in variant_bodies() {
        for k in [0.0, 30.0, 300.0] {
            let file = file_of(&name, &body);
            let moves = |f: usize| {
                let a = 0.7 * f as f64 + 0.3;
                Vec3::new(a.pcos(), a.psin(), (1.3 * a).pcos()) * (k * tol.linear * 0.5)
            };
            let apart = faces_apart(&file, moves);
            match read_all(&apart) {
                Ok(imported) => {
                    let ours = measure(&[&body]);
                    let theirs = measure(&imported.iter().map(|i| &i.body).collect::<Vec<_>>());
                    if (ours.faces, ours.edges, ours.vertices, ours.shells)
                        != (theirs.faces, theirs.edges, theirs.vertices, theirs.shells)
                    {
                        problems.push(format!("{name} k {k}: counts {ours:?} became {theirs:?}"));
                        continue;
                    }
                    // A slab of the largest tolerance over the area.
                    let w = imported
                        .iter()
                        .map(|i| i.body.working_tolerance(tol.linear))
                        .fold(tol.linear, f64::max)
                        .max(k * tol.linear);
                    let bound = 4.0 * w * ours.area + 1e-9 * ours.volume.abs();
                    if (ours.volume - theirs.volume).abs() > bound {
                        problems.push(format!(
                            "{name} k {k}: volume {} against {}, bound {bound:e}",
                            theirs.volume, ours.volume
                        ));
                    }
                    for one in &imported {
                        let r = &one.report;
                        if k > 0.0 && body.kind == BodyKind::Solid && r.sewn_edges == 0 {
                            problems.push(format!("{name} k {k}: nothing sewn: {r:?}"));
                        }
                        if k > 0.0 && r.sewn_edges > 0 && r.sewn_gap <= 0.0 {
                            problems.push(format!("{name} k {k}: no gap reported"));
                        }
                        let defects = craft3d_topo::check_capped(&one.body, &tol, 1e-3);
                        if !defects.is_empty() {
                            problems.push(format!("{name} k {k}: {defects:?}"));
                        }
                        tally.0 += 1;
                        tally.1 += r.sewn_edges;
                        tally.2 = tally.2.max(r.sewn_gap);
                    }
                }
                Err(e) => problems.push(format!("{name} k {k}: {e}")),
            }
        }
    }
    println!(
        "faces apart: {} bodies read, {} edges sewn, largest gap {:e}",
        tally.0, tally.1, tally.2
    );
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn a_face_apart_by_more_than_the_cap_is_refused_naming_its_edge() {
    let body = craft3d_kernel::primitives::block(&Frame::WORLD, 10.0, 20.0, 30.0).unwrap();
    let file = file_of("block", &body);
    // The second face 2e-3 mm off its neighbours, twice the cap.
    let apart = faces_apart(&file, |f| {
        if f == 1 {
            Vec3::new(0.6, 0.0, 0.8) * 2e-3
        } else {
            Vec3::ZERO
        }
    });
    let error = read_all(&apart).unwrap_err();
    assert!(error.contains("edge #"), "{error}");
    assert!(error.contains("refused rather than moved"), "{error}");
}

/// `file`, a sheet of open shells as the export writes it, as a solid of
/// one closed shell with all their faces: the first `OPEN_SHELL` closed
/// with the faces of every one, the `SHELL_BASED_SURFACE_MODEL` a
/// `MANIFOLD_SOLID_BREP` of it and the representation an advanced
/// B-rep's (a solid whose faces our export, which checks the body, would
/// not write).
fn as_solid(file: &StepFile) -> StepFile {
    let mut file = file.clone();
    let mut faces = Vec::new();
    let mut first = None;
    for instance in &file.data {
        if let Entity::Simple(record) = &instance.entity
            && record.keyword == "OPEN_SHELL"
            && let Some(Param::List(items)) = record.params.get(1)
        {
            first.get_or_insert(instance.id);
            faces.extend(items.iter().cloned());
        }
    }
    let first = first.unwrap();
    for instance in &mut file.data {
        let id = instance.id;
        if let Entity::Simple(record) = &mut instance.entity {
            match record.keyword.as_str() {
                "OPEN_SHELL" if id == first => {
                    record.keyword = "CLOSED_SHELL".into();
                    record.params[1] = Param::List(faces.clone());
                }
                "SHELL_BASED_SURFACE_MODEL" => {
                    record.keyword = "MANIFOLD_SOLID_BREP".into();
                    record.params[1] = Param::Ref(first);
                }
                "MANIFOLD_SURFACE_SHAPE_REPRESENTATION" => {
                    record.keyword = "ADVANCED_BREP_SHAPE_REPRESENTATION".into();
                }
                _ => {}
            }
        }
    }
    file
}

/// A file whose front and back faces have their top side whole where the
/// top is two faces split across it (a T-junction on each side, as files
/// from modellers that do not split such edges have them), every face its
/// own edges and vertices, moved by up to k tolerances: read as the block
/// with the sides split (its counts, valid with what was recorded), both
/// sides split and every piece sewn, the volume within a slab of the
/// largest tolerance.
#[test]
fn a_side_against_two_faces_split_across_it_reads_as_one_body() {
    use craft3d_testkit::loose::{SPLIT_TOP_SIDES, exploded_from, split_top_block};
    let tol = tolerance(1.0);
    let expected = split_top_block(10.0, true).unwrap();
    let whole = split_top_block(10.0, false).unwrap();
    let ours = measure(&[&expected]);
    let mut problems = Vec::new();
    for k in [0.0, 30.0, 300.0] {
        let moves: Vec<Vec3> = (0..expected.faces.len())
            .map(|f| {
                let a = 0.7 * f as f64 + 0.3;
                Vec3::new(a.pcos(), a.psin(), (1.3 * a).pcos()) * (k * tol.linear * 0.5)
            })
            .collect();
        let loose = exploded_from(
            [&expected, &whole],
            |f| SPLIT_TOP_SIDES.contains(&f),
            &moves,
            None,
        )
        .unwrap();
        // A sheet of one shell per face, which the export writes.
        let mut sheet = loose.clone();
        sheet.kind = BodyKind::Sheet;
        sheet.shells = (0..sheet.faces.len() as u32)
            .map(|f| craft3d_topo::Shell {
                faces: vec![craft3d_topo::FaceId(f)],
            })
            .collect();
        let imported = match read_all(&as_solid(&file_of("split block", &sheet))) {
            Ok(i) => i,
            Err(e) => {
                problems.push(format!("k {k}: {e}"));
                continue;
            }
        };
        let theirs = measure(&imported.iter().map(|i| &i.body).collect::<Vec<_>>());
        if (ours.faces, ours.edges, ours.vertices, ours.shells)
            != (theirs.faces, theirs.edges, theirs.vertices, theirs.shells)
        {
            problems.push(format!("k {k}: counts {ours:?} became {theirs:?}"));
            continue;
        }
        let one = &imported[0];
        if one.report.split_edges != 2 {
            problems.push(format!("k {k}: split {:?}", one.report));
        }
        let defects = craft3d_topo::check_capped(&one.body, &tol, 1e-3);
        if !defects.is_empty() {
            problems.push(format!("k {k}: {defects:?}"));
        }
        let w = one.body.working_tolerance(tol.linear).max(k * tol.linear);
        let bound = 4.0 * w * ours.area + 1e-9 * ours.volume.abs();
        if (ours.volume - theirs.volume).abs() > bound {
            problems.push(format!(
                "k {k}: volume {} against {}, bound {bound:e}",
                theirs.volume, ours.volume
            ));
        }
        println!(
            "split block k {k}: {} edges sewn, {} split, largest tolerance {:e}",
            one.report.sewn_edges, one.report.split_edges, one.report.largest_tolerance
        );
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}
