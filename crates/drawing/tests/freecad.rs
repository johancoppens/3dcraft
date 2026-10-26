//! An independent program reads the DXF and SVG files (oracle 8 of
//! docs/testing.md): FreeCAD imports them and measures what it got.
//!
//! Opt-in, because it needs FreeCAD:
//!
//! ```sh
//! CRAFT3D_FREECAD=/path/to/FreeCAD.AppImage cargo test -p craft3d-drawing --test freecad -- --ignored --nocapture
//! ```
//!
//! When opted in, a missing or failing FreeCAD is a test failure, never a
//! skip. FreeCAD runs as a separate program; nothing of it is linked.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_drawing::{
    Color, Drawing, Layer, Path, Point, Segment, Spline, SvgOptions, Unit, write_dxf, write_svg,
};
use std::collections::BTreeMap;
use std::f64::consts::{PI, TAU};
use std::path::PathBuf;
use std::process::Command;

/// What FreeCAD reports for a file.
#[derive(Debug)]
struct Report {
    edges: usize,
    kinds: BTreeMap<String, usize>,
    length: f64,
    bbox: [f64; 4],
    wires: usize,
    closed: usize,
    area: f64,
}

/// Writes the files into a fresh directory, lets FreeCAD read them all in
/// one run, and returns its report per file name.
fn freecad_reads(run_name: &str, files: &[(String, String)]) -> BTreeMap<String, Report> {
    let freecad = std::env::var("CRAFT3D_FREECAD").expect(
        "set CRAFT3D_FREECAD to a FreeCAD AppImage or FreeCADCmd executable to run these tests",
    );
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(run_name);
    let _ = std::fs::remove_dir_all(&dir);
    let input = dir.join("files");
    std::fs::create_dir_all(&input).unwrap();
    for (name, text) in files {
        std::fs::write(input.join(name), text).unwrap();
    }
    let output = dir.join("report.txt");
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/drawing_report.py");
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
            "FreeCAD wrote no report ({}):\n{}\n{}",
            run.status,
            String::from_utf8_lossy(&run.stdout),
            String::from_utf8_lossy(&run.stderr)
        )
    });
    let mut reports = BTreeMap::new();
    for line in text.lines() {
        let fields: BTreeMap<&str, &str> =
            line.split('\t').filter_map(|f| f.split_once('=')).collect();
        let file = fields["file"].to_string();
        if let Some(error) = fields.get("error") {
            panic!("FreeCAD could not read {file}: {error}");
        }
        let number = |key: &str| -> f64 { fields[key].parse().unwrap() };
        let bbox: Vec<f64> = fields["bbox"]
            .split(' ')
            .map(|v| v.parse().unwrap())
            .collect();
        let kinds = fields["kinds"]
            .split(',')
            .filter_map(|k| k.split_once(':'))
            .map(|(kind, count)| (kind.to_string(), count.parse().unwrap()))
            .collect();
        reports.insert(
            file,
            Report {
                edges: number("edges") as usize,
                kinds,
                length: number("length"),
                bbox: bbox.try_into().unwrap(),
                wires: number("wires") as usize,
                closed: number("closed") as usize,
                area: number("area"),
            },
        );
    }
    assert_eq!(reports.len(), files.len(), "a report per file");
    reports
}

fn near(a: f64, b: f64, relative: f64, scale: f64) -> bool {
    (a - b).abs() <= relative * scale.abs()
}

fn line(to: Point) -> Segment {
    Segment::Line { to }
}

fn arc(to: Point, center: Point, radius: f64, ccw: bool) -> Segment {
    Segment::Arc {
        to,
        center,
        radius,
        ccw,
    }
}

fn chain(start: Point, segments: Vec<Segment>, closed: bool) -> Path {
    Path::Chain {
        start,
        segments,
        closed,
    }
}

fn drawing(unit: Unit, paths: Vec<Path>) -> Drawing {
    Drawing {
        unit,
        layers: vec![Layer {
            name: "outline".into(),
            color: Color::Black,
            dash: None,
            dot: false,
            paths,
        }],
    }
}

