//! The DXF writer: ASCII DXF of AutoCAD 2000 (`AC1015`).
//!
//! That release is the oldest whose format has every entity a flat outline
//! needs as itself: `ELLIPSE` and `SPLINE` came with R13, and R13 and R14
//! files are rarer than those of 2000, which every reader of the last
//! twenty years takes. R12, the usual "safe" choice, has neither and would
//! force ellipses and splines into polylines.
//!
//! A loop is written as its own entities head to tail (`LINE`, `ARC`,
//! `ELLIPSE`, `SPLINE`), not as one `LWPOLYLINE`: a polyline holds lines
//! and circular arcs only (as bulges, from which a reader has to work the
//! centre and radius back out), so a loop with an ellipse or a spline in
//! it would fall apart anyway. Entities that meet share their end point
//! to the last bit where the format stores end points (lines, splines),
//! and laser and CAM software joins such entities into loops.

use crate::{Drawing, DrawingError, Piece, Point, Spline, norm, shortest};
use std::f64::consts::TAU;

/// The value of `$ACADVER` in the files [`write_dxf`] writes.
pub const DXF_VERSION: &str = "AC1015";

/// The handle of the model space block record: the owner of every entity.
const MODEL_SPACE: &str = "1F";
/// The name and the handle of the line type of a dashed layer.
const DASHED: &str = "DASHED";
const DASHED_HANDLE: &str = "17";
/// The name and the handle of the line type of a dash-dot layer.
const DASHDOT: &str = "DASHDOT";
const DASHDOT_HANDLE: &str = "18";
/// The first handle given to a layer or an entity.
const FIRST_FREE_HANDLE: u64 = 0x100;

struct Out {
    text: String,
    handle: u64,
}

impl Out {
    /// One group: the code right-aligned in three columns, then the value
    /// on its own line.
    fn group(&mut self, code: u16, value: &str) {
        let code = code.to_string();
        for _ in code.len()..3 {
            self.text.push(' ');
        }
        self.text.push_str(&code);
        self.text.push('\n');
        self.text.push_str(value);
        self.text.push('\n');
    }

    fn groups(&mut self, groups: &[(u16, &str)]) {
        for (code, value) in groups {
            self.group(*code, value);
        }
    }

    fn int(&mut self, code: u16, value: usize) {
        self.group(code, &value.to_string());
    }

    fn real(&mut self, code: u16, value: f64) -> Result<(), DrawingError> {
        let text = real(value).ok_or(DrawingError::NonFinite { layer: 0, path: 0 })?;
        self.group(code, &text);
        Ok(())
    }

    /// A point as groups `code`, `code + 10` and `code + 20`, with z 0.
    fn point(&mut self, code: u16, p: Point) -> Result<(), DrawingError> {
        self.real(code, p[0])?;
        self.real(code + 10, p[1])?;
        self.real(code + 20, 0.0)
    }

    fn next_handle(&mut self) -> String {
        let handle = format!("{:X}", self.handle);
        self.handle = self.handle.saturating_add(1);
        handle
    }

    /// The groups every entity starts with.
    fn entity(&mut self, kind: &str, layer: &str, class: &str) {
        let handle = self.next_handle();
        self.groups(&[
            (0, kind),
            (5, &handle),
            (330, MODEL_SPACE),
            (100, "AcDbEntity"),
            (8, layer),
            (100, class),
        ]);
    }
}

/// A real number as DXF text: the shortest decimal that reads back as the
/// same `f64`, always with a decimal point or an exponent, since some
/// readers take a bare `10` for an integer.
fn real(x: f64) -> Option<String> {
    let mut text = shortest(x)?;
    if !text.contains(['.', 'E']) {
        text.push_str(".0");
    }
    Some(text)
}

/// An angle in degrees in `[0, 360)`.
fn degrees(radians: f64) -> f64 {
    let d = radians.to_degrees().rem_euclid(360.0);
    if d < 360.0 { d } else { 0.0 }
}

