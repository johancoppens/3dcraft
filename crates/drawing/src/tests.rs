//! The writers against hand-written files, and against two small readers
//! written here: enough of DXF's group codes and of SVG's path grammar to
//! get the entities back and compare them with what went in.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use std::f64::consts::{FRAC_PI_2, PI, TAU};

// ---------------------------------------------------------------- shapes

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

fn closed(start: Point, segments: Vec<Segment>) -> Path {
    Path::Chain {
        start,
        segments,
        closed: true,
    }
}

fn open(start: Point, segments: Vec<Segment>) -> Path {
    Path::Chain {
        start,
        segments,
        closed: false,
    }
}

fn drawing(paths: Vec<Path>) -> Drawing {
    Drawing {
        unit: Unit::Millimeter,
        layers: vec![Layer {
            name: "outline".into(),
            color: Color::Black,
            dash: None,
            dot: false,
            paths,
        }],
    }
}

/// A `w` by `h` rectangle with corners of radius `r`, from the origin,
/// counter-clockwise.
fn rounded_rectangle(w: f64, h: f64, r: f64) -> Path {
    closed(
        [r, 0.0],
        vec![
            line([w - r, 0.0]),
            arc([w, r], [w - r, r], r, true),
            line([w, h - r]),
            arc([w - r, h], [w - r, h - r], r, true),
            line([r, h]),
            arc([0.0, h - r], [r, h - r], r, true),
            line([0.0, r]),
            arc([r, 0.0], [r, r], r, true),
        ],
    )
}

/// A slot of radius `r` between the centres `(x0, y)` and `(x1, y)`, run
/// clockwise as a hole is.
fn slot(x0: f64, x1: f64, y: f64, r: f64) -> Path {
    closed(
        [x0, y - r],
        vec![
            arc([x0, y + r], [x0, y], r, false),
            line([x1, y + r]),
            arc([x1, y - r], [x1, y], r, false),
            line([x0, y - r]),
        ],
    )
}

/// A circle of radius `r` about `c` with a flat at `flat` to the right of
/// the centre: one line and one arc of more than a half turn.
fn d_hole(c: Point, r: f64, flat: f64) -> Path {
    let y = (r * r - flat * flat).sqrt();
    closed(
        [c[0] + flat, c[1] - y],
        vec![
            line([c[0] + flat, c[1] + y]),
            arc([c[0] + flat, c[1] - y], c, r, true),
        ],
    )
}

/// The 60 × 40 plate of the docs: rounded corners of 5, a round hole, a D
/// hole and a slot.
fn plate() -> Drawing {
    drawing(vec![
        rounded_rectangle(60.0, 40.0, 5.0),
        Path::Circle {
            center: [15.0, 28.0],
            radius: 4.0,
        },
        d_hole([45.0, 28.0], 5.0, 3.0),
        slot(15.0, 35.0, 10.0, 3.0),
    ])
}

fn plate_length() -> f64 {
    let outline = 2.0 * (50.0 + 30.0) + TAU * 5.0;
    let hole = TAU * 4.0;
    // The D: the chord, and the circle less the arc the chord cuts off.
    let half = libm::acos(3.0_f64 / 5.0);
    let d = 2.0 * 4.0 + 5.0 * (TAU - 2.0 * half);
    let slot = 2.0 * 20.0 + TAU * 3.0;
    outline + hole + d + slot
}

fn scaled(drawing: &Drawing, k: f64) -> Drawing {
    let p = |p: Point| [p[0] * k, p[1] * k];
    let mut out = drawing.clone();
    for layer in &mut out.layers {
        for path in &mut layer.paths {
            match path {
                Path::Circle { center, radius } => {
                    *center = p(*center);
                    *radius *= k;
                }
                Path::Ellipse { center, major, .. } => {
                    *center = p(*center);
                    *major = p(*major);
                }
                Path::Chain {
                    start, segments, ..
                } => {
                    *start = p(*start);
                    for segment in segments {
                        match segment {
                            Segment::Line { to } => *to = p(*to),
                            Segment::Arc {
                                to, center, radius, ..
                            } => {
                                *to = p(*to);
                                *center = p(*center);
                                *radius *= k;
                            }
                            Segment::EllipticArc {
                                to, center, major, ..
                            } => {
                                *to = p(*to);
                                *center = p(*center);
                                *major = p(*major);
                            }
                            Segment::Spline(spline) => {
                                *spline = Spline::new(
                                    spline.degree(),
                                    spline.knots().to_vec(),
                                    spline.control_points().iter().map(|q| p(*q)).collect(),
                                    spline.weights().map(<[f64]>::to_vec),
                                )
                                .unwrap();
                            }
                        }
                    }
                }
            }
        }
    }
    out
}

fn near(a: f64, b: f64, scale: f64) -> bool {
    (a - b).abs() <= 1e-12 * scale.abs().max(f64::MIN_POSITIVE)
}

fn near_point(a: Point, b: Point, scale: f64) -> bool {
    near(a[0], b[0], scale) && near(a[1], b[1], scale)
}

// ------------------------------------------------------------ DXF reader

#[derive(Clone, Debug, PartialEq)]
enum Entity {
    Line(Point, Point),
    Circle(Point, f64),
    /// Centre, radius, start and end angle in radians (counter-clockwise).
    Arc(Point, f64, f64, f64),
    /// Centre, major axis, ratio, start and end parameter.
    Ellipse(Point, Point, f64, f64, f64),
    Spline {
        flags: u32,
        degree: usize,
        knots: Vec<f64>,
        weights: Vec<f64>,
        control: Vec<Point>,
    },
}

struct Dxf {
    groups: Vec<(u32, String)>,
}

impl Dxf {
    fn read(text: &str) -> Self {
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len() % 2, 0, "groups are pairs of lines");
        let groups = lines
            .chunks(2)
            .map(|pair| {
                let code = pair[0];
                assert_eq!(code.len(), 3, "a group code takes three columns: `{code}`");
                (code.trim().parse().unwrap(), pair[1].to_string())
            })
            .collect();
        Self { groups }
    }

    /// The groups of a section, without its `SECTION` and name.
    fn section(&self, name: &str) -> &[(u32, String)] {
        let start = self
            .groups
            .windows(2)
            .position(|w| w[0] == (0, "SECTION".into()) && w[1] == (2, name.into()))
            .unwrap_or_else(|| panic!("no section {name}"));
        let rest = &self.groups[start + 2..];
        let end = rest
            .iter()
            .position(|g| *g == (0, "ENDSEC".into()))
            .unwrap();
        &rest[..end]
    }

    /// The value after a header variable.
    fn header(&self, name: &str) -> &str {
        let header = self.section("HEADER");
        let at = header
            .iter()
            .position(|g| *g == (9, name.into()))
            .unwrap_or_else(|| panic!("no header variable {name}"));
        &header[at + 1].1
    }

    /// Every entity with its layer.
    fn entities(&self) -> Vec<(String, Entity)> {
        let section = self.section("ENTITIES");
        let mut starts: Vec<usize> = section
            .iter()
            .enumerate()
            .filter(|(_, g)| g.0 == 0)
            .map(|(i, _)| i)
            .collect();
        starts.push(section.len());
        starts
            .windows(2)
            .map(|w| {
                let groups = &section[w[0]..w[1]];
                let all = |code: u32| -> Vec<f64> {
                    groups
                        .iter()
                        .filter(|g| g.0 == code)
                        .map(|g| {
                            assert!(
                                g.1.contains(['.', 'E']),
                                "a real without a decimal point: {}",
                                g.1
                            );
                            g.1.parse().unwrap()
                        })
                        .collect()
                };
                let one = |code: u32| -> f64 {
                    let values = all(code);
                    assert_eq!(values.len(), 1, "group {code} of {}", groups[0].1);
                    values[0]
                };
                let integer = |code: u32| -> u32 {
                    groups
                        .iter()
                        .find(|g| g.0 == code)
                        .unwrap()
                        .1
                        .parse()
                        .unwrap()
                };
                let text = |code: u32| -> String {
                    groups.iter().find(|g| g.0 == code).unwrap().1.clone()
                };
                // Every entity lies in the plane z = 0, has a handle and
                // belongs to the model space.
                for z in all(30).into_iter().chain(all(31)) {
                    assert_eq!(z, 0.0);
                }
                assert!(u64::from_str_radix(&text(5), 16).unwrap() >= 0x100);
                assert_eq!(text(330), "1F");
                assert_eq!(text(100), "AcDbEntity");
                let entity = match groups[0].1.as_str() {
                    "LINE" => Entity::Line([one(10), one(20)], [one(11), one(21)]),
                    "CIRCLE" => Entity::Circle([one(10), one(20)], one(40)),
                    "ARC" => Entity::Arc(
                        [one(10), one(20)],
                        one(40),
                        one(50).to_radians(),
                        one(51).to_radians(),
                    ),
                    "ELLIPSE" => {
                        assert_eq!((one(210), one(220), one(230)), (0.0, 0.0, 1.0));
                        Entity::Ellipse(
                            [one(10), one(20)],
                            [one(11), one(21)],
                            one(40),
                            one(41),
                            one(42),
                        )
                    }
                    "SPLINE" => {
                        let (xs, ys) = (all(10), all(20));
                        let spline = Entity::Spline {
                            flags: integer(70),
                            degree: integer(71) as usize,
                            knots: all(40),
                            weights: all(41),
                            control: xs.iter().zip(&ys).map(|(x, y)| [*x, *y]).collect(),
                        };
                        let Entity::Spline { knots, control, .. } = &spline else {
                            panic!("a spline was made")
                        };
                        assert_eq!(integer(72) as usize, knots.len());
                        assert_eq!(integer(73) as usize, control.len());
                        assert_eq!(integer(74), 0);
                        spline
                    }
                    other => panic!("unexpected entity {other}"),
                };
                (text(8), entity)
            })
            .collect()
    }
}

