//! Shared by the STEP checks of `craft3d-io` and `craft3d-engine` (which
//! includes this file by path): run FreeCAD over a set of STEP files and
//! compare what it measures with what we know of each body.

// `!(x <= bound)` on purpose: a NaN from FreeCAD is a problem too.
// Two tests share this file and each uses a part of it.
#![allow(clippy::neg_cmp_op_on_partial_ord, dead_code)]

use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const SCRIPT: &str = include_str!("../step_report.py");
const MESH_SCRIPT: &str = include_str!("../mesh_report.py");

/// Relative agreement asked of volume and area where FreeCAD has nothing
/// to approximate: every edge is a line in the parameter plane of both its
/// faces (anything on a plane; on a cylinder, cone, sphere or torus the
/// circles about the axis and the lines and circles through it), or the
/// boundary of a spline surface.
pub const RELATIVE: f64 = 1e-7;

/// The agreement asked where an edge is not such a line (an ellipse or a
/// spline on a cylinder: what booleans between curved bodies leave).
///
/// A STEP file as we write it has no parameter-space curves, so FreeCAD
/// makes its own on import, and for these edges it interpolates one. The
/// result does not follow the uncertainty the file declares, nor the
/// size: a cylinder cut by an oblique plane comes back with its volume
/// 2.8e-6 and its wall's area 7e-6 too large at tolerance 1e-6 and at
/// 1e-9, at scales 1e-3, 1 and 1e6 alike, while FreeCAD measures the
/// same solid built with its own primitives 2.7e-9 off the closed form,
/// and the flat face bounded by the very same ellipse to 1e-10. So the
/// difference is made on import, not written in the file. The bound is
/// some 25 times what was measured; a wrong sense, bound, radius, angle
/// or unit is off by orders of magnitude more.
pub const APPROXIMATED: f64 = 1e-4;

/// How our bounding box of a body relates to the exact one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Extent {
    /// Tight, and FreeCAD's is too (planar faces only): they must agree.
    Exact,
    /// Tight (`Body::bbox` on planes, cylinders, cones, whole spheres
    /// and tori); FreeCAD's may be larger, never smaller.
    Tight,
    /// Ours takes the whole surface for a part of a sphere, a torus or a
    /// spline surface: one box must contain the other.
    Loose,
    /// Not compared: the body has a face on a torus, and FreeCAD's box of
    /// a torus is not a bounding box. Measured on a whole torus (R = 3,
    /// r = 1) about the axis (0.3, -0.4, 1): FreeCAD gives a z extent of
    /// ±2.302 about the centre where the exact one is
    /// R √(1 − n_z²) + r = ±2.342, and ±4.098 in x where it is ±3.890.
    Unchecked,
}

/// One exported file and what we say is in it. Lengths are in the file's
/// unit; `to_mm` is that unit in millimetres (FreeCAD reports in mm).
#[derive(Clone, Debug)]
pub struct Case {
    /// File name, without directory.
    pub file: String,
    pub bytes: Vec<u8>,
    pub to_mm: f64,
    /// `false` for a sheet.
    pub solid: bool,
    /// How many solids FreeCAD should find (lumps); 0 for a sheet.
    pub solids: usize,
    pub volume: f64,
    pub area: f64,
    pub centroid: Option<[f64; 3]>,
    /// `[x0, y0, z0, x1, y1, z1]`, when known.
    pub bbox: Option<[f64; 6]>,
    /// How far inside the body our box may be, in the file's unit (a box
    /// taken from a mesh); 0 for an exact one.
    pub bbox_slack: f64,
    pub extent: Extent,
    /// `(faces, edges, vertices, shells)` of our body.
    pub counts: (usize, usize, usize, usize),
    /// The names FreeCAD should show, in any order.
    pub names: Vec<String>,
    /// Relative tolerance on volume and area for this case.
    pub relative: f64,
    /// Our bodies one by one, in the order they were written, to be held
    /// to FreeCAD's solids one by one (empty: only the totals are
    /// compared). A small body next to a large one is otherwise measured
    /// against the large one's size. Compared only when FreeCAD finds as
    /// many solids as there are parts (a body of several lumps is several
    /// solids in FreeCAD).
    pub parts: Vec<Part>,
}

/// One body of a [`Case`], in the file's unit.
#[derive(Clone, Debug)]
pub struct Part {
    pub name: String,
    pub volume: f64,
    pub area: f64,
    pub centroid: [f64; 3],
}