fn angle_of(p: Point, center: Point) -> f64 {
    (p[1] - center[1]).atan2(p[0] - center[0])
}

/// Writes the drawing as an ASCII DXF file of AutoCAD 2000.
///
/// The header carries the unit (`$INSUNITS`, `$MEASUREMENT`) and the
/// extents; every layer of the drawing is a DXF layer with its colour, and
/// the entities take colour and line type from their layer. The same
/// drawing gives the same bytes: handles are counted up from a fixed
/// start, and there is no date or time in the file.
pub fn write_dxf(drawing: &Drawing) -> Result<String, DrawingError> {
    drawing.validate()?;
    let mut out = Out {
        text: String::new(),
        handle: FIRST_FREE_HANDLE,
    };
    let bbox = drawing.bbox().ok_or(DrawingError::Empty)?;
    let layers: Vec<(String, &crate::Layer)> = drawing
        .layers
        .iter()
        .filter(|layer| layer.name != "0")
        .map(|layer| (out.next_handle(), layer))
        .collect();
    let mut entities = Out {
        text: String::new(),
        handle: out.handle,
    };
    for (index, layer) in drawing.layers.iter().enumerate() {
        for (path_index, path) in layer.paths.iter().enumerate() {
            for piece in path.pieces() {
                write_piece(&mut entities, &layer.name, &piece).map_err(|e| match e {
                    DrawingError::NonFinite { .. } => DrawingError::NonFinite {
                        layer: index,
                        path: path_index,
                    },
                    other => other,
                })?;
            }
        }
    }
    let seed = format!("{:X}", entities.handle);

    // ---------------------------------------------------------- header
    out.groups(&[(0, "SECTION"), (2, "HEADER")]);
    out.groups(&[(9, "$ACADVER"), (1, DXF_VERSION)]);
    out.groups(&[(9, "$DWGCODEPAGE"), (3, "ANSI_1252")]);
    out.group(9, "$INSBASE");
    out.point(10, [0.0, 0.0])?;
    out.group(9, "$EXTMIN");
    out.point(10, [bbox[0], bbox[1]])?;
    out.group(9, "$EXTMAX");
    out.point(10, [bbox[2], bbox[3]])?;
    out.group(9, "$MEASUREMENT");
    out.int(70, usize::from(drawing.unit.is_metric()));
    out.group(9, "$INSUNITS");
    out.int(70, usize::from(drawing.unit.insunits()));
    out.groups(&[(9, "$HANDSEED"), (5, &seed)]);
    out.groups(&[(0, "ENDSEC")]);
    out.groups(&[(0, "SECTION"), (2, "CLASSES"), (0, "ENDSEC")]);

    // ---------------------------------------------------------- tables
    out.groups(&[(0, "SECTION"), (2, "TABLES")]);
    let table = |out: &mut Out, name: &str, handle: &str, entries: usize| {
        out.groups(&[
            (0, "TABLE"),
            (2, name),
            (5, handle),
            (330, "0"),
            (100, "AcDbSymbolTable"),
        ]);
        out.int(70, entries);
    };
    let record = |out: &mut Out, kind: &str, handle: &str, owner: &str, class: &str| {
        out.groups(&[
            (0, kind),
            (5, handle),
            (330, owner),
            (100, "AcDbSymbolTableRecord"),
            (100, class),
        ]);
    };

    table(&mut out, "VPORT", "8", 1);
    record(&mut out, "VPORT", "29", "8", "AcDbViewportTableRecord");
    out.groups(&[(2, "*Active"), (70, "0")]);
    let (width, height) = (bbox[2] - bbox[0], bbox[3] - bbox[1]);
    let usable = |v: f64| v.is_finite() && v > 0.0;
    let view_height = [height, width]
        .into_iter()
        .find(|v| usable(*v))
        .unwrap_or(1.0);
    let aspect = Some(width / height)
        .filter(|v| usable(*v))
        .map_or(1.0, |v| v.max(1.0));
    let middle = |a: f64, b: f64| Some(a / 2.0 + b / 2.0).filter(|v| v.is_finite());
    out.real(10, 0.0)?;
    out.real(20, 0.0)?;
    out.real(11, 1.0)?;
    out.real(21, 1.0)?;
    // The centre of the view, its height and its aspect: the drawing.
    out.real(12, middle(bbox[0], bbox[2]).unwrap_or(0.0))?;
    out.real(22, middle(bbox[1], bbox[3]).unwrap_or(0.0))?;
    for code in [13, 23] {
        out.real(code, 0.0)?;
    }
    for code in [14, 24, 15, 25] {
        out.real(code, 1.0)?;
    }
    out.point(16, [0.0, 0.0])?;
    out.real(36, 1.0)?;
    out.point(17, [0.0, 0.0])?;
    out.real(40, view_height)?;
    out.real(41, aspect)?;
    out.real(42, 50.0)?;
    for code in [43, 44, 50, 51] {
        out.real(code, 0.0)?;
    }
    out.groups(&[
        (71, "0"),
        (72, "100"),
        (73, "1"),
        (74, "3"),
        (75, "0"),
        (76, "0"),
        (77, "0"),
        (78, "0"),
    ]);
    out.groups(&[(0, "ENDTAB")]);

    // A dashed layer's line type, only when there is one (a drawing
    // without one is written as before, byte for byte): the first dashed
    // layer's dash, a gap half as long.
    let dash = drawing
        .layers
        .iter()
        .filter(|layer| !layer.dot)
        .find_map(|layer| layer.dash);
    // A dash-dot layer's: the first one's dash, gaps a quarter of it.
    let dash_dot = drawing
        .layers
        .iter()
        .filter(|layer| layer.dot)
        .find_map(|layer| layer.dash);
    let types = 3 + usize::from(dash.is_some()) + usize::from(dash_dot.is_some());
    table(&mut out, "LTYPE", "5", types);
    for (handle, name, description) in [
        ("14", "ByBlock", ""),
        ("15", "ByLayer", ""),
        ("16", "Continuous", "Solid line"),
    ] {
        record(&mut out, "LTYPE", handle, "5", "AcDbLinetypeTableRecord");
        out.groups(&[
            (2, name),
            (70, "0"),
            (3, description),
            (72, "65"),
            (73, "0"),
            (40, "0.0"),
        ]);
    }
    if let Some(dash) = dash {
        record(
            &mut out,
            "LTYPE",
            DASHED_HANDLE,
            "5",
            "AcDbLinetypeTableRecord",
        );
        out.groups(&[
            (2, DASHED),
            (70, "0"),
            (3, "Dashed __ __ __"),
            (72, "65"),
            (73, "2"),
        ]);
        out.real(40, dash * 1.5)?;
        out.real(49, dash)?;
        out.group(74, "0");
        out.real(49, -dash * 0.5)?;
        out.group(74, "0");
    }
    if let Some(dash) = dash_dot {
        record(
            &mut out,
            "LTYPE",
            DASHDOT_HANDLE,
            "5",
            "AcDbLinetypeTableRecord",
        );
        out.groups(&[
            (2, DASHDOT),
            (70, "0"),
            (3, "Dash dot __ . __ . __"),
            (72, "65"),
            (73, "4"),
        ]);
        out.real(40, dash * 1.5)?;
        out.real(49, dash)?;
        out.group(74, "0");
        out.real(49, -dash * 0.25)?;
        out.group(74, "0");
        out.real(49, 0.0)?;
        out.group(74, "0");
        out.real(49, -dash * 0.25)?;
        out.group(74, "0");
    }
    out.groups(&[(0, "ENDTAB")]);

    table(&mut out, "LAYER", "2", layers.len().saturating_add(1));
    let zero = drawing.layers.iter().find(|layer| layer.name == "0");
    let zero_color = zero.map_or(7, |layer| layer.color.aci()).to_string();
    record(&mut out, "LAYER", "10", "2", "AcDbLayerTableRecord");
    out.groups(&[(2, "0"), (70, "0"), (62, &zero_color), (6, "Continuous")]);
    for (handle, layer) in &layers {
        record(&mut out, "LAYER", handle, "2", "AcDbLayerTableRecord");
        out.groups(&[
            (2, &layer.name),
            (70, "0"),
            (62, &layer.color.aci().to_string()),
            (
                6,
                match (layer.dash, layer.dot) {
                    (Some(_), true) => DASHDOT,
                    (Some(_), false) => DASHED,
                    (None, _) => "Continuous",
                },
            ),
        ]);
    }
    out.groups(&[(0, "ENDTAB")]);

    table(&mut out, "STYLE", "3", 1);
    record(&mut out, "STYLE", "11", "3", "AcDbTextStyleTableRecord");
    out.groups(&[
        (2, "Standard"),
        (70, "0"),
        (40, "0.0"),
        (41, "1.0"),
        (50, "0.0"),
        (71, "0"),
        (42, "2.5"),
        (3, "txt"),
        (4, ""),
    ]);
    out.groups(&[(0, "ENDTAB")]);

    table(&mut out, "VIEW", "6", 0);
    out.groups(&[(0, "ENDTAB")]);
    table(&mut out, "UCS", "7", 0);
    out.groups(&[(0, "ENDTAB")]);

    table(&mut out, "APPID", "9", 1);
    record(&mut out, "APPID", "12", "9", "AcDbRegAppTableRecord");
    out.groups(&[(2, "ACAD"), (70, "0")]);
    out.groups(&[(0, "ENDTAB")]);

    table(&mut out, "DIMSTYLE", "A", 1);
    out.groups(&[(100, "AcDbDimStyleTable")]);
    // A dimension style has its handle in group 105.
    out.groups(&[
        (0, "DIMSTYLE"),
        (105, "27"),
        (330, "A"),
        (100, "AcDbSymbolTableRecord"),
        (100, "AcDbDimStyleTableRecord"),
        (2, "Standard"),
        (70, "0"),
        (340, "11"),
    ]);
    out.groups(&[(0, "ENDTAB")]);

    table(&mut out, "BLOCK_RECORD", "1", 2);
    for (handle, name) in [(MODEL_SPACE, "*Model_Space"), ("1B", "*Paper_Space")] {
        record(
            &mut out,
            "BLOCK_RECORD",
            handle,
            "1",
            "AcDbBlockTableRecord",
        );
        out.groups(&[(2, name)]);
    }
    out.groups(&[(0, "ENDTAB")]);
    out.groups(&[(0, "ENDSEC")]);

    // ---------------------------------------------------------- blocks
    out.groups(&[(0, "SECTION"), (2, "BLOCKS")]);
    for (begin, end, owner, name, paper) in [
        ("20", "21", MODEL_SPACE, "*Model_Space", false),
        ("1C", "1D", "1B", "*Paper_Space", true),
    ] {
        out.groups(&[(0, "BLOCK"), (5, begin), (330, owner), (100, "AcDbEntity")]);
        if paper {
            out.groups(&[(67, "1")]);
        }
        out.groups(&[(8, "0"), (100, "AcDbBlockBegin"), (2, name), (70, "0")]);
        out.point(10, [0.0, 0.0])?;
        out.groups(&[(3, name), (1, "")]);
        out.groups(&[(0, "ENDBLK"), (5, end), (330, owner), (100, "AcDbEntity")]);
        if paper {
            out.groups(&[(67, "1")]);
        }
        out.groups(&[(8, "0"), (100, "AcDbBlockEnd")]);
    }
    out.groups(&[(0, "ENDSEC")]);

    // -------------------------------------------------------- entities
    out.groups(&[(0, "SECTION"), (2, "ENTITIES")]);
    out.text.push_str(&entities.text);
    out.groups(&[(0, "ENDSEC")]);

    // --------------------------------------------------------- objects
    out.groups(&[(0, "SECTION"), (2, "OBJECTS")]);
    out.groups(&[
        (0, "DICTIONARY"),
        (5, "C"),
        (330, "0"),
        (100, "AcDbDictionary"),
        (281, "1"),
        (3, "ACAD_GROUP"),
        (350, "D"),
    ]);
    out.groups(&[
        (0, "DICTIONARY"),
        (5, "D"),
        (330, "C"),
        (100, "AcDbDictionary"),
        (281, "1"),
    ]);
    out.groups(&[(0, "ENDSEC"), (0, "EOF")]);
    Ok(out.text)
}

