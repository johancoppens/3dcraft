//! Oracle 8 (docs/testing.md): an independent program reads our exports.
//!
//! Opt-in, because it needs FreeCAD:
//!
//! ```sh
//! CRAFT3D_FREECAD=/path/to/FreeCAD.AppImage cargo test -p craft3d-io --test freecad -- --ignored
//! ```
//!
//! `CRAFT3D_FREECAD` is a FreeCAD AppImage or a `FreeCADCmd` executable.
//! When opted in, a missing or failing FreeCAD is a test failure, never a
//! skip. FreeCAD runs as a separate program; nothing of it is linked.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_io::{MeshFormat, export_mesh, to_object};
use craft3d_math::Vec3;
use craft3d_meshio::Unit;
use craft3d_tess::{Mesh, TessParams, tessellate_patches};
use craft3d_testkit::teapot::teapot;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;

/// What FreeCAD reports for a file.
#[derive(Debug)]
struct Report {
    facets: usize,
    points: usize,
    area: f64,
    volume: f64,
    solid: bool,
    bbox: [f64; 6],
}

fn freecad_reads(file_name: &str, bytes: &[u8]) -> Report {
    let freecad = std::env::var("CRAFT3D_FREECAD").expect(
        "set CRAFT3D_FREECAD to a FreeCAD AppImage or FreeCADCmd executable to run these tests",
    );
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("freecad");
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join(file_name);
    let output = dir.join(format!("{file_name}.report"));
    std::fs::write(&input, bytes).unwrap();
    let _ = std::fs::remove_file(&output);

    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/mesh_report.py");
    let mut command = Command::new(&freecad);
    if freecad.ends_with(".AppImage") {
        command.arg("freecadcmd");
    }
    let run = command
        .arg(&script)
        .env("CRAFT3D_FC_IN", &input)
        .env("CRAFT3D_FC_OUT", &output)
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap_or_else(|e| panic!("cannot run {freecad}: {e}"));
    let text = std::fs::read_to_string(&output).unwrap_or_else(|_| {
        panic!(
            "FreeCAD wrote no report for {file_name} ({}):\n{}\n{}",
            run.status,
            String::from_utf8_lossy(&run.stdout),
            String::from_utf8_lossy(&run.stderr)
        )
    });
    let fields: BTreeMap<&str, &str> = text.lines().filter_map(|l| l.split_once('=')).collect();
    let number = |key: &str| -> f64 { fields[key].parse().unwrap() };
    let bbox: Vec<f64> = fields["bbox"]
        .split(' ')
        .map(|v| v.parse().unwrap())
        .collect();
    Report {
        facets: number("facets") as usize,
        points: number("points") as usize,
        area: number("area"),
        volume: number("volume"),
        solid: fields["solid"] == "1",
        bbox: bbox.try_into().unwrap(),
    }
}

/// FreeCAD's mesh kernel stores 32-bit floats.
fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 2e-5 * a.abs().max(b.abs()).max(1.0)
}

const FORMATS: [(&str, MeshFormat); 4] = [
    ("stl", MeshFormat::StlBinary),
    ("ascii.stl", MeshFormat::StlAscii),
    ("3mf", MeshFormat::ThreeMf),
    ("obj", MeshFormat::Obj),
];

/// A 2 × 3 × 4 box at an offset, as six separate quads.
fn cuboid() -> Mesh {
    let (o, d) = (Vec3::new(10.0, -5.0, 1.5), Vec3::new(2.0, 3.0, 4.0));
    let corner = |i: u32| {
        Vec3::new(
            o.x + if i & 1 != 0 { d.x } else { 0.0 },
            o.y + if i & 2 != 0 { d.y } else { 0.0 },
            o.z + if i & 4 != 0 { d.z } else { 0.0 },
        )
    };
    // Corner indices per face, counter-clockwise seen from outside.
    let faces = [
        [0, 2, 3, 1],
        [4, 5, 7, 6],
        [0, 1, 5, 4],
        [2, 6, 7, 3],
        [0, 4, 6, 2],
        [1, 3, 7, 5],
    ];
    let mut mesh = Mesh::default();
    for (face, quad) in faces.iter().enumerate() {
        let base = mesh.positions.len() as u32;
        for &i in quad {
            mesh.positions.push(corner(i));
            mesh.normals.push(Vec3::ZERO);
        }
        mesh.triangles
            .extend([[base, base + 1, base + 2], [base, base + 2, base + 3]]);
        mesh.face_ids.extend([face as u32; 2]);
    }
    mesh
}