/// FreeCAD's solid `name` over ours: what [`compare`] found per part.
#[derive(Clone, Debug)]
pub struct PartRow {
    pub name: String,
    pub volume_ratio: f64,
    pub area_ratio: f64,
    /// Distance between the centres of mass over the part's size.
    pub centre_off: f64,
}

/// The address space FreeCAD gets, in kB (4 GiB). Reading and measuring
/// the files of these checks takes a few hundred MB. What it guards
/// against: on 2026-10-13 a version of the report script asked FreeCAD for
/// a mesh of one extruded spline at 1e-7 of its size, the mesher of
/// OpenCASCADE took 7.5 GB for it, and the machine went down.
const ADDRESS_SPACE_KB: u64 = 4 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct Row {
    pub file: String,
    pub valid: String,
    pub volume_ratio: f64,
    pub area_ratio: f64,
    pub centre_off: f64,
    pub bbox_off: f64,
    /// The volume by the report script's own quadrature over the geometry
    /// FreeCAD imported (`quadrature_volume` in `step_report.py`) over
    /// our volume, for the files it is taken of: those with `spline` in
    /// their name.
    pub quadrature_volume_ratio: Option<f64>,
    /// Ours, then FreeCAD's: faces/edges/vertices/shells.
    pub counts: String,
    pub added: String,
    /// Per part of the case, where they were compared.
    pub parts: Vec<PartRow>,
    pub problems: Vec<String>,
}

pub fn freecad() -> String {
    std::env::var("CRAFT3D_FREECAD").expect(
        "set CRAFT3D_FREECAD to a FreeCAD AppImage or FreeCADCmd executable to run these tests",
    )
}

/// Writes the files into `dir` and returns FreeCAD's record per file name.
pub fn run_freecad(dir: &Path, cases: &[Case]) -> (String, BTreeMap<String, Value>) {
    let freecad = freecad();
    std::fs::create_dir_all(dir).unwrap();
    let script = dir.join("step_report.py");
    std::fs::write(&script, SCRIPT).unwrap();
    let mut records = BTreeMap::new();
    let mut version = String::new();
    let mut rest: Vec<&Case> = cases.iter().collect();
    // A file that crashes FreeCAD ends the run; the files after it are
    // run again without it.
    while !rest.is_empty() {
        let listing = dir.join("files.txt");
        let output = dir.join("report.jsonl");
        let mut list = String::new();
        for case in &rest {
            let path = dir.join(&case.file);
            std::fs::write(&path, &case.bytes).unwrap();
            writeln!(list, "{}", path.display()).unwrap();
        }
        std::fs::write(&listing, list).unwrap();
        let _ = std::fs::remove_file(&output);
        // Under a limit on its address space where the shell has one, so
        // that a FreeCAD that runs away with memory dies alone (it ends
        // as a crash on the file it was reading, reported as such below)
        // and not with the machine.
        let mut command = if cfg!(target_os = "linux") {
            let mut shell = Command::new("sh");
            shell
                .arg("-c")
                .arg(format!("ulimit -v {ADDRESS_SPACE_KB} && exec \"$@\""))
                .arg("sh")
                .arg(&freecad);
            shell
        } else {
            Command::new(&freecad)
        };
        if freecad.ends_with(".AppImage") {
            command.arg("freecadcmd");
        }
        let run = command
            .arg(&script)
            .env("CRAFT3D_FC_IN", &listing)
            .env("CRAFT3D_FC_OUT", &output)
            .stdin(Stdio::null())
            .output()
            .unwrap_or_else(|e| panic!("cannot run {freecad}: {e}"));
        let text = std::fs::read_to_string(&output).unwrap_or_else(|_| {
            panic!(
                "FreeCAD wrote no report ({}):\n{}\n{}",
                run.status,
                String::from_utf8_lossy(&run.stdout),
                String::from_utf8_lossy(&run.stderr)
            )
        });
        let mut done = 0;
        for line in text.lines() {
            let value: Value = serde_json::from_str(line).unwrap();
            if let Some(v) = value.get("version") {
                version = v
                    .as_array()
                    .map(|parts| {
                        parts
                            .iter()
                            .filter_map(Value::as_str)
                            .collect::<Vec<_>>()
                            .join(".")
                    })
                    .unwrap_or_default();
                continue;
            }
            let file = value["file"].as_str().unwrap().to_string();
            records.insert(file, value);
            done += 1;
        }
        if done < rest.len() {
            // The one after the last reported crashed or hung FreeCAD.
            let culprit = rest[done];
            records.insert(
                culprit.file.clone(),
                serde_json::json!({"file": culprit.file, "error": format!(
                    "FreeCAD stopped on this file ({})", run.status)}),
            );
            done += 1;
        }
        rest.drain(..done);
    }
    (version, records)
}