fn write_piece(out: &mut Out, layer: &str, piece: &Piece<'_>) -> Result<(), DrawingError> {
    match piece {
        Piece::Line { from, to } => {
            out.entity("LINE", layer, "AcDbLine");
            out.point(10, *from)?;
            out.point(11, *to)
        }
        Piece::Arc {
            from,
            to,
            center,
            radius,
            sweep,
            ..
        } => {
            let whole = sweep.abs() >= TAU;
            out.entity(if whole { "CIRCLE" } else { "ARC" }, layer, "AcDbCircle");
            out.point(10, *center)?;
            out.real(40, *radius)?;
            if whole {
                return Ok(());
            }
            // An ARC runs counter-clockwise from its start angle to its
            // end angle; a clockwise piece is the same arc from its end.
            let (first, last) = if *sweep >= 0.0 {
                (from, to)
            } else {
                (to, from)
            };
            out.group(100, "AcDbArc");
            out.real(50, degrees(angle_of(*first, *center)))?;
            out.real(51, degrees(angle_of(*last, *center)))
        }
        Piece::Ellipse {
            center,
            major,
            ratio,
            start,
            sweep,
            ..
        } => {
            out.entity("ELLIPSE", layer, "AcDbEllipse");
            out.point(10, *center)?;
            // The end of the longer axis, from the centre.
            out.point(11, *major)?;
            out.real(210, 0.0)?;
            out.real(220, 0.0)?;
            out.real(230, 1.0)?;
            out.real(40, *ratio)?;
            // Counter-clockwise from the first parameter to the second.
            let (first, last) = if sweep.abs() >= TAU {
                (0.0, TAU)
            } else {
                let first = if *sweep >= 0.0 {
                    *start
                } else {
                    *start + *sweep
                };
                let first = first.rem_euclid(TAU);
                let first = if first < TAU { first } else { 0.0 };
                (first, first + sweep.abs())
            };
            out.real(41, first)?;
            out.real(42, last)
        }
        Piece::Spline { spline } => write_spline(out, layer, spline),
    }
}

fn write_spline(out: &mut Out, layer: &str, spline: &Spline) -> Result<(), DrawingError> {
    out.entity("SPLINE", layer, "AcDbSpline");
    out.real(210, 0.0)?;
    out.real(220, 0.0)?;
    out.real(230, 1.0)?;
    // 8: planar; 4: rational.
    out.int(70, if spline.is_rational() { 12 } else { 8 });
    out.int(71, spline.degree());
    out.int(72, spline.knots().len());
    out.int(73, spline.control_points().len());
    out.int(74, 0);
    // Knot and control point tolerances: a scale-free small number, as
    // the files of other writers carry.
    let size = spline
        .control_points()
        .iter()
        .map(|p| norm(*p))
        .fold(0.0, f64::max);
    out.real(42, 1e-10)?;
    out.real(43, if size > 0.0 { size * 1e-10 } else { 1e-10 })?;
    for knot in spline.knots() {
        out.real(40, *knot)?;
    }
    if let Some(weights) = spline.weights() {
        for weight in weights {
            out.real(41, *weight)?;
        }
    }
    for p in spline.control_points() {
        out.point(10, *p)?;
    }
    Ok(())
}
