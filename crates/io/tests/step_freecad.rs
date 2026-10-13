//! Oracle 8 (docs/testing.md) for STEP: FreeCAD, an independent kernel,
//! reads the exact B-rep we export and measures it.
//!
//! Opt-in, because it needs FreeCAD (`cargo xtask freecad` runs it):
//!
//! ```sh
//! CRAFT3D_FREECAD=/path/to/FreeCAD.AppImage cargo test --release -p craft3d-io --test step_freecad -- --ignored --nocapture
//! ```
//!
//! The bodies here are built directly on the kernel and by hand, which
//! covers what the command layer cannot make yet (spline surfaces,
//! cavities of every kind, sheets); the bodies made by commands are in
//! `crates/engine/tests/step_freecad.rs`. The table is printed and left in
//! `target/tmp/step-io/report.md`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

#[path = "common/bodies.rs"]
mod bodies;
#[path = "common/step_harness.rs"]
mod harness;

use craft3d_geom::Surface;
use craft3d_io::{AngleUnit, StepBody, StepOptions, export_step};
use craft3d_kernel::primitives;
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_meshio::Unit;
use craft3d_testkit::{solids, trimmed};
use craft3d_topo::{Body, BodyKind, mass_properties};
use harness::{APPROXIMATED, Case, Extent, RELATIVE};

fn tilted(scale: f64) -> Frame {
    Frame::new(
        Vec3::new(1.0, -2.0, 0.5) * scale,
        Vec3::new(0.3, -0.4, 1.0),
        Vec3::new(1.0, 0.2, 0.0),
    )
    .unwrap()
}

fn case(file: &str, body: &Body, lumps: usize, options: &StepOptions, to_mm: f64) -> Case {
    let (bytes, _) = export_step(&[StepBody { name: file, body }], options)
        .unwrap_or_else(|e| panic!("{file}: {e}"));
    let mass = mass_properties(body).unwrap_or_else(|e| panic!("{file}: {e}"));
    let bbox = body
        .bbox()
        .ok()
        .and_then(|b| b.corners())
        .map(|(lo, hi)| [lo.x, lo.y, lo.z, hi.x, hi.y, hi.z]);
    let solid = body.kind == BodyKind::Solid;
    Case {
        file: format!("{file}.step"),
        bytes,
        to_mm,
        solid,
        solids: if solid { lumps } else { 0 },
        volume: mass.volume,
        area: mass.area,
        centroid: mass.centroid.map(|c| [c.x, c.y, c.z]),
        bbox,
        counts: (
            body.faces.len(),
            body.edges.len(),
            body.vertices.len(),
            body.shells.len(),
        ),
        extent: extent(body),
        names: vec![file.to_string()],
        relative: RELATIVE,
    }
}

/// `Body::bbox` is tight for planes, cylinders and cones, and for whole
/// spheres and tori.
fn extent(body: &Body) -> Extent {
    if body
        .faces
        .iter()
        .any(|f| matches!(f.surface, Surface::Torus(_)))
    {
        Extent::Unchecked
    } else if body
        .faces
        .iter()
        .all(|f| matches!(f.surface, Surface::Plane(_)))
    {
        Extent::Exact
    } else if body.faces.iter().all(|f| {
        f.loops.is_empty()
            || matches!(
                f.surface,
                Surface::Plane(_) | Surface::Cylinder(_) | Surface::Cone(_)
            )
    }) {
        Extent::Tight
    } else {
        Extent::Loose
    }
}

fn options(scale: f64) -> StepOptions {
    StepOptions {
        // The tolerance follows the size, as a document's would be set.
        tolerance: Tolerance::new(1e-6 * scale.min(1.0), 1e-9).unwrap(),
        ..StepOptions::default()
    }
}