/// A 60 × 40 plate with rounded corners of 5, a round hole of radius 4, a
/// D hole (radius 5, flat at 3 from its centre) and a slot of radius 3
/// over 20, all times `k`.
fn plate(unit: Unit, k: f64) -> Drawing {
    let p = |x: f64, y: f64| [x * k, y * k];
    let (w, h, r) = (60.0, 40.0, 5.0);
    let outline = chain(
        p(r, 0.0),
        vec![
            line(p(w - r, 0.0)),
            arc(p(w, r), p(w - r, r), r * k, true),
            line(p(w, h - r)),
            arc(p(w - r, h), p(w - r, h - r), r * k, true),
            line(p(r, h)),
            arc(p(0.0, h - r), p(r, h - r), r * k, true),
            line(p(0.0, r)),
            arc(p(r, 0.0), p(r, r), r * k, true),
        ],
        true,
    );
    let hole = Path::Circle {
        center: p(15.0, 28.0),
        radius: 4.0 * k,
    };
    let d = chain(
        p(48.0, 24.0),
        vec![
            line(p(48.0, 32.0)),
            arc(p(48.0, 24.0), p(45.0, 28.0), 5.0 * k, true),
        ],
        true,
    );
    let slot = chain(
        p(15.0, 7.0),
        vec![
            arc(p(15.0, 13.0), p(15.0, 10.0), 3.0 * k, false),
            line(p(35.0, 13.0)),
            arc(p(35.0, 7.0), p(35.0, 10.0), 3.0 * k, false),
            line(p(15.0, 7.0)),
        ],
        true,
    );
    drawing(unit, vec![outline, hole, d, slot])
}

/// Closed forms for the plate at `k` = 1: its length and its area.
fn plate_numbers() -> (f64, f64) {
    // Half the angle the D's flat cuts off.
    let half = (3.0_f64 / 5.0).acos();
    let length = (2.0 * (50.0 + 30.0) + TAU * 5.0)
        + TAU * 4.0
        + (8.0 + 5.0 * (TAU - 2.0 * half))
        + (40.0 + TAU * 3.0);
    let outline = 60.0 * 40.0 - (4.0 - PI) * 25.0;
    let hole = PI * 16.0;
    // The circle less the segment beyond the flat.
    let d = PI * 25.0 - 25.0 * (half - half.sin() * half.cos());
    let slot = 20.0 * 6.0 + PI * 9.0;
    (length, outline - hole - d - slot)
}

fn both(name: &str, drawing: &Drawing) -> Vec<(String, String)> {
    vec![
        (format!("{name}.dxf"), write_dxf(drawing).unwrap()),
        (
            format!("{name}.svg"),
            write_svg(drawing, &SvgOptions::default()).unwrap().0,
        ),
    ]
}

#[test]
#[ignore = "needs FreeCAD: set CRAFT3D_FREECAD and run with --ignored"]
fn freecad_reads_the_plate_at_its_true_size_in_every_unit_and_scale() {
    // FreeCAD works in millimetres: a file in another unit comes in
    // scaled, which is the proof that the unit is in the file.
    let cases = [
        ("mm", Unit::Millimeter, 1.0, 1.0),
        ("in", Unit::Inch, 1.0, 25.4),
        ("cm", Unit::Centimeter, 1.0, 10.0),
        ("small", Unit::Millimeter, 1e-3, 1.0),
        ("large", Unit::Millimeter, 1e3, 1.0),
    ];
    let mut files = Vec::new();
    for (name, unit, k, _) in cases {
        files.extend(both(name, &plate(unit, k)));
    }
    let reports = freecad_reads("freecad-plate", &files);
    let (length, area) = plate_numbers();
    println!("closed form at scale 1: length {length}, area {area}");
    for (name, unit, k, to_mm) in cases {
        let ours = plate(unit, k);
        assert!(near(ours.length(), length * k, 1e-13, length * k));
        for extension in ["dxf", "svg"] {
            let file = format!("{name}.{extension}");
            let report = &reports[&file];
            let s = k * to_mm;
            println!(
                "{file}: length {} (expected {}), area {} (expected {}), bbox {:?}, {} edges {:?}, {} wires, {} closed",
                report.length,
                length * s,
                report.area,
                area * s * s,
                report.bbox,
                report.edges,
                report.kinds,
                report.wires,
                report.closed
            );
            assert!(near(report.length, length * s, 1e-9, length * s), "{file}");
            assert!(
                near(report.area, area * s * s, 1e-9, area * s * s),
                "{file}"
            );
            for (got, expected) in report.bbox.iter().zip([0.0, 0.0, 60.0 * s, 40.0 * s]) {
                assert!(
                    near(*got, expected, 1e-9, 60.0 * s),
                    "{file}: {:?}",
                    report.bbox
                );
            }
            assert_eq!((report.wires, report.closed), (4, 4), "{file}");
            if to_mm != 1.0 {
                // FreeCAD scales to millimetres, and what it scales it
                // turns into B-splines (the SVG reader) or keeps (DXF).
                assert!(report.edges >= 15, "{file}");
                continue;
            }
            assert_eq!(report.kinds["Line"], 7, "{file}");
            if extension == "dxf" {
                // Arcs and the circle arrive as circles, one edge each.
                assert_eq!(report.edges, 15, "{file}");
                assert_eq!(report.kinds["Circle"], 8, "{file}");
            } else {
                // FreeCAD's SVG reader makes every arc command an edge on
                // an ellipse with two equal radii: 4 quarter arcs, 2 half
                // turns in 2 parts each, the D's arc in 3, the circle in 4.
                assert_eq!(report.kinds.len(), 2, "{file}: {:?}", report.kinds);
                assert_eq!(report.edges, 7 + 4 + 4 + 3 + 4, "{file}");
            }
        }
    }
}