#[test]
#[ignore = "needs FreeCAD: set CRAFT3D_FREECAD and run with --ignored"]
fn freecad_reads_a_box_as_a_solid_of_the_right_volume() {
    let mesh = cuboid();
    for (extension, format) in FORMATS {
        let bytes = export_mesh(&mesh, "box", Unit::Millimeter, format).unwrap();
        let report = freecad_reads(&format!("box.{extension}"), &bytes);
        println!("box.{extension}: {report:?}");
        assert_eq!(report.facets, 12, "{extension}");
        assert_eq!(report.points, 8, "{extension}");
        assert!(
            report.solid,
            "{extension}: FreeCAD does not see a closed solid"
        );
        // Positive: our triangles face outward.
        assert!(
            close(report.volume, 24.0),
            "{extension}: volume {}",
            report.volume
        );
        assert!(
            close(report.area, 52.0),
            "{extension}: area {}",
            report.area
        );
        for (got, want) in report.bbox.iter().zip([10.0, -5.0, 1.5, 12.0, -2.0, 5.5]) {
            assert!(close(*got, want), "{extension}: bbox {:?}", report.bbox);
        }
    }
}

/// A box with colours (3MF base materials, an object's and a face's own):
/// FreeCAD reads it as the same closed solid. Its command-line reader does
/// not hand the colours on (a mesh object has no colour without the GUI),
/// so they are checked by our own read-back (`craft3d-meshio`).
#[test]
#[ignore = "needs FreeCAD: set CRAFT3D_FREECAD and run with --ignored"]
fn freecad_reads_a_coloured_3mf_box_as_the_same_solid() {
    let mesh = cuboid();
    let mut object =
        craft3d_io::to_object_with_materials(&mesh, "box", |face| (face == 1).then_some(1))
            .unwrap();
    object.material = Some(0);
    let model = craft3d_meshio::Model {
        unit: Unit::Millimeter,
        objects: vec![object],
        materials: vec![
            craft3d_meshio::Material {
                name: "Cast iron".into(),
                color: [80, 82, 86],
                ..craft3d_meshio::Material::default()
            },
            craft3d_meshio::Material {
                name: "Brass".into(),
                color: [181, 140, 60],
                metallic: Some(1.0),
                roughness: Some(0.3),
                ..craft3d_meshio::Material::default()
            },
        ],
    };
    let bytes = craft3d_io::encode(&model, MeshFormat::ThreeMf).unwrap();
    assert!(String::from_utf8_lossy(&bytes).contains("displaycolor=\"#B58C3C\""));
    let report = freecad_reads("coloured-box.3mf", &bytes);
    println!("coloured-box.3mf: {report:?}");
    assert_eq!((report.facets, report.points), (12, 8));
    assert!(report.solid);
    assert!(close(report.volume, 24.0), "volume {}", report.volume);

    // The same box as OBJ with its material library beside it (`mtllib`,
    // `usemtl`, the PBR lines): read as the same solid. The colours are
    // checked by an independent reader (`craft3d_testkit::readers`, in
    // `crates/engine/tests/colour_export.rs`).
    let (obj, mtl) = craft3d_meshio::write_obj_with_mtl(&model, "coloured-box.mtl").unwrap();
    let mtl = mtl.unwrap();
    assert!(mtl.contains("Pm 1.000000"), "{mtl}");
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("freecad");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("coloured-box.mtl"), mtl).unwrap();
    let report = freecad_reads("coloured-box.obj", obj.as_bytes());
    println!("coloured-box.obj: {report:?}");
    assert_eq!((report.facets, report.points), (12, 8));
    assert!(report.solid);
    assert!(close(report.volume, 24.0), "volume {}", report.volume);
}