/// The length of what the DXF reader got back, with nothing from the
/// writer's side: arcs by their angles.
fn entity_length(entity: &Entity) -> f64 {
    match entity {
        Entity::Line(a, b) => libm::hypot(b[0] - a[0], b[1] - a[1]),
        Entity::Circle(_, r) => TAU * r,
        Entity::Arc(_, r, a0, a1) => {
            let sweep = (a1 - a0).rem_euclid(TAU);
            r * if sweep == 0.0 { TAU } else { sweep }
        }
        Entity::Ellipse(..) | Entity::Spline { .. } => panic!("not measured here"),
    }
}

// ------------------------------------------------------------ SVG reader

#[derive(Clone, Debug, PartialEq)]
enum Step {
    Move(Point),
    Line(Point),
    /// rx, ry, rotation in degrees, large, sweep, end.
    Arc(f64, f64, f64, bool, bool, Point),
    Quadratic(Point, Point),
    Cubic(Point, Point, Point),
    Close,
}

fn attribute<'a>(element: &'a str, name: &str) -> &'a str {
    let key = format!(" {name}=\"");
    let start = element
        .find(&key)
        .unwrap_or_else(|| panic!("no attribute {name}"))
        + key.len();
    let end = element[start..].find('"').unwrap();
    &element[start..start + end]
}

/// The `<path>` elements of a document.
fn svg_paths(svg: &str) -> Vec<&str> {
    svg.lines().filter(|l| l.starts_with("<path ")).collect()
}

/// The path grammar as far as the writer uses it: absolute `M L A Q C Z`,
/// numbers separated by commas and spaces. The y of every point is
/// negated back.
fn steps(d: &str) -> Vec<Step> {
    let mut tokens: Vec<String> = Vec::new();
    for word in d.split([' ', ',']).filter(|w| !w.is_empty()) {
        let first = word.chars().next().unwrap();
        if first.is_ascii_alphabetic() {
            tokens.push(first.to_string());
            if word.len() > 1 {
                tokens.push(word[1..].to_string());
            }
        } else {
            tokens.push(word.to_string());
        }
    }
    let mut out = Vec::new();
    let mut at = 0;
    let number = |at: &mut usize| -> f64 {
        let text = &tokens[*at];
        *at += 1;
        assert_ne!(text, "-0", "negative zero in a path");
        text.parse().unwrap_or_else(|_| panic!("number `{text}`"))
    };
    while at < tokens.len() {
        let command = tokens[at].clone();
        at += 1;
        let point = |at: &mut usize| -> Point {
            let x = number(at);
            let y = number(at);
            [x, if y == 0.0 { 0.0 } else { -y }]
        };
        out.push(match command.as_str() {
            "M" => Step::Move(point(&mut at)),
            "L" => Step::Line(point(&mut at)),
            "Q" => Step::Quadratic(point(&mut at), point(&mut at)),
            "C" => Step::Cubic(point(&mut at), point(&mut at), point(&mut at)),
            "A" => {
                let (rx, ry, rotation) = (number(&mut at), number(&mut at), number(&mut at));
                let (large, sweep) = (number(&mut at), number(&mut at));
                assert!([0.0, 1.0].contains(&large) && [0.0, 1.0].contains(&sweep));
                Step::Arc(rx, ry, rotation, large == 1.0, sweep == 1.0, point(&mut at))
            }
            "Z" => Step::Close,
            other => panic!("unexpected path command {other}"),
        });
    }
    out
}

/// The centre and the signed sweep (positive counter-clockwise, y up) of
/// an SVG arc from `from`, by the conversion of the SVG recommendation's
/// implementation notes (F.6.5), done in SVG's own y-down coordinates.
fn arc_center(from: Point, step: &Step) -> (Point, f64) {
    let Step::Arc(rx, ry, rotation, large, sweep, to) = step else {
        panic!("not an arc")
    };
    let (x1, y1, x2, y2) = (from[0], -from[1], to[0], -to[1]);
    let (sin, cos) = libm::sincos(rotation.to_radians());
    let (dx, dy) = ((x1 - x2) / 2.0, (y1 - y2) / 2.0);
    let (xp, yp) = (cos * dx + sin * dy, -sin * dx + cos * dy);
    let lambda = libm::pow(xp / rx, 2.0) + libm::pow(yp / ry, 2.0);
    assert!(
        lambda <= 1.0 + 1e-9,
        "the radii do not reach the end points: {lambda}"
    );
    let numerator = libm::pow(rx * ry, 2.0) - libm::pow(rx * yp, 2.0) - libm::pow(ry * xp, 2.0);
    let denominator = libm::pow(rx * yp, 2.0) + libm::pow(ry * xp, 2.0);
    let mut k = (numerator.max(0.0) / denominator).sqrt();
    if large == sweep {
        k = -k;
    }
    let (cxp, cyp) = (k * rx * yp / ry, -k * ry * xp / rx);
    let cx = cos * cxp - sin * cyp + (x1 + x2) / 2.0;
    let cy = sin * cxp + cos * cyp + (y1 + y2) / 2.0;
    let angle =
        |ux: f64, uy: f64, vx: f64, vy: f64| libm::atan2(ux * vy - uy * vx, ux * vx + uy * vy);
    let (ux, uy) = ((xp - cxp) / rx, (yp - cyp) / ry);
    let (vx, vy) = ((-xp - cxp) / rx, (-yp - cyp) / ry);
    let mut delta = angle(ux, uy, vx, vy);
    if *sweep && delta < 0.0 {
        delta += TAU;
    } else if !*sweep && delta > 0.0 {
        delta -= TAU;
    }
    // Clockwise on a y-down screen is counter-clockwise with y up.
    ([cx, -cy], -delta)
}

/// What a subpath is made of, with arcs as centre, radius and sweep.
#[derive(Clone, Debug)]
enum Got {
    Line(Point, Point),
    /// From, to, centre, radii, rotation in degrees (y up), sweep.
    Arc(Point, Point, Point, [f64; 2], f64, f64),
}

/// The subpaths of a path: their pieces and whether they are closed.
fn subpaths(d: &str) -> Vec<(Vec<Got>, bool)> {
    let mut out: Vec<(Vec<Got>, bool)> = Vec::new();
    let mut at = [0.0, 0.0];
    for step in steps(d) {
        match &step {
            Step::Move(p) => {
                out.push((Vec::new(), false));
                at = *p;
            }
            Step::Line(p) => {
                out.last_mut().unwrap().0.push(Got::Line(at, *p));
                at = *p;
            }
            Step::Arc(rx, ry, rotation, _, _, to) => {
                let (center, sweep) = arc_center(at, &step);
                let rotation = if *rotation == 0.0 { 0.0 } else { -rotation };
                out.last_mut().unwrap().0.push(Got::Arc(
                    at,
                    *to,
                    center,
                    [*rx, *ry],
                    rotation,
                    sweep,
                ));
                at = *to;
            }
            Step::Close => out.last_mut().unwrap().1 = true,
            Step::Quadratic(..) | Step::Cubic(..) => panic!("no curves expected here"),
        }
    }
    out
}

fn got_length(pieces: &[Got]) -> f64 {
    pieces
        .iter()
        .map(|piece| match piece {
            Got::Line(a, b) => libm::hypot(b[0] - a[0], b[1] - a[1]),
            Got::Arc(_, _, _, r, _, sweep) => {
                assert_eq!(r[0], r[1], "a circular arc");
                r[0] * sweep.abs()
            }
        })
        .sum()
}

// ----------------------------------------------------- hand-written files

/// A triangle with one side an arc, and a circle: small enough to write
/// the expected files by hand.
fn tiny() -> Drawing {
    Drawing {
        unit: Unit::Millimeter,
        layers: vec![Layer {
            name: "cut".into(),
            color: Color::Red,
            dash: None,
            dot: false,
            paths: vec![
                closed(
                    [0.0, 0.0],
                    vec![line([10.0, 0.0]), arc([0.0, 0.0], [5.0, 0.0], 5.0, true)],
                ),
                Path::Circle {
                    center: [5.0, 2.0],
                    radius: 1.0,
                },
            ],
        }],
    }
}

#[test]
fn a_tiny_drawing_is_the_svg_written_by_hand() {
    let (svg, report) = write_svg(&tiny(), &SvgOptions::default()).unwrap();
    let expected = concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
        "<svg xmlns=\"http://www.w3.org/2000/svg\" version=\"1.1\" width=\"10mm\" height=\"5mm\" viewBox=\"0 -5 10 5\">\n",
        "<path id=\"layer-cut\" d=\"M0,0 L10,0 A5,5 0 0 0 5,-5 A5,5 0 0 0 0,0 Z ",
        "M6,-2 A1,1 0 0 0 5,-3 A1,1 0 0 0 4,-2 A1,1 0 0 0 5,-1 A1,1 0 0 0 6,-2 Z\" ",
        "fill=\"none\" fill-rule=\"evenodd\" stroke=\"#ff0000\" stroke-width=\"0.01\"/>\n",
        "</svg>\n",
    );
    assert_eq!(svg, expected);
    assert_eq!(report.view_box, [0.0, -5.0, 10.0, 5.0]);
    assert_eq!((report.approximated, report.deviation), (0, 0.0));
}