#[test]
#[ignore = "needs FreeCAD: set CRAFT3D_FREECAD"]
fn freecad_reads_every_kernel_body_as_the_solid_we_made() {
    let mut cases = Vec::new();
    for (scale, tag) in [(1e-3, "milli"), (1.0, "one"), (1e6, "mega")] {
        let frame = tilted(scale);
        let mut all = solids::all(&frame, scale).unwrap();
        all.extend(trimmed::all(&frame, scale).unwrap());
        all.push(trimmed::sphere_octant(&frame, 2.0 * scale).unwrap());
        all.push(trimmed::pipe_elbow(&frame, 3.0 * scale, 1.0 * scale, 1.2).unwrap());
        for (i, reference) in all.iter().enumerate() {
            let name = format!("{tag}-{i:02}-{}", reference.name.replace(' ', "_"));
            let mut c = case(&name, &reference.body, 1, &options(scale), 1.0);
            // The closed forms, not our integrals, where the kit has them.
            c.volume = reference.volume;
            c.area = reference.area;
            // The one body here with an edge that is not a line in the
            // parameter plane of its face: an ellipse on a cylinder.
            if reference.name == "oblique cylinder" {
                c.relative = APPROXIMATED;
            }
            cases.push(c);
        }
        for (name, body, lumps) in [
            (
                "cavity_sphere",
                bodies::block_with_spherical_cavity(scale),
                1,
            ),
            ("cavity_box_cone", bodies::block_with_two_cavities(scale), 1),
            ("two_lumps", bodies::two_lumps(scale), 2),
            ("nested", bodies::nested(scale), 3),
            (
                "quarter_round",
                bodies::quarter_round(&frame, 1.5 * scale, 2.0 * scale),
                1,
            ),
            ("open_box", bodies::open_box(scale), 0),
            (
                "washer",
                bodies::washer(&frame, 3.0 * scale, 1.0 * scale),
                0,
            ),
            (
                "ellipse_wide",
                bodies::elliptic_disc(&frame, 3.0 * scale, 1.0 * scale),
                0,
            ),
            (
                "ellipse_tall",
                bodies::elliptic_disc(&frame, 1.0 * scale, 3.0 * scale),
                0,
            ),
        ] {
            cases.push(case(
                &format!("{tag}-{name}"),
                &body,
                lumps,
                &options(scale),
                1.0,
            ));
        }
    }
    // A cylinder whose wall and rims are rational splines of four spans
    // joined with full-multiplicity knots (the nine-point circle). Neither
    // integrator is good across such knots: FreeCAD's area of the flat
    // lid, bounded by a curve whose length it gets right to 1e-15, is off
    // by 1.8e-4, its volume by 8e-3 (and `mass_properties` by 2.4e-3:
    // reported). So this body is compared with the closed form, coarsely;
    // the rational one-span `quarter_round` above is the precise check.
    {
        let (r, h) = (1.5, 2.0);
        let body = bodies::nurbs_can(&tilted(1.0), r, h);
        let mut c = case("nurbs_can", &body, 1, &options(1.0), 1.0);
        c.volume = std::f64::consts::PI * r * r * h;
        c.area = std::f64::consts::TAU * r * (r + h);
        c.centroid = None;
        c.relative = 2e-2;
        cases.push(c);
    }
    // Units: the same 40 x 20 x 10 mm block and a cone, from documents in
    // each unit; FreeCAD must report millimetres. And angles in degrees.
    for (unit, name, to_mm) in [
        (Unit::Micron, "um", 1e-3),
        (Unit::Millimeter, "mm", 1.0),
        (Unit::Centimeter, "cm", 10.0),
        (Unit::Meter, "m", 1000.0),
        (Unit::Inch, "in", 25.4),
        (Unit::Foot, "ft", 304.8),
    ] {
        for angle_unit in [AngleUnit::Radian, AngleUnit::Degree] {
            let options = StepOptions {
                length_unit: unit,
                angle_unit,
                tolerance: Tolerance::new(1e-6 / to_mm, 1e-9).unwrap(),
                ..StepOptions::default()
            };
            let block =
                primitives::block(&Frame::WORLD, 40.0 / to_mm, 20.0 / to_mm, 10.0 / to_mm).unwrap();
            let cone = primitives::cone(&tilted(1.0 / to_mm), 20.0 / to_mm, 30.0 / to_mm).unwrap();
            let tag = if angle_unit == AngleUnit::Degree {
                "deg"
            } else {
                "rad"
            };
            let c = case(
                &format!("unit-{name}-{tag}-block"),
                &block,
                1,
                &options,
                to_mm,
            );
            // In millimetres whatever the unit: 40 x 20 x 10.
            assert!((c.volume * to_mm.powi(3) - 8000.0).abs() < 1e-6);
            cases.push(c);
            cases.push(case(
                &format!("unit-{name}-{tag}-cone"),
                &cone,
                1,
                &options,
                to_mm,
            ));
        }
    }
    let rows = harness::check("step-io", "STEP: kernel bodies read by FreeCAD", &cases);
    let bad: Vec<String> = rows
        .iter()
        .filter(|r| !r.problems.is_empty())
        .map(|r| format!("{}: {}", r.file, r.problems.join("; ")))
        .collect();
    assert!(
        bad.is_empty(),
        "{} files with problems:\n{}",
        bad.len(),
        bad.join("\n")
    );
}