#[test]
#[ignore = "needs FreeCAD: set CRAFT3D_FREECAD and run with --ignored"]
fn freecad_reads_the_teapot_as_we_wrote_it() {
    let params = TessParams {
        chordal_tolerance: 0.01,
        ..TessParams::default()
    };
    let mesh = tessellate_patches(&teapot().unwrap(), &params).unwrap();
    let object = to_object(&mesh, "teapot").unwrap();
    let (lo, hi) = mesh.bbox().corners().unwrap();
    for (extension, format) in FORMATS {
        let bytes = export_mesh(&mesh, "teapot", Unit::Millimeter, format).unwrap();
        let report = freecad_reads(&format!("teapot.{extension}"), &bytes);
        println!("teapot.{extension}: {report:?}");
        assert_eq!(report.facets, object.triangles.len(), "{extension}");
        assert_eq!(report.points, object.positions.len(), "{extension}");
        assert!(
            !report.solid,
            "{extension}: the teapot dataset is not closed"
        );
        assert!(
            close(report.area, mesh.area()),
            "{extension}: area {} vs {}",
            report.area,
            mesh.area()
        );
        for (got, want) in report.bbox.iter().zip([lo.x, lo.y, lo.z, hi.x, hi.y, hi.z]) {
            assert!(close(*got, want), "{extension}: bbox {:?}", report.bbox);
        }
    }
}

/// The full path for solids: exact B-rep → tessellator → file → FreeCAD.
/// FreeCAD must see a closed solid whose volume is the mesh's, and that
/// volume must be within `area × tolerance` of the closed form.
#[test]
#[ignore = "needs FreeCAD: set CRAFT3D_FREECAD and run with --ignored"]
fn freecad_reads_tessellated_solids_as_solids_of_the_right_volume() {
    use craft3d_math::Frame;
    use craft3d_tess::tessellate_body;
    use craft3d_testkit::solids;

    let frame = Frame::new(Vec3::new(3.0, -1.0, 2.0), Vec3::new(0.2, 0.3, 1.0), Vec3::X).unwrap();
    let tolerance = 0.01;
    let params = TessParams {
        chordal_tolerance: tolerance,
        ..TessParams::default()
    };
    // The primitives, the hemisphere, and the solids with trimmed faces
    // (holes, half walls, an ellipse edge, freeform roofs).
    let mut all = solids::all(&frame, 1.0).unwrap();
    all.extend(craft3d_testkit::trimmed::all(&frame, 1.0).unwrap());
    for solid in all {
        let mesh = tessellate_body(&solid.body, &params).unwrap();
        let welded = mesh.weld().unwrap();
        for (extension, format) in [("stl", MeshFormat::StlBinary), ("3mf", MeshFormat::ThreeMf)] {
            let bytes = export_mesh(&mesh, solid.name, Unit::Millimeter, format).unwrap();
            let report = freecad_reads(&format!("{}.{extension}", solid.name), &bytes);
            println!("{}.{extension}: {report:?}", solid.name);
            let what = format!("{}.{extension}", solid.name);
            assert!(report.solid, "{what}: FreeCAD does not see a closed solid");
            assert_eq!(report.facets, welded.triangles.len(), "{what}");
            assert_eq!(report.points, welded.positions.len(), "{what}");
            assert!(
                close(report.volume, welded.volume()),
                "{what}: {} vs {}",
                report.volume,
                welded.volume()
            );
            assert!(
                (report.volume - solid.volume).abs() <= solid.area * tolerance,
                "{what}: FreeCAD's volume {} is not within area x tolerance of the exact {}",
                report.volume,
                solid.volume
            );
            assert!(
                close(report.area, mesh.area()),
                "{what}: area {} vs {}",
                report.area,
                mesh.area()
            );
        }
    }
}