#[test]
fn a_tiny_drawing_is_the_dxf_entities_written_by_hand() {
    let dxf = write_dxf(&tiny()).unwrap();
    let start = dxf.find("  2\nENTITIES\n").unwrap();
    let end = dxf.find("  2\nOBJECTS\n").unwrap();
    let pairs = |groups: &[(&str, &str)]| -> String {
        groups
            .iter()
            .map(|(code, value)| format!("{code:>3}\n{value}\n"))
            .collect()
    };
    let expected = pairs(&[
        ("2", "ENTITIES"),
        ("0", "LINE"),
        ("5", "101"),
        ("330", "1F"),
        ("100", "AcDbEntity"),
        ("8", "cut"),
        ("100", "AcDbLine"),
        ("10", "0.0"),
        ("20", "0.0"),
        ("30", "0.0"),
        ("11", "10.0"),
        ("21", "0.0"),
        ("31", "0.0"),
        ("0", "ARC"),
        ("5", "102"),
        ("330", "1F"),
        ("100", "AcDbEntity"),
        ("8", "cut"),
        ("100", "AcDbCircle"),
        ("10", "5.0"),
        ("20", "0.0"),
        ("30", "0.0"),
        ("40", "5.0"),
        ("100", "AcDbArc"),
        ("50", "0.0"),
        ("51", "180.0"),
        ("0", "CIRCLE"),
        ("5", "103"),
        ("330", "1F"),
        ("100", "AcDbEntity"),
        ("8", "cut"),
        ("100", "AcDbCircle"),
        ("10", "5.0"),
        ("20", "2.0"),
        ("30", "0.0"),
        ("40", "1.0"),
        ("0", "ENDSEC"),
        ("0", "SECTION"),
    ]);
    assert_eq!(&dxf[start..end], expected);
    // The frame around them: version, unit, extents, the next handle, a
    // layer with its colour, and the end of the file.
    let file = Dxf::read(&dxf);
    assert_eq!(file.header("$ACADVER"), "AC1015");
    assert_eq!(DXF_VERSION, "AC1015");
    assert_eq!(file.header("$INSUNITS"), "4");
    assert_eq!(file.header("$MEASUREMENT"), "1");
    assert_eq!(file.header("$HANDSEED"), "104");
    let header = file.section("HEADER");
    let extents = |name: &str| -> Vec<String> {
        let at = header.iter().position(|g| *g == (9, name.into())).unwrap();
        header[at + 1..at + 4].iter().map(|g| g.1.clone()).collect()
    };
    assert_eq!(extents("$EXTMIN"), ["0.0", "0.0", "0.0"]);
    assert_eq!(extents("$EXTMAX"), ["10.0", "5.0", "0.0"]);
    let tables = file.section("TABLES");
    let layer = tables.iter().position(|g| *g == (2, "cut".into())).unwrap();
    assert_eq!(tables[layer + 2], (62, "1".to_string()));
    assert!(dxf.ends_with("  0\nENDSEC\n  0\nEOF\n"));
    for name in [
        "HEADER", "CLASSES", "TABLES", "BLOCKS", "ENTITIES", "OBJECTS",
    ] {
        file.section(name);
    }
    assert!(dxf.is_ascii());
}

#[test]
fn every_handle_of_a_dxf_file_is_used_once_and_lies_below_the_seed() {
    let dxf = write_dxf(&plate()).unwrap();
    let file = Dxf::read(&dxf);
    let seed = u64::from_str_radix(file.header("$HANDSEED"), 16).unwrap();
    let mut handles: Vec<u64> = file
        .groups
        .iter()
        .scan(false, |in_header, g| {
            if *g == (2, "HEADER".to_string()) {
                *in_header = true;
            } else if *g == (0, "ENDSEC".to_string()) {
                *in_header = false;
            }
            Some((*in_header, g))
        })
        .filter(|(in_header, g)| !in_header && (g.0 == 5 || g.0 == 105))
        .map(|(_, g)| u64::from_str_radix(&g.1, 16).unwrap())
        .collect();
    let count = handles.len();
    handles.sort_unstable();
    handles.dedup();
    assert_eq!(handles.len(), count, "a handle is used twice");
    assert!(handles.iter().all(|h| *h > 0 && *h < seed));
    // Every owner that is named exists.
    for g in file.groups.iter().filter(|g| g.0 == 330 && g.1 != "0") {
        let owner = u64::from_str_radix(&g.1, 16).unwrap();
        assert!(handles.contains(&owner), "owner {} does not exist", g.1);
    }
}

// ------------------------------------------------------------ round trips

/// The pieces of every path, as the model has them: what the readers must
/// give back.
fn expected_pieces(drawing: &Drawing) -> Vec<Vec<Piece<'_>>> {
    drawing
        .layers
        .iter()
        .flat_map(|layer| &layer.paths)
        .map(Path::pieces)
        .collect()
}

fn check_dxf(drawing: &Drawing, size: f64) {
    let dxf = write_dxf(drawing).unwrap();
    let entities = Dxf::read(&dxf).entities();
    let pieces: Vec<Piece<'_>> = expected_pieces(drawing).into_iter().flatten().collect();
    assert_eq!(entities.len(), pieces.len());
    let mut length = 0.0;
    for ((layer, entity), piece) in entities.iter().zip(&pieces) {
        assert_eq!(layer, "outline");
        length += entity_length(entity);
        match (entity, piece) {
            (Entity::Line(a, b), Piece::Line { from, to }) => {
                // End points are stored: they come back bit for bit.
                assert_eq!((a, b), (from, to));
            }
            (
                Entity::Circle(c, r),
                Piece::Arc {
                    center,
                    radius,
                    sweep,
                    ..
                },
            ) => {
                assert_eq!((c, r), (center, radius));
                assert_eq!(sweep.abs(), TAU);
            }
            (
                Entity::Arc(c, r, a0, a1),
                Piece::Arc {
                    from,
                    to,
                    center,
                    radius,
                    start,
                    sweep,
                },
            ) => {
                assert_eq!((c, r), (center, radius));
                // Counter-clockwise from a0 to a1: for a clockwise piece
                // that is from its end to its start.
                let (first, last) = if *sweep > 0.0 { (from, to) } else { (to, from) };
                let on = |a: f64| [c[0] + r * libm::cos(a), c[1] + r * libm::sin(a)];
                assert!(near_point(on(*a0), *first, size), "{entity:?} {piece:?}");
                assert!(near_point(on(*a1), *last, size), "{entity:?} {piece:?}");
                let run = (a1 - a0).rem_euclid(TAU);
                assert!(near(run, sweep.abs(), TAU), "{run} {sweep}");
                let begin = if *sweep > 0.0 { *start } else { start + sweep };
                assert!(near(
                    (a0 - begin)
                        .rem_euclid(TAU)
                        .min(TAU - (a0 - begin).rem_euclid(TAU)),
                    0.0,
                    TAU
                ));
            }
            other => panic!("the reader got something else: {other:?}"),
        }
    }
    assert!(near(length, drawing.length(), size), "{length}");
}

fn check_svg(drawing: &Drawing, size: f64) {
    let (svg, report) = write_svg(drawing, &SvgOptions::default()).unwrap();
    assert_eq!(report.approximated, 0);
    let paths = svg_paths(&svg);
    assert_eq!(paths.len(), 1);
    assert_eq!(attribute(paths[0], "fill"), "none");
    assert_eq!(attribute(paths[0], "fill-rule"), "evenodd");
    let got = subpaths(attribute(paths[0], "d"));
    let expected = expected_pieces(drawing);
    assert_eq!(got.len(), expected.len());
    let mut length = 0.0;
    for (index, ((got, is_closed), pieces)) in got.iter().zip(&expected).enumerate() {
        let path = drawing.layers[0].paths.get(index).unwrap();
        assert_eq!(*is_closed, path.is_closed());
        length += got_length(got);
        let mut got = got.iter();
        for piece in pieces {
            match piece {
                Piece::Line { from, to } => {
                    let Some(Got::Line(a, b)) = got.next() else {
                        panic!("a line expected")
                    };
                    assert_eq!((a, b), (from, to));
                }
                Piece::Arc {
                    from,
                    to,
                    center,
                    radius,
                    sweep,
                    ..
                } => {
                    // One arc, or the two halves of a long one.
                    let mut total = 0.0;
                    let mut at = *from;
                    while !near(total, *sweep, TAU) {
                        let Some(Got::Arc(a, b, c, r, _, part)) = got.next() else {
                            panic!("an arc expected")
                        };
                        assert!(near_point(*a, at, size));
                        assert_eq!(r, &[*radius, *radius]);
                        assert!(
                            near_point(*c, *center, size.max(norm(*center))),
                            "{c:?} {center:?}"
                        );
                        // The same way round.
                        assert_eq!(part.signum(), sweep.signum());
                        total += part;
                        at = *b;
                        assert!(total.abs() <= sweep.abs() + 1e-9, "{total} {sweep}");
                    }
                    assert_eq!(at, *to, "the end of an arc is stored");
                }
                Piece::Ellipse { .. } | Piece::Spline { .. } => panic!("not in these drawings"),
            }
        }
        assert!(got.next().is_none());
    }
    assert!(near(length, drawing.length(), size), "{length}");
}