#[test]
#[ignore = "needs FreeCAD: set CRAFT3D_FREECAD and run with --ignored"]
fn freecad_reads_ellipses_and_splines_as_what_they_are() {
    // An ellipse turned 30° with semi-axes 4 and 1.5, whole.
    let turn = PI / 6.0;
    let major = [4.0 * turn.cos(), 4.0 * turn.sin()];
    let whole = drawing(
        Unit::Millimeter,
        vec![Path::Ellipse {
            center: [2.0, -1.0],
            major,
            ratio: 0.375,
        }],
    );
    // Half of it, closed by its chord: both directions.
    let on = |t: f64| {
        let minor = [-major[1] * 0.375, major[0] * 0.375];
        [
            2.0 + major[0] * t.cos() + minor[0] * t.sin(),
            -1.0 + major[1] * t.cos() + minor[1] * t.sin(),
        ]
    };
    let half = |ccw: bool| {
        let (from, to) = if ccw { (0.0, PI) } else { (PI, 0.0) };
        drawing(
            Unit::Millimeter,
            vec![chain(
                on(from),
                vec![
                    Segment::EllipticArc {
                        to: on(to),
                        center: [2.0, -1.0],
                        major,
                        ratio: 0.375,
                        ccw,
                    },
                    line(on(from)),
                ],
                true,
            )],
        )
    };
    // A closed shape of a cubic spline with a knot inside and a line.
    let cubic = Spline::new(
        3,
        vec![0.0, 0.0, 0.0, 0.0, 1.0, 2.0, 2.0, 2.0, 2.0],
        vec![[0.0, 0.0], [1.0, 3.0], [4.0, 4.0], [7.0, 3.0], [8.0, 0.0]],
        None,
    )
    .unwrap();
    let splined = |spline: Spline| {
        let (start, end) = (spline.start(), spline.end());
        drawing(
            Unit::Millimeter,
            vec![chain(
                start,
                vec![Segment::Spline(spline), line(start)],
                true,
            )],
        )
        .tap(|_| assert_ne!(start, end))
    };
    // A half disc of radius 2 whose arc is a rational quadratic spline.
    let s = std::f64::consts::FRAC_1_SQRT_2;
    let half_circle = Spline::new(
        2,
        vec![0.0, 0.0, 0.0, 1.0, 1.0, 2.0, 2.0, 2.0],
        vec![[2.0, 0.0], [2.0, 2.0], [0.0, 2.0], [-2.0, 2.0], [-2.0, 0.0]],
        Some(vec![1.0, s, 1.0, s, 1.0]),
    )
    .unwrap();
    // Degree 5: SVG gets cubics within the tolerance.
    let quintic = Spline::new(
        5,
        vec![0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0],
        vec![
            [0.0, 0.0],
            [1.0, 2.0],
            [3.0, 5.0],
            [4.0, -3.0],
            [5.0, 1.0],
            [6.0, 0.0],
        ],
        None,
    )
    .unwrap();
    let drawings = [
        ("ellipse", whole),
        ("half-ellipse-ccw", half(true)),
        ("half-ellipse-cw", half(false)),
        ("cubic", splined(cubic)),
        ("half-circle", splined(half_circle)),
        ("quintic", splined(quintic)),
    ];
    let mut files = Vec::new();
    for (name, drawing) in &drawings {
        files.extend(both(name, drawing));
    }
    let reports = freecad_reads("freecad-curves", &files);

    // Closed forms. The ellipse: area π a b; its length by the series in
    // h = ((a - b) / (a + b))².
    let (a, b) = (4.0_f64, 1.5_f64);
    let h = ((a - b) / (a + b)).powi(2);
    let (mut series, mut term) = (0.0, 1.0_f64);
    for n in 0..60 {
        series += term * term * h.powi(n);
        term *= (0.5 - f64::from(n)) / (f64::from(n) + 1.0);
    }
    let perimeter = PI * (a + b) * series;
    // The half ellipse's bounding box is not needed: length and area do.
    let expected: BTreeMap<&str, (Option<f64>, Option<f64>)> = [
        ("ellipse", (Some(perimeter), Some(PI * a * b))),
        (
            "half-ellipse-ccw",
            (Some(perimeter / 2.0 + 2.0 * a), Some(PI * a * b / 2.0)),
        ),
        (
            "half-ellipse-cw",
            (Some(perimeter / 2.0 + 2.0 * a), Some(PI * a * b / 2.0)),
        ),
        // Green's theorem over the two cubic pieces, integrated exactly
        // (an 8-point Gauss rule, in a script): 20.9.
        ("cubic", (None, Some(20.9))),
        ("half-circle", (Some(TAU + 4.0), Some(TAU))),
        ("quintic", (None, None)),
    ]
    .into();
    for (name, drawing) in &drawings {
        let ours_length = drawing.length();
        let ours_area = green_area(drawing);
        for extension in ["dxf", "svg"] {
            let file = format!("{name}.{extension}");
            let report = &reports[&file];
            println!(
                "{file}: length {} (ours {ours_length}), area {} (ours {ours_area}), {} edges {:?}, {} closed of {} wires",
                report.length, report.area, report.edges, report.kinds, report.closed, report.wires
            );
            if extension == "dxf" {
                let kind = match *name {
                    "ellipse" | "half-ellipse-ccw" | "half-ellipse-cw" => "Ellipse",
                    _ => "BSplineCurve",
                };
                assert_eq!(
                    report.kinds.get(kind),
                    Some(&1),
                    "{file}: {:?}",
                    report.kinds
                );
            }
            // FreeCAD's DXF reader (1.1) draws every ELLIPSE whole: it
            // does not look at the start and end parameters (groups 41
            // and 42). So an elliptic arc cannot be measured through it;
            // what it got is the whole ellipse and the chord.
            if extension == "dxf" && name.starts_with("half-ellipse") {
                assert!(
                    near(report.length, perimeter + 2.0 * a, 1e-9, perimeter),
                    "{file}: FreeCAD reads elliptic arcs now; check them here"
                );
                continue;
            }
            // SVG holds a quintic and a rational spline as cubics within
            // the tolerance of 1e-6; everything else is exact.
            let exact = extension == "dxf" || !matches!(*name, "half-circle" | "quintic");
            let relative = if exact { 1e-9 } else { 1e-6 };
            // FreeCAD's area of a face with a B-spline in its boundary is
            // rough (measured: 20.89947 for the cubic, whose area is 20.9
            // exactly, and 1.4e-4 off for the half disc), while its
            // length of the same edges agrees to 15 digits and its area
            // of the same shape from Bézier curves (the SVG) is exact.
            let splined = matches!(*name, "cubic" | "half-circle" | "quintic");
            let area_relative = if splined { 1e-3 } else { relative };
            assert!(
                near(report.length, ours_length, relative, ours_length),
                "{file}"
            );
            assert!(
                near(report.area, ours_area, area_relative, ours_area),
                "{file}"
            );
            assert_eq!((report.wires, report.closed), (1, 1), "{file}");
            let (length, area) = expected[name];
            if let Some(length) = length {
                assert!(near(report.length, length, relative, length), "{file}");
            }
            if let Some(area) = area {
                assert!(near(report.area, area, area_relative, area), "{file}");
                assert!(near(ours_area, area, 1e-7, area), "{name}: {ours_area}");
            }
        }
    }
}