fn numbers(value: &Value) -> Vec<f64> {
    value
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_f64).collect())
        .unwrap_or_default()
}

/// Compares FreeCAD's record of a file with the case.
pub fn compare(case: &Case, record: &Value) -> Row {
    let mut row = Row {
        file: case.file.clone(),
        valid: "-".to_string(),
        volume_ratio: f64::NAN,
        area_ratio: f64::NAN,
        centre_off: f64::NAN,
        bbox_off: f64::NAN,
        quadrature_volume_ratio: None,
        counts: String::new(),
        added: String::new(),
        parts: Vec::new(),
        problems: Vec::new(),
    };
    if let Some(error) = record.get("error").and_then(Value::as_str) {
        row.problems
            .push(format!("FreeCAD could not read it: {error}"));
        return row;
    }
    let shape = &record["shape"];
    let solids = record["solids"].as_array().cloned().unwrap_or_default();
    let all_valid = shape["valid"] == true && solids.iter().all(|s| s["valid"] == true);
    row.valid = if all_valid { "yes" } else { "NO" }.to_string();
    if !all_valid {
        row.problems
            .push("FreeCAD calls the shape invalid".to_string());
    }
    if solids.len() != case.solids {
        row.problems.push(format!(
            "{} solids in FreeCAD, {} expected",
            solids.len(),
            case.solids
        ));
    }
    let (k, k2, k3) = (case.to_mm, case.to_mm.powi(2), case.to_mm.powi(3));
    row.quadrature_volume_ratio = record["quadrature_volume"]
        .as_f64()
        .map(|volume| volume / (case.volume * k3));
    let area = shape["area"].as_f64().unwrap_or(f64::NAN);
    row.area_ratio = area / (case.area * k2);
    if !((row.area_ratio - 1.0).abs() <= case.relative) {
        row.problems
            .push(format!("area {area:e} mm², ours {:e}", case.area * k2));
    }
    if case.solid {
        let volume: f64 = solids.iter().filter_map(|s| s["volume"].as_f64()).sum();
        row.volume_ratio = volume / (case.volume * k3);
        if !((row.volume_ratio - 1.0).abs() <= case.relative) {
            row.problems.push(format!(
                "volume {volume:e} mm³, ours {:e}",
                case.volume * k3
            ));
        }
        if !solids.iter().all(|s| {
            s["closed"]
                .as_array()
                .is_some_and(|c| c.iter().all(|x| x == true))
        }) {
            row.problems.push("a shell is not closed".to_string());
        }
    }
    // The size of the body, for what "close" means.
    let size = case.area.sqrt() * k;
    if let (Some(ours), true) = (case.centroid, case.solid) {
        // The centre of several solids: weighted by volume.
        let mut centre = [0.0; 3];
        let mut total = 0.0;
        for solid in &solids {
            let (v, c) = (
                solid["volume"].as_f64().unwrap_or(0.0),
                numbers(&solid["centre"]),
            );
            if c.len() == 3 {
                for axis in 0..3 {
                    centre[axis] += c[axis] * v;
                }
                total += v;
            }
        }
        row.centre_off = (0..3)
            .map(|axis| (centre[axis] / total - ours[axis] * k).powi(2))
            .sum::<f64>()
            .sqrt()
            / size;
        if !(row.centre_off <= 10.0 * case.relative) {
            row.problems.push(format!(
                "centre of mass off by {:e} of the size",
                row.centre_off
            ));
        }
    }
    // Body by body, where FreeCAD's solids are our bodies.
    if case.solid && !case.parts.is_empty() && case.parts.len() == solids.len() {
        for (part, solid) in case.parts.iter().zip(&solids) {
            let volume = solid["volume"].as_f64().unwrap_or(f64::NAN);
            let area = solid["area"].as_f64().unwrap_or(f64::NAN);
            let centre = numbers(&solid["centre"]);
            let part_size = part.area.sqrt() * k;
            let centre_off = if centre.len() == 3 {
                (0..3)
                    .map(|axis| (centre[axis] - part.centroid[axis] * k).powi(2))
                    .sum::<f64>()
                    .sqrt()
                    / part_size
            } else {
                f64::NAN
            };
            let found = PartRow {
                name: part.name.clone(),
                volume_ratio: volume / (part.volume * k3),
                area_ratio: area / (part.area * k2),
                centre_off,
            };
            if !((found.volume_ratio - 1.0).abs() <= case.relative) {
                row.problems.push(format!(
                    "{}: volume {volume:e} mm³, ours {:e}",
                    part.name,
                    part.volume * k3
                ));
            }
            if !((found.area_ratio - 1.0).abs() <= case.relative) {
                row.problems.push(format!(
                    "{}: area {area:e} mm², ours {:e}",
                    part.name,
                    part.area * k2
                ));
            }
            if !(found.centre_off <= 10.0 * case.relative) {
                row.problems.push(format!(
                    "{}: centre of mass off by {centre_off:e} of its size",
                    part.name
                ));
            }
            row.parts.push(found);
        }
    }
    if let Some(ours) = case.bbox {
        let theirs = numbers(&shape["bbox"]);
        if theirs.len() == 6 {
            row.bbox_off = (0..6)
                .map(|i| (theirs[i] - ours[i] * k).abs())
                .fold(0.0, f64::max)
                / size;
            // How far one box sticks out of the other.
            let excess = |sign: f64| {
                (0..6)
                    .map(|i| {
                        let d = (theirs[i] - ours[i] * k) * sign;
                        if i < 3 { -d } else { d }
                    })
                    .fold(0.0, f64::max)
                    / size
            };
            let (theirs_out, ours_out) = (excess(1.0), excess(-1.0));
            let slack = 1e-6 + case.bbox_slack * k / size;
            let ok = match case.extent {
                Extent::Exact => row.bbox_off <= slack,
                Extent::Tight => ours_out <= slack,
                Extent::Loose => ours_out <= slack || theirs_out <= slack,
                Extent::Unchecked => true,
            };
            if !ok {
                row.problems.push(format!(
                    "bounding boxes ({:?}): FreeCAD's sticks out of ours by {theirs_out:e} of the size, ours out of FreeCAD's by {ours_out:e}",
                    case.extent
                ));
            }
        } else {
            row.problems
                .push("FreeCAD gave no bounding box".to_string());
        }
    }
    let count = |key: &str| shape[key].as_u64().unwrap_or(0) as usize;
    let (faces, edges, vertices, shells) = case.counts;
    let (seams, degenerated) = (count("seams"), count("degenerated"));
    row.counts = format!(
        "{faces}/{edges}/{vertices}/{shells} : {}/{}/{}/{}",
        count("faces"),
        count("edges"),
        count("vertices"),
        count("shells")
    );
    row.added = format!("{seams} seams, {degenerated} degenerated");
    if count("faces") != faces {
        row.problems
            .push(format!("{} faces in FreeCAD, {faces} ours", count("faces")));
    }
    // FreeCAD adds seams and poles to faces we write without, and splits
    // an edge that crosses a seam: never fewer edges or vertices. (A seam
    // it reports may be one of ours: a spline wall has its own.)
    if count("edges") < edges {
        row.problems
            .push(format!("{} edges in FreeCAD, {edges} ours", count("edges")));
    }
    if count("vertices") < vertices {
        row.problems.push(format!(
            "{} vertices in FreeCAD, {vertices} ours",
            count("vertices")
        ));
    }
    if count("shells") != shells {
        row.problems.push(format!(
            "{} shells in FreeCAD, {shells} ours",
            count("shells")
        ));
    }
    if let Some(names) = record["names"].as_array() {
        // FreeCAD shows a product with one solid as one object with
        // the product's name, and one with several as a container of
        // that name holding objects called name001, name002, …
        let theirs: Vec<&str> = names.iter().filter_map(Value::as_str).collect();
        let arrived = case.names.iter().all(|n| theirs.contains(&n.as_str()));
        let known = theirs
            .iter()
            .all(|t| case.names.iter().any(|n| t.starts_with(n.as_str())));
        if !(arrived && known) {
            row.problems.push(format!(
                "names in FreeCAD {theirs:?}, ours {:?}",
                case.names
            ));
        }
    } else {
        row.problems
            .push("FreeCAD's Import module is missing: names not checked".to_string());
    }
    row
}