#[test]
fn arcs_of_either_direction_and_size_come_back_from_both_files() {
    let r = 5.0;
    let p = |a: f64| [3.0 + r * libm::cos(a), -2.0 + r * libm::sin(a)];
    let c = [3.0, -2.0];
    for (a0, a1, ccw) in [
        (0.3, 1.1, true),
        (1.1, 0.3, false),
        // More than a half turn, both ways.
        (0.3, 0.3 + 4.0, true),
        (0.3 + 4.0, 0.3, false),
        // More than three quarters: written as two halves in SVG.
        (0.3, 0.3 + 5.9, true),
        (-1.0, -1.0 - 6.2, false),
        // Across the angle 0 and across ±π.
        (-0.4, 0.4, true),
        (3.0, -3.0, true),
        (3.0, -3.0, false),
        // A half turn exactly.
        (0.0, PI, true),
        (FRAC_PI_2, -FRAC_PI_2, false),
    ] {
        let drawing = drawing(vec![open(p(a0), vec![arc(p(a1), c, r, ccw)])]);
        // The model's own account of the arc first.
        let Piece::Arc { sweep, .. } = &drawing.layers[0].paths[0].pieces()[0] else {
            panic!()
        };
        let run = if ccw { a1 - a0 } else { a0 - a1 }.rem_euclid(TAU);
        assert!(near(sweep.abs(), run, TAU), "{a0} {a1} {ccw}: {sweep}");
        assert_eq!(*sweep > 0.0, ccw);
        assert!(near(drawing.length(), r * run, r * TAU));
        check_dxf(&drawing, 10.0);
        check_svg(&drawing, 10.0);
    }
}

#[test]
fn the_plate_comes_back_from_both_files_at_every_scale() {
    for k in [1e-3, 1.0, 25.4, 1e3, 1e6] {
        let drawing = scaled(&plate(), k);
        let size = 60.0 * k;
        assert!(near(drawing.length(), plate_length() * k, size));
        let bbox = drawing.bbox().unwrap();
        for (got, expected) in bbox.iter().zip([0.0, 0.0, 60.0 * k, 40.0 * k]) {
            assert!(near(*got, expected, size), "{bbox:?}");
        }
        check_dxf(&drawing, size);
        check_svg(&drawing, size);
        let counts = drawing.counts();
        assert_eq!(
            (
                counts.paths,
                counts.closed,
                counts.lines,
                counts.arcs,
                counts.circles
            ),
            (4, 4, 7, 7, 1)
        );
    }
}

#[test]
fn a_full_circle_is_a_circle_in_dxf_and_four_quarter_arcs_in_svg() {
    let drawing = drawing(vec![Path::Circle {
        center: [-7.0, 3.5],
        radius: 2.25,
    }]);
    let entities = Dxf::read(&write_dxf(&drawing).unwrap()).entities();
    assert_eq!(entities[0].1, Entity::Circle([-7.0, 3.5], 2.25));
    let (svg, _) = write_svg(&drawing, &SvgOptions::default()).unwrap();
    let got = subpaths(attribute(svg_paths(&svg)[0], "d"));
    assert_eq!(got.len(), 1);
    assert!(got[0].1);
    assert_eq!(got[0].0.len(), 4);
    let mut total = 0.0;
    for piece in &got[0].0 {
        let Got::Arc(_, _, c, r, _, sweep) = piece else {
            panic!()
        };
        assert!(near_point(*c, [-7.0, 3.5], 10.0));
        assert_eq!(r, &[2.25, 2.25]);
        total += sweep;
    }
    assert!(near(total, TAU, TAU));
    assert_eq!(attribute(&svg, "viewBox"), "-9.25 -5.75 4.5 4.5");
    assert_eq!(attribute(&svg, "width"), "4.5mm");
}

// --------------------------------------------------------------- ellipses

#[test]
fn ellipses_keep_their_axes_and_direction_in_both_files() {
    // Turned 30°, semi-axes 4 and 1.5.
    let turn = PI / 6.0;
    let major = [4.0 * libm::cos(turn), 4.0 * libm::sin(turn)];
    let (center, ratio) = ([2.0, -1.0], 0.375);
    let on = |t: f64| ellipse_point(center, major, ratio, t);
    let whole = Path::Ellipse {
        center,
        major,
        ratio,
    };
    // Ramanujan's second formula is good to 1e-9 here; the series is exact.
    let length = drawing(vec![whole.clone()]).length();
    let (a, b) = (4.0_f64, 1.5_f64);
    let h = libm::pow((a - b) / (a + b), 2.0);
    let mut series = 0.0;
    let mut term = 1.0_f64;
    for n in 0..60 {
        series += term * term * libm::pow(h, f64::from(n));
        // Binomial(1/2, n + 1) from binomial(1/2, n).
        term *= (0.5 - f64::from(n)) / (f64::from(n) + 1.0);
    }
    assert!(near(length, PI * (a + b) * series, 1e3), "{length}");

    for (t0, t1, ccw) in [
        (0.2, 1.9, true),
        (1.9, 0.2, false),
        (-2.5, 2.9, true),
        (0.5, 0.6, false),
    ] {
        let path = open(
            on(t0),
            vec![Segment::EllipticArc {
                to: on(t1),
                center,
                major,
                ratio,
                ccw,
            }],
        );
        let drawing = drawing(vec![path, whole.clone()]);
        let run = if ccw { t1 - t0 } else { t0 - t1 }.rem_euclid(TAU);
        let entities = Dxf::read(&write_dxf(&drawing).unwrap()).entities();
        let Entity::Ellipse(c, m, k, p0, p1) = &entities[0].1 else {
            panic!()
        };
        assert_eq!((c, m, k), (&center, &major, &ratio));
        assert!((0.0..TAU).contains(p0) && p1 > p0);
        assert!(near(p1 - p0, run, TAU));
        let (first, last) = if ccw { (t0, t1) } else { (t1, t0) };
        assert!(near_point(on(*p0), on(first), 10.0));
        assert!(near_point(on(*p1), on(last), 10.0));
        assert_eq!(
            entities[1].1,
            Entity::Ellipse(center, major, ratio, 0.0, TAU)
        );

        let (svg, _) = write_svg(&drawing, &SvgOptions::default()).unwrap();
        let got = subpaths(attribute(svg_paths(&svg)[0], "d"));
        assert_eq!(got.len(), 2);
        assert!(!got[0].1 && got[1].1);
        let mut total = 0.0;
        for piece in &got[0].0 {
            let Got::Arc(_, _, c, r, rotation, sweep) = piece else {
                panic!()
            };
            assert!(near_point(*c, center, 10.0), "{c:?}");
            assert!(near(r[0], 4.0, 4.0) && near(r[1], 1.5, 4.0));
            assert!(near(*rotation, 30.0, 360.0));
            total += sweep;
        }
        // In the ellipse's own parameter, the way it was drawn.
        assert!(
            near(total, if ccw { run } else { -run }, TAU),
            "{total} {run}"
        );
        let whole: f64 = got[1]
            .0
            .iter()
            .map(|piece| match piece {
                Got::Arc(.., sweep) => *sweep,
                Got::Line(..) => panic!(),
            })
            .sum();
        assert!(near(whole, TAU, TAU));
    }
}

#[test]
fn the_bounding_box_of_arcs_and_ellipses_is_exact() {
    // A quarter arc that does not reach any axis extreme but its ends.
    let quarter = drawing(vec![open(
        [1.0, 0.0],
        vec![arc([0.0, 1.0], [0.0, 0.0], 1.0, true)],
    )]);
    assert_eq!(quarter.bbox().unwrap(), [0.0, 0.0, 1.0, 1.0]);
    // The other three quarters reach three of them.
    let rest = drawing(vec![open(
        [1.0, 0.0],
        vec![arc([0.0, 1.0], [0.0, 0.0], 1.0, false)],
    )]);
    let bbox = rest.bbox().unwrap();
    for (got, expected) in bbox.iter().zip([-1.0, -1.0, 1.0, 1.0]) {
        assert!(near(*got, expected, 1.0), "{bbox:?}");
    }
    // An ellipse turned 45° with semi-axes 2 and 1: half extents
    // sqrt((4 + 1) / 2) both ways.
    let s = std::f64::consts::FRAC_1_SQRT_2;
    let turned = drawing(vec![Path::Ellipse {
        center: [10.0, 20.0],
        major: [2.0 * s, 2.0 * s],
        ratio: 0.5,
    }]);
    let half = 2.5_f64.sqrt();
    let bbox = turned.bbox().unwrap();
    for (got, expected) in bbox
        .iter()
        .zip([10.0 - half, 20.0 - half, 10.0 + half, 20.0 + half])
    {
        assert!(near(*got, expected, 20.0), "{bbox:?}");
    }
}

// ---------------------------------------------------------------- splines

/// A B-spline basis function by the Cox-de Boor recursion: slow, and
/// nothing like the code under test.
fn basis(i: usize, p: usize, knots: &[f64], t: f64) -> f64 {
    if p == 0 {
        let last = knots[knots.len() - 1];
        let inside = knots[i] <= t && t < knots[i + 1];
        // The end of the domain belongs to the last span that has length.
        let at_end = t == last && knots[i] < t && knots[i + 1] == last;
        return f64::from(u8::from(inside || at_end));
    }
    let mut value = 0.0;
    let left = knots[i + p] - knots[i];
    if left > 0.0 {
        value += (t - knots[i]) / left * basis(i, p - 1, knots, t);
    }
    let right = knots[i + p + 1] - knots[i + 1];
    if right > 0.0 {
        value += (knots[i + p + 1] - t) / right * basis(i + 1, p - 1, knots, t);
    }
    value
}