trait Tap: Sized {
    fn tap(self, f: impl FnOnce(&Self)) -> Self {
        f(&self);
        self
    }
}
impl<T> Tap for T {}

/// The area inside a drawing of one closed chain of lines and polynomial
/// splines, by Green's theorem on a fine polygon through the curve
/// (10,000 points per spline: good to 1e-7 of the area; only used for the
/// shapes without a closed form, as a check on FreeCAD's number at that
/// precision).
fn green_area(drawing: &Drawing) -> f64 {
    let Path::Chain {
        start, segments, ..
    } = &drawing.layers[0].paths[0]
    else {
        // The whole ellipse.
        return PI * 4.0 * 1.5;
    };
    let mut points = vec![*start];
    for segment in segments {
        match segment {
            Segment::Line { to } => points.push(*to),
            Segment::Spline(spline) => {
                let (a, b) = spline.domain();
                for i in 1..=10_000 {
                    points.push(spline.eval(a + (b - a) * f64::from(i) / 10_000.0).unwrap());
                }
            }
            Segment::EllipticArc { .. } => return PI * 4.0 * 1.5 / 2.0,
            Segment::Arc { .. } => panic!("not in these drawings"),
        }
    }
    let mut twice = 0.0;
    for pair in points.windows(2) {
        twice += pair[0][0] * pair[1][1] - pair[1][0] * pair[0][1];
    }
    (twice / 2.0).abs()
}