/// One exported mesh file and what the exporter said of it.
#[derive(Clone, Debug)]
pub struct MeshFile {
    /// File name, without directory (`.stl`, `.3mf`).
    pub file: String,
    pub bytes: Vec<u8>,
}

/// Writes the mesh files into `dir` and returns what FreeCAD's mesh
/// module makes of each, by file name: `facets`, `points`, `area`,
/// `volume`, `solid` and `bbox` (in the file's own numbers: FreeCAD
/// stores a mesh in 32-bit floats and without a unit), or `error`. One
/// FreeCAD for all of them; a file it stops on ends the run and the
/// files after it are run again, as for STEP.
pub fn run_freecad_meshes(dir: &Path, files: &[MeshFile]) -> BTreeMap<String, Value> {
    let freecad = freecad();
    std::fs::create_dir_all(dir).unwrap();
    let script = dir.join("mesh_report.py");
    std::fs::write(&script, MESH_SCRIPT).unwrap();
    let mut records = BTreeMap::new();
    let mut rest: Vec<&MeshFile> = files.iter().collect();
    while !rest.is_empty() {
        let listing = dir.join("meshes.txt");
        let output = dir.join("meshes.jsonl");
        let mut list = String::new();
        for file in &rest {
            let path = dir.join(&file.file);
            std::fs::write(&path, &file.bytes).unwrap();
            writeln!(list, "{}", path.display()).unwrap();
        }
        std::fs::write(&listing, list).unwrap();
        let _ = std::fs::remove_file(&output);
        let mut command = if cfg!(target_os = "linux") {
            let mut shell = Command::new("sh");
            shell
                .arg("-c")
                .arg(format!("ulimit -v {ADDRESS_SPACE_KB} && exec \"$@\""))
                .arg("sh")
                .arg(&freecad);
            shell
        } else {
            Command::new(&freecad)
        };
        if freecad.ends_with(".AppImage") {
            command.arg("freecadcmd");
        }
        let run = command
            .arg(&script)
            .env("CRAFT3D_FC_LIST", &listing)
            .env("CRAFT3D_FC_OUT", &output)
            .stdin(Stdio::null())
            .output()
            .unwrap_or_else(|e| panic!("cannot run {freecad}: {e}"));
        let text = std::fs::read_to_string(&output).unwrap_or_else(|_| {
            panic!(
                "FreeCAD wrote no mesh report ({}):\n{}\n{}",
                run.status,
                String::from_utf8_lossy(&run.stdout),
                String::from_utf8_lossy(&run.stderr)
            )
        });
        let mut done = 0;
        for line in text.lines() {
            let value: Value = serde_json::from_str(line).unwrap();
            let file = value["file"].as_str().unwrap().to_string();
            records.insert(file, value);
            done += 1;
        }
        if done < rest.len() {
            let culprit = rest[done];
            records.insert(
                culprit.file.clone(),
                serde_json::json!({"file": culprit.file, "error": format!(
                    "FreeCAD stopped on this file ({})", run.status)}),
            );
            done += 1;
        }
        rest.drain(..done);
    }
    records
}