/// A needle (a cubic that leaves the axis along it, turned about it: a
/// cusp at its point) cut in two by a block whose face holds the axis:
/// the half's freeform face runs into the point from two edges. FreeCAD
/// must read its STL and 3MF as a closed solid of the half's volume,
/// `π ∫ ρ² dz / 2` with `ρ = 9 t² − 6 t³` and `z = 6 t`, to within the
/// area times the tolerance. (The tessellator left that face open at the
/// point until 2026-10-17.)
#[test]
#[ignore = "needs FreeCAD: set CRAFT3D_FREECAD and run with --ignored"]
fn freecad_reads_a_needle_cut_along_its_axis_as_a_solid() {
    use craft3d_kernel::boolean::{BooleanOp, boolean};
    use craft3d_kernel::profile::{Curve2, SketchCurve, Spline2, detect_profiles};
    use craft3d_kernel::{primitives, revolve::revolve};
    use craft3d_math::{Frame, Tolerance};
    use craft3d_tess::tessellate_body;

    let k = 10.0;
    let tol = Tolerance::new(1e-5 * k, 1e-9).unwrap();
    let p = |x: f64, y: f64| [x * k, y * k];
    let curves = vec![
        SketchCurve {
            id: 3,
            curve: Curve2::Spline(Spline2 {
                degree: 3,
                knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
                control: vec![p(0.0, 0.0), p(0.0, 2.0), p(3.0, 4.0), p(3.0, 6.0)],
            }),
        },
        SketchCurve {
            id: 5,
            curve: Curve2::Line {
                start: p(3.0, 6.0),
                end: p(0.0, 6.0),
            },
        },
        SketchCurve {
            id: 7,
            curve: Curve2::Line {
                start: p(0.0, 6.0),
                end: p(0.0, 0.0),
            },
        },
    ];
    let profiles = detect_profiles(&curves, &tol).unwrap();
    let needle = revolve(
        &profiles,
        &Frame::WORLD,
        [0.0, 0.0],
        [0.0, 1.0],
        0.0,
        std::f64::consts::TAU,
        &tol,
    )
    .unwrap()
    .body;
    // Everything with x ≥ 0: its face x = 0 holds the axis.
    let corner = Frame::new(Vec3::new(0.0, -k, -5.0 * k), Vec3::Z, Vec3::X).unwrap();
    let block = primitives::block(&corner, 5.0 * k, 8.0 * k, 10.0 * k).unwrap();
    let half = boolean(&needle, &block, BooleanOp::Subtract, &tol)
        .unwrap()
        .body;
    // π ∫ (9t² − 6t³)² 6 dt over [0, 1], halved.
    let volume = 0.5 * std::f64::consts::PI * 6.0 * (81.0 / 5.0 - 18.0 + 36.0 / 7.0) * k.powi(3);
    let tolerance = 0.01 * k;
    let params = TessParams {
        chordal_tolerance: tolerance,
        ..TessParams::default()
    };
    let mesh = tessellate_body(&half, &params).unwrap();
    let welded = mesh.weld().unwrap();
    assert!(welded.is_closed());
    for (extension, format) in [("stl", MeshFormat::StlBinary), ("3mf", MeshFormat::ThreeMf)] {
        let bytes = export_mesh(&mesh, "needle half", Unit::Millimeter, format).unwrap();
        let what = format!("needle-half.{extension}");
        let report = freecad_reads(&what, &bytes);
        println!("{what}: {report:?} against {volume}");
        assert!(report.solid, "{what}: FreeCAD does not see a closed solid");
        assert_eq!(report.facets, welded.triangles.len(), "{what}");
        assert!(
            close(report.volume, welded.volume()),
            "{what}: {} vs {}",
            report.volume,
            welded.volume()
        );
        assert!(
            (report.volume - volume).abs() <= mesh.area() * tolerance,
            "{what}: FreeCAD's volume {} is not within area x tolerance of the exact {volume}",
            report.volume
        );
    }
}