fn reference(
    degree: usize,
    knots: &[f64],
    control: &[Point],
    weights: Option<&[f64]>,
    t: f64,
) -> Point {
    let mut sum = [0.0, 0.0, 0.0];
    for (i, p) in control.iter().enumerate() {
        let w = weights.map_or(1.0, |w| w[i]) * basis(i, degree, knots, t);
        sum = [sum[0] + w * p[0], sum[1] + w * p[1], sum[2] + w];
    }
    [sum[0] / sum[2], sum[1] / sum[2]]
}

fn wavy() -> Spline {
    Spline::new(
        3,
        vec![0.0, 0.0, 0.0, 0.0, 1.0, 2.0, 2.0, 4.0, 5.0, 5.0, 5.0, 5.0],
        vec![
            [0.0, 0.0],
            [1.0, 3.0],
            [3.0, 4.0],
            [5.0, -1.0],
            [6.0, 2.0],
            [8.0, 3.0],
            [9.0, 0.0],
            [10.0, 1.0],
        ],
        None,
    )
    .unwrap()
}

/// A quarter of the unit circle as a rational quadratic.
fn quarter_circle() -> Spline {
    Spline::new(
        2,
        vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
        vec![[1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
        Some(vec![1.0, std::f64::consts::FRAC_1_SQRT_2, 1.0]),
    )
    .unwrap()
}

fn samples(a: f64, b: f64) -> impl Iterator<Item = f64> {
    (0..=40).map(move |i| a + (b - a) * f64::from(i) / 40.0)
}

#[test]
fn a_spline_evaluates_as_the_basis_functions_say() {
    let spline = wavy();
    for t in samples(0.0, 5.0) {
        let expected = reference(3, spline.knots(), spline.control_points(), None, t);
        assert!(near_point(spline.eval(t).unwrap(), expected, 10.0), "{t}");
    }
    let circle = quarter_circle();
    for t in samples(0.0, 1.0) {
        let p = circle.eval(t).unwrap();
        assert!(near(libm::hypot(p[0], p[1]), 1.0, 1.0));
    }
    assert_eq!(spline.start(), [0.0, 0.0]);
    assert_eq!(spline.end(), [10.0, 1.0]);
}

#[test]
fn bezier_pieces_pieces_between_parameters_and_the_reverse_are_the_same_curve() {
    let spline = wavy();
    // Bézier pieces: one per span, each the curve on its span.
    let pieces = spline.beziers().unwrap();
    let spans = [(0.0, 1.0), (1.0, 2.0), (2.0, 4.0), (4.0, 5.0)];
    assert_eq!(pieces.len(), spans.len());
    for (piece, (a, b)) in pieces.iter().zip(spans) {
        assert_eq!(piece.degree(), 3);
        for s in samples(0.0, 1.0) {
            let expected = spline.eval(a + (b - a) * s).unwrap();
            assert!(near_point(piece.eval(s), expected, 10.0));
        }
        // The derivative against a central difference.
        let h = 1e-6;
        let d = piece.derivative(0.4);
        let (p, q) = (piece.eval(0.4 - h), piece.eval(0.4 + h));
        assert!((d[0] - (q[0] - p[0]) / (2.0 * h)).abs() < 1e-6);
        assert!((d[1] - (q[1] - p[1]) / (2.0 * h)).abs() < 1e-6);
    }
    for pair in pieces.windows(2) {
        assert_eq!(pair[0].control[3], pair[1].control[0]);
    }
    // A piece between two parameters, on knots and off them.
    for (a, b) in [(0.5, 3.25), (1.0, 4.0), (0.0, 2.0), (2.0, 5.0), (4.9, 5.0)] {
        let part = spline.between(a, b).unwrap();
        assert_eq!(part.domain(), (a, b));
        for t in samples(a, b) {
            assert!(near_point(
                part.eval(t).unwrap(),
                spline.eval(t).unwrap(),
                10.0
            ));
        }
        assert!(near_point(part.start(), spline.eval(a).unwrap(), 10.0));
        assert!(near_point(part.end(), spline.eval(b).unwrap(), 10.0));
    }
    // The reverse.
    let back = spline.reversed();
    for t in samples(0.0, 5.0) {
        assert!(near_point(
            back.eval(5.0 - t).unwrap(),
            spline.eval(t).unwrap(),
            10.0
        ));
    }
    assert_eq!(back.reversed(), spline);
    // Rational: the pieces of a half circle of two quarters stay on it.
    let s = std::f64::consts::FRAC_1_SQRT_2;
    let half = Spline::new(
        2,
        vec![0.0, 0.0, 0.0, 1.0, 1.0, 2.0, 2.0, 2.0],
        vec![[1.0, 0.0], [1.0, 1.0], [0.0, 1.0], [-1.0, 1.0], [-1.0, 0.0]],
        Some(vec![1.0, s, 1.0, s, 1.0]),
    )
    .unwrap();
    let part = half.between(0.3, 1.6).unwrap();
    assert!(part.is_rational());
    for piece in part.beziers().unwrap() {
        for t in samples(0.0, 1.0) {
            let p = piece.eval(t);
            assert!(near(libm::hypot(p[0], p[1]), 1.0, 1.0));
            let d = piece.derivative(t);
            // The tangent of a circle is square to its radius.
            assert!((d[0] * p[0] + d[1] * p[1]).abs() < 1e-12);
        }
    }
}

#[test]
fn a_spline_with_open_knots_is_cut_to_its_domain() {
    // A uniform cubic: the curve lives on [3, 5] of knots 0..=8.
    let knots: Vec<f64> = (0..9).map(f64::from).collect();
    let control = vec![[0.0, 0.0], [1.0, 2.0], [3.0, 3.0], [4.0, 0.0], [6.0, -1.0]];
    let spline = Spline::new(3, knots.clone(), control.clone(), None).unwrap();
    assert_eq!(spline.domain(), (3.0, 5.0));
    assert_eq!(&spline.knots()[..4], &[3.0; 4]);
    assert_eq!(&spline.knots()[spline.knots().len() - 4..], &[5.0; 4]);
    for t in samples(3.0, 5.0) {
        let expected = reference(3, &knots, &control, None, t);
        assert!(near_point(spline.eval(t).unwrap(), expected, 10.0), "{t}");
    }
    assert!(near_point(
        spline.start(),
        reference(3, &knots, &control, None, 3.0),
        10.0
    ));
}

#[test]
fn a_cubic_spline_is_exact_beziers_in_svg_and_itself_in_dxf() {
    let spline = wavy();
    let drawing = drawing(vec![open(
        [-2.0, 0.0],
        vec![line([0.0, 0.0]), Segment::Spline(spline.clone())],
    )]);
    let dxf = write_dxf(&drawing).unwrap();
    let entities = Dxf::read(&dxf).entities();
    assert_eq!(entities.len(), 2);
    assert_eq!(
        entities[1].1,
        Entity::Spline {
            flags: 8,
            degree: 3,
            knots: spline.knots().to_vec(),
            weights: vec![],
            control: spline.control_points().to_vec(),
        }
    );

    let (svg, report) = write_svg(&drawing, &SvgOptions::default()).unwrap();
    assert_eq!((report.approximated, report.deviation), (0, 0.0));
    let steps = steps(attribute(svg_paths(&svg)[0], "d"));
    assert_eq!(steps.len(), 6);
    assert_eq!(steps[0], Step::Move([-2.0, 0.0]));
    assert_eq!(steps[1], Step::Line([0.0, 0.0]));
    let spans = [(0.0, 1.0), (1.0, 2.0), (2.0, 4.0), (4.0, 5.0)];
    let mut at = [0.0, 0.0];
    for (step, (a, b)) in steps[2..].iter().zip(spans) {
        let Step::Cubic(c1, c2, to) = step else {
            panic!("{step:?}")
        };
        let cubic = Bezier {
            control: vec![at, *c1, *c2, *to],
            weights: None,
        };
        for s in samples(0.0, 1.0) {
            let expected = spline.eval(a + (b - a) * s).unwrap();
            assert!(near_point(cubic.eval(s), expected, 10.0));
        }
        at = *to;
    }
    assert_eq!(at, spline.end());

    // Degree 2 is a quadratic Bézier, degree 1 a line.
    let parabola = Spline::new(
        2,
        vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
        vec![[0.0, 0.0], [1.0, 2.0], [2.0, 0.0]],
        None,
    )
    .unwrap();
    let polyline = Spline::new(
        1,
        vec![0.0, 0.0, 1.0, 2.0, 2.0],
        vec![[2.0, 0.0], [3.0, 1.0], [4.0, 0.0]],
        None,
    )
    .unwrap();
    let both = self::drawing(vec![open(
        [0.0, 0.0],
        vec![Segment::Spline(parabola), Segment::Spline(polyline)],
    )]);
    let (svg, report) = write_svg(&both, &SvgOptions::default()).unwrap();
    assert_eq!(report.approximated, 0);
    assert_eq!(
        attribute(svg_paths(&svg)[0], "d"),
        "M0,0 Q1,-2 2,0 L3,-1 L4,0"
    );
}

#[test]
fn a_rational_or_high_degree_spline_is_itself_in_dxf_and_within_tolerance_in_svg() {
    let circle = quarter_circle();
    // Degree 5 with a bump.
    let quintic = Spline::new(
        5,
        vec![0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0],
        vec![
            [0.0, 1.0],
            [-1.0, 2.0],
            [-3.0, 5.0],
            [-4.0, -3.0],
            [-5.0, 1.0],
            [-6.0, 0.0],
        ],
        None,
    )
    .unwrap();
    let drawing = drawing(vec![open(
        [1.0, 0.0],
        vec![
            Segment::Spline(circle.clone()),
            Segment::Spline(quintic.clone()),
        ],
    )]);
    let entities = Dxf::read(&write_dxf(&drawing).unwrap()).entities();
    assert_eq!(
        entities[0].1,
        Entity::Spline {
            flags: 12,
            degree: 2,
            knots: circle.knots().to_vec(),
            weights: circle.weights().unwrap().to_vec(),
            control: circle.control_points().to_vec(),
        }
    );
    let Entity::Spline {
        flags,
        degree,
        weights,
        ..
    } = &entities[1].1
    else {
        panic!()
    };
    assert_eq!((*flags, *degree, weights.len()), (8, 5, 0));

    for tolerance in [1e-3, 1e-6, 1e-9] {
        let options = SvgOptions {
            tolerance,
            ..SvgOptions::default()
        };
        let (svg, report) = write_svg(&drawing, &options).unwrap();
        assert_eq!(report.approximated, 2);
        assert!(report.deviation > 0.0 && report.deviation <= tolerance);
        let steps = steps(attribute(svg_paths(&svg)[0], "d"));
        // The cubics that stand in for the quarter circle end at (0, 1);
        // every point of them is within the tolerance of the circle.
        let mut at = [1.0, 0.0];
        let mut on_circle = true;
        let mut worst = 0.0_f64;
        let mut count = 0;
        for step in &steps[1..] {
            let Step::Cubic(c1, c2, to) = step else {
                panic!("{step:?}")
            };
            let cubic = Bezier {
                control: vec![at, *c1, *c2, *to],
                weights: None,
            };
            if on_circle {
                count += 1;
                for s in samples(0.0, 1.0) {
                    let p = cubic.eval(s);
                    worst = worst.max((libm::hypot(p[0], p[1]) - 1.0).abs());
                }
            }
            at = *to;
            if at == [0.0, 1.0] {
                on_circle = false;
            }
        }
        assert!(!on_circle, "the quarter circle ends at its own end point");
        assert_eq!(at, [-6.0, 0.0]);
        assert!(worst <= tolerance, "{worst} > {tolerance}");
        println!(
            "tolerance {tolerance:e}: {count} cubics for the quarter circle, {} in all, off by {worst:e} (reported {:e})",
            steps.len() - 1,
            report.deviation
        );
    }
}

#[test]
fn spline_lengths_are_integrated() {
    // A quarter of the unit circle.
    let drawing = drawing(vec![open(
        [1.0, 0.0],
        vec![Segment::Spline(quarter_circle())],
    )]);
    assert!(
        near(drawing.length(), FRAC_PI_2, 1.0),
        "{}",
        drawing.length()
    );
}

// ------------------------------------------------------------------ units

#[test]
fn the_unit_is_in_both_files() {
    for (unit, insunits, metric, width, height) in [
        (Unit::Millimeter, "4", "1", "60mm", "40mm"),
        (Unit::Inch, "1", "0", "60in", "40in"),
        (Unit::Centimeter, "5", "1", "60cm", "40cm"),
        (Unit::Meter, "6", "1", "60000mm", "40000mm"),
        (Unit::Micron, "13", "1", "0.06mm", "0.04mm"),
        (Unit::Foot, "2", "0", "720in", "480in"),
    ] {
        let drawing = Drawing { unit, ..plate() };
        let file = Dxf::read(&write_dxf(&drawing).unwrap());
        assert_eq!(file.header("$INSUNITS"), insunits);
        assert_eq!(file.header("$MEASUREMENT"), metric);
        let (svg, _) = write_svg(&drawing, &SvgOptions::default()).unwrap();
        assert_eq!(attribute(&svg, "width"), width);
        assert_eq!(attribute(&svg, "height"), height);
        // One user unit is one drawing unit, whatever the unit.
        assert_eq!(attribute(&svg, "viewBox"), "0 -40 60 40");
        // The hairline is 0.01 mm in drawing units.
        let stroke: f64 = attribute(svg_paths(&svg)[0], "stroke-width")
            .parse()
            .unwrap();
        assert!(near(stroke * unit.millimetres(), HAIRLINE_MM, 1.0));
    }
}

#[test]
fn layers_keep_their_names_and_colours() {
    let mut drawing = plate();
    drawing.layers.push(Layer {
        name: "engrave-2".into(),
        color: Color::Blue,
        dash: None,
        dot: false,
        paths: vec![open([1.0, 1.0], vec![line([2.0, 2.0])])],
    });
    drawing.layers.push(Layer {
        name: "empty".into(),
        color: Color::Green,
        dash: None,
        dot: false,
        paths: vec![],
    });
    let dxf = write_dxf(&drawing).unwrap();
    let file = Dxf::read(&dxf);
    let entities = file.entities();
    assert_eq!(entities.last().unwrap().0, "engrave-2");
    let tables = file.section("TABLES");
    for (name, color) in [
        ("0", "7"),
        ("outline", "7"),
        ("engrave-2", "5"),
        ("empty", "3"),
    ] {
        let at = tables
            .iter()
            .enumerate()
            .position(|(i, g)| {
                *g == (2, name.into()) && tables[i - 1] == (100, "AcDbLayerTableRecord".into())
            })
            .unwrap_or_else(|| panic!("no layer {name}"));
        assert_eq!(tables[at + 2], (62, color.to_string()));
    }
    let (svg, _) = write_svg(&drawing, &SvgOptions::default()).unwrap();
    let paths = svg_paths(&svg);
    assert_eq!(paths.len(), 2);
    assert_eq!(attribute(paths[1], "id"), "layer-engrave-2");
    assert_eq!(attribute(paths[1], "stroke"), "#0000ff");
    assert_eq!(attribute(paths[1], "d"), "M1,-1 L2,-2");
}

// ------------------------------------------------------------ determinism

#[test]
fn the_same_drawing_gives_the_same_bytes_and_numbers_read_back_bit_for_bit() {
    let odd = [
        0.1 + 0.2,
        1.0 / 3.0,
        1e-7,
        123_456_789.123_456_79,
        5e-324,
        1.7976931348623157e308,
    ];
    let drawing = drawing(
        odd.iter()
            .map(|x| open([*x, -x], vec![line([-x, x / 3.0])]))
            .collect(),
    );
    let dxf = write_dxf(&drawing).unwrap();
    assert_eq!(dxf, write_dxf(&drawing).unwrap());
    for ((_, entity), x) in Dxf::read(&dxf).entities().iter().zip(odd) {
        assert_eq!(*entity, Entity::Line([x, -x], [-x, x / 3.0]));
    }
    // Without the largest: its box has no finite width.
    let drawing = self::drawing(
        odd.iter()
            .take(5)
            .map(|x| open([*x, -x], vec![line([-x, x / 3.0])]))
            .collect(),
    );
    let (svg, _) = write_svg(&drawing, &SvgOptions::default()).unwrap();
    assert_eq!(svg, write_svg(&drawing, &SvgOptions::default()).unwrap().0);
    for ((got, _), x) in subpaths(attribute(svg_paths(&svg)[0], "d")).iter().zip(odd) {
        let Got::Line(a, b) = &got[0] else { panic!() };
        assert_eq!((*a, *b), ([x, -x], [-x, x / 3.0]));
    }
    assert_eq!(shortest(-0.0).unwrap(), "0");
    assert_eq!(shortest(0.1 + 0.2).unwrap(), "0.30000000000000004");
    assert_eq!(shortest(1e-7).unwrap(), "1E-7");
    assert_eq!(shortest(2.5e20).unwrap(), "2.5E20");
    assert_eq!(shortest(f64::NAN), None);
}

#[test]
fn a_drawing_without_height_still_has_a_box() {
    let drawing = drawing(vec![open([0.0, 3.0], vec![line([10.0, 3.0])])]);
    let (svg, report) = write_svg(&drawing, &SvgOptions::default()).unwrap();
    assert_eq!(report.view_box, [0.0, -3.0 - 0.01, 10.0, 0.02]);
    assert_eq!(attribute(&svg, "height"), "0.02mm");
    write_dxf(&drawing).unwrap();
}

// ----------------------------------------------------------------- errors

#[test]
fn what_cannot_be_drawn_is_an_error_that_says_why() {
    let bad_paths = [
        (open([f64::NAN, 0.0], vec![line([1.0, 0.0])]), "not finite"),
        (
            open([0.0, 0.0], vec![line([f64::INFINITY, 0.0])]),
            "not finite",
        ),
        (open([0.0, 0.0], vec![]), "at least one segment"),
        (
            open([1.0, 0.0], vec![arc([0.0, 1.0], [0.0, 0.0], 0.0, true)]),
            "radius must be positive",
        ),
        (
            open([1.0, 0.0], vec![arc([0.0, 1.0], [0.0, 0.0], -1.0, true)]),
            "radius must be positive",
        ),
        (
            open(
                [1.0, 0.0],
                vec![arc([0.0, 1.0], [0.0, 0.0], f64::NAN, true)],
            ),
            "not finite",
        ),
        (
            open([1.0, 0.0], vec![arc([1.0, 0.0], [0.0, 0.0], 1.0, true)]),
            "a whole circle is a path of its own",
        ),
        (
            open([1.0, 0.0], vec![arc([0.0, 1.0], [0.5, 0.0], 1.0, true)]),
            "not at its radius",
        ),
        (
            Path::Circle {
                center: [0.0, 0.0],
                radius: 0.0,
            },
            "radius must be positive",
        ),
        (
            Path::Circle {
                center: [0.0, f64::NAN],
                radius: 1.0,
            },
            "not finite",
        ),
        (
            Path::Ellipse {
                center: [0.0, 0.0],
                major: [0.0, 0.0],
                ratio: 0.5,
            },
            "semi-axis of some length",
        ),
        (
            Path::Ellipse {
                center: [0.0, 0.0],
                major: [1.0, 0.0],
                ratio: 1.5,
            },
            "at most 1 times",
        ),
        (
            Path::Ellipse {
                center: [0.0, 0.0],
                major: [1.0, 0.0],
                ratio: 0.0,
            },
            "more than 0",
        ),
        (
            open(
                [2.0, 0.0],
                vec![Segment::EllipticArc {
                    to: [0.0, 5.0],
                    center: [0.0, 0.0],
                    major: [2.0, 0.0],
                    ratio: 0.5,
                    ccw: true,
                }],
            ),
            "not on its ellipse",
        ),
    ];
    for (path, expected) in bad_paths {
        let drawing = drawing(vec![rounded_rectangle(4.0, 3.0, 1.0), path]);
        for error in [
            write_dxf(&drawing).unwrap_err(),
            write_svg(&drawing, &SvgOptions::default()).unwrap_err(),
        ] {
            let text = error.to_string();
            assert!(text.contains(expected), "`{text}` lacks `{expected}`");
            assert!(text.contains("layer 0, path 1"), "{text}");
        }
    }
    assert_eq!(write_dxf(&Drawing::default()), Err(DrawingError::Empty));
    assert_eq!(write_dxf(&drawing(vec![])), Err(DrawingError::Empty));
    assert!(write_svg(&drawing(vec![]), &SvgOptions::default()).is_err());

    for name in ["", "a b", "naïve", "x\ny", &"n".repeat(32), "a\"b", "<g>"] {
        let mut drawing = plate();
        drawing.layers[0].name = name.to_string();
        assert!(matches!(
            write_dxf(&drawing),
            Err(DrawingError::LayerName(_))
        ));
        assert!(matches!(
            write_svg(&drawing, &SvgOptions::default()),
            Err(DrawingError::LayerName(_))
        ));
    }
    let mut twice = plate();
    twice.layers.push(Layer {
        name: "OUTLINE".into(),
        ..Layer::default()
    });
    assert!(matches!(write_dxf(&twice), Err(DrawingError::LayerName(_))));

    for options in [
        SvgOptions {
            tolerance: 0.0,
            ..SvgOptions::default()
        },
        SvgOptions {
            tolerance: f64::NAN,
            ..SvgOptions::default()
        },
        SvgOptions {
            stroke_mm: -1.0,
            ..SvgOptions::default()
        },
        SvgOptions {
            stroke_mm: f64::INFINITY,
            ..SvgOptions::default()
        },
    ] {
        assert!(matches!(
            write_svg(&plate(), &options),
            Err(DrawingError::Option(_))
        ));
    }
    // A drawing too large for its box to be a number.
    let huge = drawing(vec![open([-1e308, 0.0], vec![line([1e308, 1.0])])]);
    assert!(write_svg(&huge, &SvgOptions::default()).is_err());
}

#[test]
fn a_spline_that_does_not_hold_together_is_refused() {
    let control = vec![[0.0, 0.0], [1.0, 1.0], [2.0, 0.0], [3.0, 1.0]];
    let clamped = vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0];
    let ok = Spline::new(3, clamped.clone(), control.clone(), None);
    assert!(ok.is_ok());
    type Case = (usize, Vec<f64>, Vec<Point>, Option<Vec<f64>>);
    let cases: Vec<Case> = vec![
        (0, clamped.clone(), control.clone(), None),
        (16, clamped.clone(), control.clone(), None),
        (usize::MAX, clamped.clone(), control.clone(), None),
        (4, clamped.clone(), control.clone(), None),
        (3, clamped[..7].to_vec(), control.clone(), None),
        (3, vec![], vec![], None),
        (3, vec![0.0; 8], control.clone(), None),
        (
            3,
            vec![0.0, 0.0, 0.0, 0.0, 1.0, 0.5, 1.0, 1.0],
            control.clone(),
            None,
        ),
        (
            3,
            vec![0.0, 0.0, 0.0, 0.0, f64::NAN, 1.0, 1.0, 1.0],
            control.clone(),
            None,
        ),
        (
            3,
            clamped.clone(),
            vec![[0.0, 0.0], [1.0, f64::INFINITY], [2.0, 0.0], [3.0, 1.0]],
            None,
        ),
        (
            3,
            clamped.clone(),
            control.clone(),
            Some(vec![1.0, 1.0, 1.0]),
        ),
        (
            3,
            clamped.clone(),
            control.clone(),
            Some(vec![1.0, 0.0, 1.0, 1.0]),
        ),
        (
            3,
            clamped.clone(),
            control.clone(),
            Some(vec![1.0, -2.0, 1.0, 1.0]),
        ),
        (
            3,
            clamped.clone(),
            control.clone(),
            Some(vec![1.0, f64::NAN, 1.0, 1.0]),
        ),
        // A knot inside four times: the curve breaks there.
        (1, vec![0.0, 0.0, 1.0, 1.0, 2.0, 2.0], control.clone(), None),
    ];
    for (degree, knots, control, weights) in cases {
        let result = Spline::new(degree, knots.clone(), control, weights);
        assert!(
            matches!(result, Err(DrawingError::InvalidSpline(_))),
            "degree {degree}, knots {knots:?}: {result:?}"
        );
    }
    // More control points than a spline may have.
    let many = MAX_SPLINE_CONTROL_POINTS + 1;
    let mut knots = vec![0.0; 2];
    knots.extend((1..many - 1).map(|i| i as f64));
    knots.extend([many as f64 - 1.0; 2]);
    assert!(Spline::new(1, knots, vec![[0.0, 0.0]; many], None).is_err());
    // Parameters that are no piece.
    let spline = ok.unwrap();
    for (a, b) in [
        (0.5, 0.5),
        (0.8, 0.2),
        (f64::NAN, 1.0),
        (0.0, f64::INFINITY),
        (2.0, 3.0),
    ] {
        assert!(spline.between(a, b).is_err(), "{a} {b}");
    }
    assert!(spline.eval(f64::NAN).is_err());
    // Weights that are all 1 are no weights.
    let plain = Spline::new(3, clamped, control, Some(vec![1.0; 4])).unwrap();
    assert!(!plain.is_rational());
}

// ------------------------------------------------- area and translation

#[test]
fn the_signed_area_of_a_path_is_its_closed_form_and_says_which_way_it_runs() {
    let r: f64 = 5.0;
    let rectangle = 60.0 * 40.0 - (4.0 - PI) * r * r;
    let slot = 20.0 * 6.0 + PI * 9.0;
    // The D: the circle less the segment the chord at 3 cuts off.
    let half = libm::acos(3.0_f64 / 5.0);
    let d = 25.0 / 2.0 * ((TAU - 2.0 * half) + libm::sin(2.0 * half));
    let cases = [
        (rounded_rectangle(60.0, 40.0, r), rectangle),
        (slot_path(), -slot),
        (d_hole([45.0, 28.0], 5.0, 3.0), d),
        (
            Path::Circle {
                center: [15.0, 28.0],
                radius: 4.0,
            },
            PI * 16.0,
        ),
        (
            Path::Ellipse {
                center: [3.0, -2.0],
                major: [3.0, 4.0],
                ratio: 0.5,
            },
            PI * 5.0 * 2.5,
        ),
        // A quarter disc closed by its two radii, as a rational spline.
        (
            closed(
                [0.0, 0.0],
                vec![
                    line([1.0, 0.0]),
                    Segment::Spline(quarter_circle()),
                    line([0.0, 0.0]),
                ],
            ),
            PI / 4.0,
        ),
    ];
    for (path, expected) in cases {
        for d in [[0.0, 0.0], [1e3, -7.5], [-0.25, 1e-3]] {
            let moved = path.translated(d);
            let area = moved.area();
            println!("{area} (expected {expected}) moved by {d:?}");
            assert!(
                (area - expected).abs() <= 1e-10 * expected.abs().max(1.0),
                "{area} {expected}"
            );
        }
    }
    // A polynomial spline closed by a line: the shoelace formula of its
    // Bézier control polygon does not hold, but a straight cubic does.
    let straight = Spline::new(
        3,
        vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
        vec![[4.0, 0.0], [4.0, 1.0], [4.0, 2.0], [4.0, 3.0]],
        None,
    )
    .unwrap();
    let triangle = closed(
        [0.0, 0.0],
        vec![line([4.0, 0.0]), Segment::Spline(straight)],
    );
    assert!((triangle.area() - 6.0).abs() < 1e-12, "{}", triangle.area());
    let open_chain = open([0.0, 0.0], vec![line([4.0, 0.0]), line([4.0, 3.0])]);
    assert!((open_chain.area() - 6.0).abs() < 1e-12);
}

fn slot_path() -> Path {
    slot(15.0, 35.0, 10.0, 3.0)
}

#[test]
fn a_translated_drawing_is_the_same_drawing_moved() {
    let plate = plate();
    let d = [12.5, -3.25];
    let mut moved = plate.clone();
    for layer in &mut moved.layers {
        for path in &mut layer.paths {
            *path = path.translated(d);
        }
    }
    let (a, b) = (plate.bbox().unwrap(), moved.bbox().unwrap());
    for (i, (a, b)) in a.iter().zip(b).enumerate() {
        assert!((a + d[i % 2] - b).abs() < 1e-12, "{a} {b}");
    }
    assert!((plate.length() - moved.length()).abs() < 1e-9);
    let spline = closed([0.0, 0.0], vec![Segment::Spline(wavy()), line([0.0, 0.0])]);
    let Path::Chain { segments, .. } = spline.translated(d) else {
        panic!()
    };
    let Segment::Spline(s) = &segments[0] else {
        panic!()
    };
    assert_eq!(s.knots(), wavy().knots());
    assert_eq!(s.start(), [d[0], d[1]]);
}

#[test]
fn a_dashed_layer_is_dashed_in_both_formats_and_others_are_as_before() {
    let plain = drawing(vec![open([0.0, 0.0], vec![line([10.0, 0.0])])]);
    let mut dashed = plain.clone();
    dashed.layers.push(Layer {
        name: "hidden".into(),
        color: Color::Black,
        dash: Some(2.0),
        dot: false,
        paths: vec![open([0.0, 5.0], vec![line([10.0, 5.0])])],
    });
    let before = write_dxf(&plain).unwrap();
    assert!(!before.contains("DASHED"));
    let dxf = write_dxf(&dashed).unwrap();
    // The line type: 3 long, a dash of 2 and a gap of 1.
    assert!(
        dxf.contains("  2\nDASHED\n 70\n0\n  3\nDashed __ __ __\n 72\n65\n 73\n2\n 40\n3.0\n 49\n2.0\n 74\n0\n 49\n-1.0\n 74\n0\n"),
        "{dxf}"
    );
    assert!(
        dxf.contains("  2\nhidden\n 70\n0\n 62\n7\n  6\nDASHED\n"),
        "{dxf}"
    );
    assert!(
        dxf.contains("  2\noutline\n 70\n0\n 62\n7\n  6\nContinuous\n"),
        "{dxf}"
    );
    let (svg, _) = write_svg(&dashed, &SvgOptions::default()).unwrap();
    assert!(
        svg.contains("id=\"layer-hidden\"") && svg.contains("stroke-dasharray=\"2 1\""),
        "{svg}"
    );
    assert_eq!(svg.matches("stroke-dasharray").count(), 1);
    // A dash that is no length is refused.
    dashed.layers[1].dash = Some(0.0);
    assert!(matches!(write_dxf(&dashed), Err(DrawingError::Option(_))));
    dashed.layers[1].dash = Some(f64::NAN);
    assert!(write_svg(&dashed, &SvgOptions::default()).is_err());
}

#[test]
fn a_dash_dot_layer_has_its_own_line_type_beside_a_dashed_one() {
    let mut both = drawing(vec![open([0.0, 0.0], vec![line([10.0, 0.0])])]);
    both.layers.push(Layer {
        name: "valley".into(),
        color: Color::Blue,
        dash: Some(2.0),
        dot: false,
        paths: vec![open([0.0, 5.0], vec![line([10.0, 5.0])])],
    });
    both.layers.push(Layer {
        name: "mountain".into(),
        color: Color::Red,
        dash: Some(4.0),
        dot: true,
        paths: vec![open([0.0, 8.0], vec![line([10.0, 8.0])])],
    });
    let dxf = write_dxf(&both).unwrap();
    // 6 long: a dash of 4, a gap of 1, a dot, a gap of 1.
    assert!(
        dxf.contains("  2\nDASHDOT\n 70\n0\n  3\nDash dot __ . __ . __\n 72\n65\n 73\n4\n 40\n6.0\n 49\n4.0\n 74\n0\n 49\n-1.0\n 74\n0\n 49\n0.0\n 74\n0\n 49\n-1.0\n 74\n0\n"),
        "{dxf}"
    );
    assert!(dxf.contains("  2\nDASHED\n 70\n0\n"), "{dxf}");
    assert!(
        dxf.contains("  2\nmountain\n 70\n0\n 62\n1\n  6\nDASHDOT\n"),
        "{dxf}"
    );
    assert!(
        dxf.contains("  2\nvalley\n 70\n0\n 62\n5\n  6\nDASHED\n"),
        "{dxf}"
    );
    // Five line types: ByBlock, ByLayer, Continuous, DASHED, DASHDOT.
    assert!(
        dxf.contains("  2\nLTYPE\n  5\n5\n330\n0\n100\nAcDbSymbolTable\n 70\n5\n"),
        "{dxf}"
    );
    let (svg, _) = write_svg(&both, &SvgOptions::default()).unwrap();
    assert!(svg.contains("stroke-dasharray=\"4 1 0.4 1\""), "{svg}");
    assert!(svg.contains("stroke-dasharray=\"2 1\""), "{svg}");
    // A dot without a dash is refused.
    both.layers[2].dash = None;
    assert!(matches!(write_dxf(&both), Err(DrawingError::Option(_))));
    assert!(write_svg(&both, &SvgOptions::default()).is_err());
}

/// A glyph as the engine writes it (docs/design/text.md): each Bézier
/// piece of a contour a SPLINE of its own, cubic. Read back by our own
/// reader, the pieces enclose the glyph's area: an "O" of eight quadratic
/// pieces each way, raised to cubic exactly, outer ring less hole.
#[test]
fn a_glyph_written_as_spline_pieces_reads_back_with_its_area() {
    // A ring of quadratic Béziers through points on a circle of radius r,
    // control points on the tangents' crossings, raised to cubic.
    fn ring(r: f64, ccw: bool) -> (Path, f64) {
        let n = 8;
        let mut segments = Vec::new();
        let point = |k: usize| {
            let a = k as f64 * std::f64::consts::TAU / n as f64 * if ccw { 1.0 } else { -1.0 };
            [r * a.cos(), r * a.sin()]
        };
        let half = std::f64::consts::PI / n as f64;
        let mut area = 0.0;
        for k in 0..n {
            let (a, c) = (point(k), point(k + 1));
            let mid = [(a[0] + c[0]) / 2.0, (a[1] + c[1]) / 2.0];
            let s = 1.0 / half.cos() / (mid[0].hypot(mid[1]) / r);
            let b = [mid[0] * s, mid[1] * s];
            let q = [
                a,
                [
                    a[0] + 2.0 / 3.0 * (b[0] - a[0]),
                    a[1] + 2.0 / 3.0 * (b[1] - a[1]),
                ],
                [
                    c[0] + 2.0 / 3.0 * (b[0] - c[0]),
                    c[1] + 2.0 / 3.0 * (b[1] - c[1]),
                ],
                c,
            ];
            // The quadratic's area term in closed form:
            // (a×b + b×c)/3 + (a×c)/6 halves of the shoelace.
            let x = |p: Point, q: Point| p[0] * q[1] - p[1] * q[0];
            area += (2.0 * x(a, b) + 2.0 * x(b, c) + x(a, c)) / 6.0;
            segments.push(Segment::Spline(
                Spline::new(
                    3,
                    vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
                    q.to_vec(),
                    None,
                )
                .unwrap(),
            ));
        }
        (closed(point(0), segments), area)
    }
    let (outer, a_out) = ring(10.0, true);
    let (inner, a_in) = ring(6.0, false);
    let expected = a_out + a_in;
    assert!(a_out > 0.0 && a_in < 0.0);
    let dxf = write_dxf(&drawing(vec![outer, inner])).unwrap();
    let entities = Dxf::read(&dxf).entities();
    assert_eq!(entities.len(), 16);
    // The area each cubic piece encloses with the origin, by Gauss–Legendre
    // (exact for the degree-5 integrand), from the control points read.
    let mut area = 0.0;
    for (_, e) in &entities {
        let Entity::Spline {
            degree,
            knots,
            control,
            ..
        } = e
        else {
            panic!("{e:?}")
        };
        assert_eq!((*degree, knots.len(), control.len()), (3, 8, 4));
        let at = |t: f64| {
            let u = 1.0 - t;
            let w = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
            let dw = [
                -3.0 * u * u,
                3.0 * u * u - 6.0 * u * t,
                6.0 * u * t - 3.0 * t * t,
                3.0 * t * t,
            ];
            let mut p = [0.0; 2];
            let mut d = [0.0; 2];
            for k in 0..4 {
                for c in 0..2 {
                    p[c] += w[k] * control[k][c];
                    d[c] += dw[k] * control[k][c];
                }
            }
            (p, d)
        };
        let (xs, ws) = (
            [
                0.046_910_077_030_668,
                0.230_765_344_947_158,
                0.5,
                0.769_234_655_052_842,
                0.953_089_922_969_332,
            ],
            [
                0.118_463_442_528_095,
                0.239_314_335_249_683,
                0.284_444_444_444_444,
                0.239_314_335_249_683,
                0.118_463_442_528_095,
            ],
        );
        for (t, w) in xs.iter().zip(ws) {
            let (p, d) = at(*t);
            area += w * (p[0] * d[1] - p[1] * d[0]) / 2.0;
        }
    }
    assert!(
        (area - expected).abs() < 1e-9 * expected.abs(),
        "{area} vs {expected}"
    );
}