/// The rows as a Markdown table, and the problems under it.
pub fn table(title: &str, version: &str, rows: &[Row]) -> String {
    let mut out = format!(
        "## {title}\n\nFreeCAD {version}. Ratios are FreeCAD's value over ours; counts are faces/edges/vertices/shells, ours : FreeCAD's.\n\n"
    );
    out.push_str("| file | valid | volume ratio - 1 | area ratio - 1 | centre off | bbox off | counts | FreeCAD added |\n|---|---|---|---|---|---|---|---|\n");
    for row in rows {
        writeln!(
            out,
            "| {} | {} | {:+.1e} | {:+.1e} | {:.1e} | {:.1e} | {} | {} |",
            row.file,
            row.valid,
            row.volume_ratio - 1.0,
            row.area_ratio - 1.0,
            row.centre_off,
            row.bbox_off,
            row.counts,
            row.added
        )
        .unwrap();
    }
    let bad: Vec<&Row> = rows.iter().filter(|r| !r.problems.is_empty()).collect();
    writeln!(
        out,
        "\n{} of {} files with problems.",
        bad.len(),
        rows.len()
    )
    .unwrap();
    for row in bad {
        writeln!(out, "- {}: {}", row.file, row.problems.join("; ")).unwrap();
    }
    out
}

/// Runs the cases, writes the table next to the files and returns the
/// rows.
pub fn check(dir_name: &str, title: &str, cases: &[Case]) -> Vec<Row> {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(dir_name);
    let _ = std::fs::remove_dir_all(&dir);
    let (version, records) = run_freecad(&dir, cases);
    let rows: Vec<Row> = cases
        .iter()
        .map(|case| match records.get(&case.file) {
            Some(record) => compare(case, record),
            None => {
                let mut row = compare(case, &serde_json::json!({"error": "no record"}));
                row.file = case.file.clone();
                row
            }
        })
        .collect();
    let text = table(title, &version, &rows);
    std::fs::write(dir.join("report.md"), &text).unwrap();
    println!("{text}\nreport: {}", dir.join("report.md").display());
    rows
}
