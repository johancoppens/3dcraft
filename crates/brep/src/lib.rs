//! Reader of the text BRep format ("CASCADE Topology V1/V2/V3"), the
//! format FreeCAD stores the shapes of a `.FCStd` document in, into its
//! own format-level model.
//!
//! Standalone: depends on no workspace crate (docs/architecture.md §2),
//! like `craft3d-step`. Written from the published description of the
//! format (Open CASCADE Technology's "BRep Format" documentation, which
//! describes every section and record) and from files FreeCAD writes; no
//! code of Open CASCADE was read or used. What the numbers mean as
//! geometry and topology is the business of `craft3d-fcstdread`.
//!
//! # What a file is
//!
//! A header line, then sections, each a name and a count: `Locations`,
//! `Curve2ds`, `Curves`, `Polygon3D`, `PolygonOnTriangulations`,
//! `Surfaces`, `Triangulations`, `TShapes`, then the root shape. Read:
//!
//! - **Locations**: elementary ones (`1` and a 3 × 4 matrix, rows of the
//!   rotation with the translation last) and composite ones (`2` and pairs
//!   of an earlier location's number and a power, ending in `0`), the
//!   product of the earlier ones raised to their powers, in order.
//! - **Curves** (3D): line (1), circle (2), ellipse (3), parabola (4),
//!   hyperbola (5), Bézier (6), B-spline (7, rational or not, periodic or
//!   not), trimmed (8), offset (9).
//! - **Surfaces**: plane (1), cylinder (2), cone (3), sphere (4), torus
//!   (5), linear extrusion (6), revolution (7), Bézier (8), B-spline (9),
//!   rectangular trimmed (10), offset (11).
//! - **TShapes**: vertices (tolerance, point), edges (tolerance, flags, the
//!   3D curve with its location and range; other representations are
//!   skipped), wires, faces (tolerance, surface and its location), shells,
//!   solids, composite solids and compounds, each with its sub-shapes
//!   (orientation, shape number, location number).
//!
//! The 2D curves, polygons and triangulations are skipped: a body here is
//! built from 3D curves and surfaces.
//!
//! # Hostile input
//!
//! Every count is checked against what is left of the file before
//! anything is reserved, every number must be finite, every reference
//! must name a location, curve, surface or shape that exists, a composite
//! location may refer only to earlier ones and a shape only to shapes
//! written before it (so nothing refers to itself), trimmed and offset
//! curves and surfaces nest at most [`MAX_NESTING`] deep, and degrees are
//! at most [`MAX_DEGREE`]. Anything else is an error that names the
//! section and record, never a panic.

use std::fmt;

/// The deepest a trimmed or offset curve or surface may nest.
pub const MAX_NESTING: usize = 8;
/// The highest B-spline or Bézier degree read (the format's own limit).
pub const MAX_DEGREE: usize = 25;
/// The most records, poles or knots reserved room for before they are
/// read: counts come from the file, so room beyond this grows only as
/// records are actually parsed (each takes some bytes of the file), and
/// what a file costs stays a small multiple of its size.
const RESERVE: usize = 4096;

/// An empty vector with room for `n` items, at most [`RESERVE`].
fn reserve<T>(n: usize) -> Vec<T> {
    Vec::with_capacity(n.min(RESERVE))
}

/// The largest power a composite location may raise another to.
pub const MAX_POWER: i64 = 1 << 20;

/// Why a file could not be read: where, and what.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrepError(pub String);

impl fmt::Display for BrepError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for BrepError {}

fn bad<T>(message: impl Into<String>) -> Result<T, BrepError> {
    Err(BrepError(message.into()))
}

/// A point or a vector.
pub type Xyz = [f64; 3];

/// A 3 × 4 matrix: the rows of a linear map with the translation as the
/// fourth column.
pub type Matrix = [[f64; 4]; 3];

pub const IDENTITY: Matrix = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
];

/// One record of the `Locations` section.
#[derive(Clone, Debug, PartialEq)]
pub enum Location {
    Elementary(Matrix),
    /// (earlier location number, power), in order.
    Composite(Vec<(usize, i64)>),
}

/// A placement of an elementary curve or surface: origin, main axis
/// (`n`), x and y directions. The y direction is given, not derived:
/// a frame may be left-handed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Axes {
    pub origin: Xyz,
    pub n: Xyz,
    pub x: Xyz,
    pub y: Xyz,
}

/// A 3D curve record.
#[derive(Clone, Debug, PartialEq)]
pub enum Curve {
    Line {
        origin: Xyz,
        direction: Xyz,
    },
    Circle {
        axes: Axes,
        radius: f64,
    },
    Ellipse {
        axes: Axes,
        major: f64,
        minor: f64,
    },
    Parabola {
        axes: Axes,
        focal: f64,
    },
    Hyperbola {
        axes: Axes,
        major: f64,
        minor: f64,
    },
    Bezier {
        poles: Vec<Xyz>,
        /// Empty when not rational.
        weights: Vec<f64>,
    },
    BSpline(BSplineCurve),
    Trimmed {
        first: f64,
        last: f64,
        basis: Box<Curve>,
    },
    Offset {
        distance: f64,
        direction: Xyz,
        basis: Box<Curve>,
    },
}

/// A B-spline curve as the file gives it: distinct knots with their
/// multiplicities.
#[derive(Clone, Debug, PartialEq)]
pub struct BSplineCurve {
    pub degree: usize,
    pub periodic: bool,
    pub poles: Vec<Xyz>,
    /// Empty when not rational.
    pub weights: Vec<f64>,
    pub knots: Vec<f64>,
    pub mults: Vec<usize>,
}

/// A B-spline surface: `poles` row by row, `count_u` rows of `count_v`.
#[derive(Clone, Debug, PartialEq)]
pub struct BSplineSurface {
    pub degree_u: usize,
    pub degree_v: usize,
    pub periodic_u: bool,
    pub periodic_v: bool,
    pub count_u: usize,
    pub count_v: usize,
    pub poles: Vec<Xyz>,
    /// Empty when not rational.
    pub weights: Vec<f64>,
    pub knots_u: Vec<f64>,
    pub mults_u: Vec<usize>,
    pub knots_v: Vec<f64>,
    pub mults_v: Vec<usize>,
}

/// A surface record.
#[derive(Clone, Debug, PartialEq)]
pub enum Surface {
    Plane {
        axes: Axes,
    },
    Cylinder {
        axes: Axes,
        radius: f64,
    },
    Cone {
        axes: Axes,
        radius: f64,
        semi_angle: f64,
    },
    Sphere {
        axes: Axes,
        radius: f64,
    },
    Torus {
        axes: Axes,
        major: f64,
        minor: f64,
    },
    Extrusion {
        direction: Xyz,
        basis: Curve,
    },
    Revolution {
        origin: Xyz,
        direction: Xyz,
        basis: Curve,
    },
    Bezier {
        count_u: usize,
        count_v: usize,
        poles: Vec<Xyz>,
        weights: Vec<f64>,
    },
    BSpline(Box<BSplineSurface>),
    Trimmed {
        u: (f64, f64),
        v: (f64, f64),
        basis: Box<Surface>,
    },
    Offset {
        distance: f64,
        basis: Box<Surface>,
    },
}

/// How a shape is used by the one that holds it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Orientation {
    Forward,
    Reversed,
    Internal,
    External,
}

impl Orientation {
    /// This orientation seen through a holder used with `outer`.
    pub fn compose(self, outer: Orientation) -> Orientation {
        use Orientation::*;
        match (outer, self) {
            (Forward, o) => o,
            (Reversed, Forward) => Reversed,
            (Reversed, Reversed) => Forward,
            (Reversed, o) => o,
            (Internal, _) => Internal,
            (External, _) => External,
        }
    }
}

/// A use of a shape: how it is oriented, which shape (its number in the
/// file, counted from the last written, as the file counts) and where
/// (a location number, 0 for none).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sub {
    pub orientation: Orientation,
    pub shape: usize,
    pub location: usize,
}

/// The 3D curve of an edge: curve number, location number, range.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EdgeCurve {
    pub curve: usize,
    pub location: usize,
    pub first: f64,
    pub last: f64,
}

/// What a shape is, with its own data.
#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Vertex {
        tolerance: f64,
        point: Xyz,
    },
    Edge {
        tolerance: f64,
        degenerated: bool,
        /// The first 3D curve representation, if any.
        curve: Option<EdgeCurve>,
    },
    Wire,
    Face {
        tolerance: f64,
        /// Surface number (0 when the face has none).
        surface: usize,
        location: usize,
    },
    Shell,
    Solid,
    CompSolid,
    Compound,
}

impl Kind {
    pub fn name(&self) -> &'static str {
        match self {
            Kind::Vertex { .. } => "vertex",
            Kind::Edge { .. } => "edge",
            Kind::Wire => "wire",
            Kind::Face { .. } => "face",
            Kind::Shell => "shell",
            Kind::Solid => "solid",
            Kind::CompSolid => "composite solid",
            Kind::Compound => "compound",
        }
    }
}

/// One record of the `TShapes` section.
#[derive(Clone, Debug, PartialEq)]
pub struct TShape {
    pub kind: Kind,
    /// The `closed` flag.
    pub closed: bool,
    pub children: Vec<Sub>,
}

/// A whole file.
#[derive(Clone, Debug, PartialEq)]
pub struct Brep {
    /// 1, 2 or 3, from the header.
    pub version: u32,
    pub locations: Vec<Location>,
    pub curves: Vec<Curve>,
    pub surfaces: Vec<Surface>,
    /// In the order written: the shape numbered `k` is
    /// `shapes[shapes.len() - k]` ([`Brep::shape`]).
    pub shapes: Vec<TShape>,
    pub root: Option<Sub>,
}

impl Brep {
    /// The shape numbered `number` (1 the last written).
    pub fn shape(&self, number: usize) -> Option<&TShape> {
        let at = self.shapes.len().checked_sub(number)?;
        if number == 0 {
            return None;
        }
        self.shapes.get(at)
    }

    /// Curve number `number` (from 1).
    pub fn curve(&self, number: usize) -> Option<&Curve> {
        self.curves.get(number.checked_sub(1)?)
    }

    /// Surface number `number` (from 1).
    pub fn surface(&self, number: usize) -> Option<&Surface> {
        self.surfaces.get(number.checked_sub(1)?)
    }

    /// The matrix of every location, by number from 1 (`[0]` is
    /// location 1), composite ones multiplied out: `None` for one with a
    /// part that cannot be inverted. Each composite refers only to earlier
    /// locations, so one pass computes them all.
    ///
    /// A composite `2 a p b q 0` is the location `b^q` applied after
    /// `a^p`: the parts are listed in the order they apply (a shape moved
    /// by `a` and then by `b`), which is how FreeCAD's files read
    /// (`moved.brp` in `craft3d-fcstdread`'s tests, against FreeCAD's
    /// bounding box).
    pub fn matrices(&self) -> Vec<Option<Matrix>> {
        let mut done: Vec<Option<Matrix>> = Vec::with_capacity(self.locations.len());
        for location in &self.locations {
            let m = match location {
                Location::Elementary(m) => Some(*m),
                Location::Composite(parts) => {
                    let mut m = Some(IDENTITY);
                    for &(k, power) in parts {
                        let part = k
                            .checked_sub(1)
                            .and_then(|i| done.get(i))
                            .copied()
                            .flatten();
                        m = match (m, part.and_then(|p| power_of(&p, power))) {
                            (Some(m), Some(p)) => Some(multiply(&p, &m)),
                            _ => None,
                        };
                    }
                    m
                }
            };
            done.push(m);
        }
        done
    }

    /// The matrix of location number `number` (0: the identity); see
    /// [`Brep::matrices`], which computes all of them at once.
    pub fn matrix(&self, number: usize) -> Option<Matrix> {
        if number == 0 {
            return Some(IDENTITY);
        }
        self.matrices().get(number - 1).copied().flatten()
    }
}

/// `a ∘ b`: first `b`, then `a`.
pub fn multiply(a: &Matrix, b: &Matrix) -> Matrix {
    let mut out = [[0.0; 4]; 3];
    for (i, row) in out.iter_mut().enumerate() {
        let ar = a.get(i).copied().unwrap_or([0.0; 4]);
        for (j, cell) in row.iter_mut().enumerate() {
            let mut sum = 0.0;
            for (k, &aik) in ar.iter().take(3).enumerate() {
                sum += aik * b.get(k).and_then(|r| r.get(j)).copied().unwrap_or(0.0);
            }
            if j == 3 {
                sum += ar.get(3).copied().unwrap_or(0.0);
            }
            *cell = sum;
        }
    }
    out
}

/// The inverse of an affine map, `None` when it is singular.
pub fn invert(m: &Matrix) -> Option<Matrix> {
    let [r0, r1, r2] = *m;
    let [a, b, c, tx] = r0;
    let [d, e, f, ty] = r1;
    let [g, h, i, tz] = r2;
    let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    if !(det.is_finite() && det.abs() > 1e-300) {
        return None;
    }
    let r0 = [
        (e * i - f * h) / det,
        (c * h - b * i) / det,
        (b * f - c * e) / det,
    ];
    let r1 = [
        (f * g - d * i) / det,
        (a * i - c * g) / det,
        (c * d - a * f) / det,
    ];
    let r2 = [
        (d * h - e * g) / det,
        (b * g - a * h) / det,
        (a * e - b * d) / det,
    ];
    let row = |[p, q, r]: [f64; 3]| [p, q, r, -(p * tx + q * ty + r * tz)];
    let out = [row(r0), row(r1), row(r2)];
    Some(out)
}

fn power_of(m: &Matrix, power: i64) -> Option<Matrix> {
    let base = if power < 0 { invert(m)? } else { *m };
    let mut n = power.unsigned_abs();
    let mut result = IDENTITY;
    let mut square = base;
    while n > 0 {
        if n & 1 == 1 {
            result = multiply(&result, &square);
        }
        square = multiply(&square, &square);
        n >>= 1;
    }
    Some(result)
}

/// The section names a file may have; a skipped section ends where the
/// next of these begins a line.
const SECTIONS: [&str; 8] = [
    "Curve2ds",
    "Curves",
    "Polygon2D",
    "Polygon3D",
    "PolygonOnTriangulations",
    "Surfaces",
    "Triangulations",
    "TShapes",
];

struct Cursor<'a> {
    text: &'a str,
    at: usize,
    /// Where we are, for messages.
    context: String,
}

impl<'a> Cursor<'a> {
    fn left(&self) -> usize {
        self.text.len().saturating_sub(self.at)
    }

    fn fail<T>(&self, what: &str) -> Result<T, BrepError> {
        bad(format!("{}: {what}", self.context))
    }

    fn token(&mut self) -> Option<&'a str> {
        let rest = self.text.get(self.at..)?;
        let start = rest.len() - rest.trim_start().len();
        let rest = rest.get(start..)?;
        let len = rest
            .find(|c: char| c.is_ascii_whitespace())
            .unwrap_or(rest.len());
        if len == 0 {
            self.at = self.text.len();
            return None;
        }
        self.at += start + len;
        rest.get(..len)
    }

    fn need(&mut self, what: &str) -> Result<&'a str, BrepError> {
        match self.token() {
            Some(t) => Ok(t),
            None => self.fail(&format!("the file ends where {what} should be")),
        }
    }

    fn real(&mut self, what: &str) -> Result<f64, BrepError> {
        let token = self.need(what)?;
        match token.parse::<f64>() {
            Ok(x) if x.is_finite() => Ok(x),
            _ => self.fail(&format!("{what} `{}` is not a finite number", clip(token))),
        }
    }

    fn integer(&mut self, what: &str) -> Result<i64, BrepError> {
        let token = self.need(what)?;
        match token.parse::<i64>() {
            Ok(x) => Ok(x),
            _ => self.fail(&format!("{what} `{}` is not an integer", clip(token))),
        }
    }

    /// A count or number from 0 up to `max`.
    fn count(&mut self, what: &str, max: usize) -> Result<usize, BrepError> {
        let n = self.integer(what)?;
        match usize::try_from(n) {
            Ok(n) if n <= max => Ok(n),
            _ => self.fail(&format!("{what} {n} is out of range (0 to {max})")),
        }
    }

    fn flag(&mut self, what: &str) -> Result<bool, BrepError> {
        match self.integer(what)? {
            0 => Ok(false),
            1 => Ok(true),
            n => self.fail(&format!("{what} is {n}, not 0 or 1")),
        }
    }

    fn xyz(&mut self, what: &str) -> Result<Xyz, BrepError> {
        Ok([self.real(what)?, self.real(what)?, self.real(what)?])
    }

    /// The rest of the current line, and moves past it.
    fn line(&mut self) -> Option<&'a str> {
        let rest = self.text.get(self.at..)?;
        if rest.is_empty() {
            return None;
        }
        let len = rest.find('\n').unwrap_or(rest.len());
        self.at += (len + 1).min(rest.len());
        rest.get(..len)
    }

    /// Moves to the start of the next line that begins with one of
    /// [`SECTIONS`], or to the end.
    fn skip_section(&mut self) {
        while let Some(rest) = self.text.get(self.at..) {
            if rest.is_empty() {
                return;
            }
            let first = rest.split_ascii_whitespace().next().unwrap_or("");
            let line_start = rest.trim_start_matches([' ', '\t', '\r']);
            if SECTIONS.contains(&first) && line_start.starts_with(first) {
                return;
            }
            if self.line().is_none() {
                return;
            }
        }
    }
}

/// A token shortened for a message.
fn clip(token: &str) -> String {
    token.chars().take(24).collect()
}

/// Reads a whole file.
pub fn read(bytes: &[u8]) -> Result<Brep, BrepError> {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return bad(
            "not a text BRep file: the bytes are not UTF-8 text (a binary BRep file is not read)",
        );
    };
    let mut cursor = Cursor {
        text,
        at: 0,
        context: "the header".to_string(),
    };
    let version = loop {
        let Some(line) = cursor.line() else {
            return bad("not a BRep file: no `CASCADE Topology V…` header");
        };
        let line = line.trim();
        // Open CASCADE's own exporter writes the kind of drawable first.
        if line.is_empty() || line == "DBRep_DrawableShape" {
            continue;
        }
        let Some(rest) = line.strip_prefix("CASCADE Topology V") else {
            return bad(format!(
                "not a BRep file: it starts with `{}`, not `CASCADE Topology V…`",
                clip(line)
            ));
        };
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        match digits.parse::<u32>() {
            Ok(v @ 1..=3) => break v,
            _ => {
                return bad(format!(
                    "BRep format version `{}` is not read (1 to 3 are)",
                    clip(rest)
                ));
            }
        }
    };
    let mut brep = Brep {
        version,
        locations: Vec::new(),
        curves: Vec::new(),
        surfaces: Vec::new(),
        shapes: Vec::new(),
        root: None,
    };
    cursor.context = "Locations".to_string();
    if cursor.need("the Locations section")? != "Locations" {
        return cursor.fail("the first section is not Locations");
    }
    brep.locations = locations(&mut cursor)?;
    loop {
        let Some(name) = cursor.token() else {
            return bad("the file ends before its TShapes section");
        };
        cursor.context = name.chars().take(32).collect();
        match name {
            "Curves" => brep.curves = curves(&mut cursor)?,
            "Surfaces" => brep.surfaces = surfaces(&mut cursor)?,
            "TShapes" => {
                shapes(&mut cursor, &mut brep)?;
                break;
            }
            other if SECTIONS.contains(&other) => {
                cursor.line();
                cursor.skip_section();
            }
            other => {
                return bad(format!("`{}` where a section should begin", clip(other)));
            }
        }
    }
    Ok(brep)
}

fn locations(c: &mut Cursor<'_>) -> Result<Vec<Location>, BrepError> {
    let n = c.count("the number of locations", c.left() / 2)?;
    let mut out = reserve(n);
    for k in 1..=n {
        c.context = format!("location {k}");
        match c.integer("the kind of location")? {
            1 => {
                let mut m = [[0.0; 4]; 3];
                for row in &mut m {
                    for cell in row.iter_mut() {
                        *cell = c.real("a matrix entry")?;
                    }
                }
                out.push(Location::Elementary(m));
            }
            2 => {
                let mut parts = Vec::new();
                loop {
                    let number = c.count("a location number", k.saturating_sub(1))?;
                    if number == 0 {
                        break;
                    }
                    let power = c.integer("a power")?;
                    if power.abs() > MAX_POWER {
                        return c.fail(&format!("the power {power} is out of range"));
                    }
                    parts.push((number, power));
                }
                out.push(Location::Composite(parts));
            }
            other => return c.fail(&format!("location kind {other} is not 1 or 2")),
        }
    }
    Ok(out)
}

fn axes(c: &mut Cursor<'_>) -> Result<Axes, BrepError> {
    Ok(Axes {
        origin: c.xyz("an origin")?,
        n: c.xyz("an axis")?,
        x: c.xyz("an x direction")?,
        y: c.xyz("a y direction")?,
    })
}

fn positive(c: &Cursor<'_>, x: f64, what: &str) -> Result<f64, BrepError> {
    if x > 0.0 {
        Ok(x)
    } else {
        c.fail(&format!("{what} {x} is not positive"))
    }
}

fn degree(c: &mut Cursor<'_>) -> Result<usize, BrepError> {
    let d = c.count("a degree", MAX_DEGREE)?;
    if d == 0 {
        return c.fail("a degree of 0");
    }
    Ok(d)
}

/// Poles (with weights when rational): at most what the rest of the file
/// can hold.
fn poles(
    c: &mut Cursor<'_>,
    count: usize,
    rational: bool,
) -> Result<(Vec<Xyz>, Vec<f64>), BrepError> {
    let room = c.left() / 6;
    if count > room {
        return c.fail(&format!(
            "{count} poles, more than the rest of the file holds"
        ));
    }
    let mut points = reserve(count);
    let mut weights = reserve(if rational { count } else { 0 });
    for _ in 0..count {
        points.push(c.xyz("a pole")?);
        if rational {
            let w = c.real("a weight")?;
            weights.push(positive(c, w, "a weight")?);
        }
    }
    Ok((points, weights))
}

fn knots(c: &mut Cursor<'_>, count: usize) -> Result<(Vec<f64>, Vec<usize>), BrepError> {
    if count > c.left() / 4 || count < 2 {
        return c.fail(&format!("{count} knots"));
    }
    let mut knots = reserve(count);
    let mut mults = reserve(count);
    for _ in 0..count {
        let k = c.real("a knot")?;
        if knots.last().is_some_and(|&last| k <= last) {
            return c.fail("the knots do not increase");
        }
        knots.push(k);
        let m = c.count("a multiplicity", MAX_DEGREE + 1)?;
        if m == 0 {
            return c.fail("a multiplicity of 0");
        }
        mults.push(m);
    }
    Ok((knots, mults))
}

/// Whether a B-spline's multiplicities add up to its poles, checked here
/// so that nothing downstream expands knots that do not: `poles + degree
/// + 1` in all, or for a periodic one the poles of one period (all but
/// the last, which repeats the first), each at most `degree + 1`.
fn add_up(
    c: &Cursor<'_>,
    degree: usize,
    periodic: bool,
    poles: usize,
    mults: &[usize],
    which: &str,
) -> Result<(), BrepError> {
    if mults.iter().any(|&m| m > degree + 1) {
        return c.fail(&format!("a {which}multiplicity above the degree + 1"));
    }
    let (sum, want) = if periodic {
        let sum: usize = mults.iter().take(mults.len().saturating_sub(1)).sum();
        if mults.first() != mults.last() {
            return c.fail(&format!(
                "a periodic B-spline whose first and last {which}multiplicities differ"
            ));
        }
        (sum, poles)
    } else {
        (mults.iter().sum(), poles.saturating_add(degree + 1))
    };
    if sum != want || poles <= degree && !periodic || poles == 0 {
        return c.fail(&format!(
            "the {which}multiplicities add up to {sum}, where {poles} poles of degree {degree} need {want}"
        ));
    }
    Ok(())
}

fn curves(c: &mut Cursor<'_>) -> Result<Vec<Curve>, BrepError> {
    let n = c.count("the number of curves", c.left() / 2)?;
    let mut out = reserve(n);
    for k in 1..=n {
        c.context = format!("curve {k}");
        out.push(curve(c, 0)?);
    }
    Ok(out)
}

fn curve(c: &mut Cursor<'_>, depth: usize) -> Result<Curve, BrepError> {
    if depth > MAX_NESTING {
        return c.fail("trimmed and offset curves nested too deep");
    }
    Ok(match c.integer("the kind of curve")? {
        1 => Curve::Line {
            origin: c.xyz("a point")?,
            direction: c.xyz("a direction")?,
        },
        2 => {
            let axes = axes(c)?;
            let r = c.real("a radius")?;
            Curve::Circle {
                axes,
                radius: positive(c, r, "the radius")?,
            }
        }
        3 => {
            let axes = axes(c)?;
            let (a, b) = (c.real("a radius")?, c.real("a radius")?);
            Curve::Ellipse {
                axes,
                major: positive(c, a, "the major radius")?,
                minor: positive(c, b, "the minor radius")?,
            }
        }
        4 => {
            let axes = axes(c)?;
            Curve::Parabola {
                axes,
                focal: c.real("a focal length")?,
            }
        }
        5 => {
            let axes = axes(c)?;
            Curve::Hyperbola {
                axes,
                major: c.real("a radius")?,
                minor: c.real("a radius")?,
            }
        }
        6 => {
            let rational = c.flag("the rational flag")?;
            let degree = degree(c)?;
            let (poles, weights) = poles(c, degree + 1, rational)?;
            Curve::Bezier { poles, weights }
        }
        7 => {
            let rational = c.flag("the rational flag")?;
            let periodic = c.flag("the periodic flag")?;
            let degree = degree(c)?;
            let count = c.count("the number of poles", c.left() / 6)?;
            let nknots = c.count("the number of knots", c.left() / 4)?;
            let (poles, weights) = poles(c, count, rational)?;
            let (knots, mults) = knots(c, nknots)?;
            add_up(c, degree, periodic, poles.len(), &mults, "")?;
            Curve::BSpline(BSplineCurve {
                degree,
                periodic,
                poles,
                weights,
                knots,
                mults,
            })
        }
        8 => {
            let first = c.real("a parameter")?;
            let last = c.real("a parameter")?;
            Curve::Trimmed {
                first,
                last,
                basis: Box::new(curve(c, depth + 1)?),
            }
        }
        9 => {
            let distance = c.real("an offset")?;
            let direction = c.xyz("a direction")?;
            Curve::Offset {
                distance,
                direction,
                basis: Box::new(curve(c, depth + 1)?),
            }
        }
        other => return c.fail(&format!("curve kind {other} is not 1 to 9")),
    })
}

fn surfaces(c: &mut Cursor<'_>) -> Result<Vec<Surface>, BrepError> {
    let n = c.count("the number of surfaces", c.left() / 2)?;
    let mut out = reserve(n);
    for k in 1..=n {
        c.context = format!("surface {k}");
        out.push(surface(c, 0)?);
    }
    Ok(out)
}

fn grid(
    c: &mut Cursor<'_>,
    count_u: usize,
    count_v: usize,
    rational: bool,
) -> Result<(Vec<Xyz>, Vec<f64>), BrepError> {
    let Some(count) = count_u.checked_mul(count_v) else {
        return c.fail("too many poles");
    };
    poles(c, count, rational)
}

fn surface(c: &mut Cursor<'_>, depth: usize) -> Result<Surface, BrepError> {
    if depth > MAX_NESTING {
        return c.fail("trimmed and offset surfaces nested too deep");
    }
    Ok(match c.integer("the kind of surface")? {
        1 => Surface::Plane { axes: axes(c)? },
        2 => {
            let axes = axes(c)?;
            let r = c.real("a radius")?;
            Surface::Cylinder {
                axes,
                radius: positive(c, r, "the radius")?,
            }
        }
        3 => {
            let axes = axes(c)?;
            let radius = c.real("a radius")?;
            let semi_angle = c.real("a semi-angle")?;
            if radius < 0.0 {
                return c.fail("a cone with a negative radius");
            }
            Surface::Cone {
                axes,
                radius,
                semi_angle,
            }
        }
        4 => {
            let axes = axes(c)?;
            let r = c.real("a radius")?;
            Surface::Sphere {
                axes,
                radius: positive(c, r, "the radius")?,
            }
        }
        5 => {
            let axes = axes(c)?;
            let (a, b) = (c.real("a radius")?, c.real("a radius")?);
            Surface::Torus {
                axes,
                major: positive(c, a, "the major radius")?,
                minor: positive(c, b, "the minor radius")?,
            }
        }
        6 => {
            let direction = c.xyz("a direction")?;
            Surface::Extrusion {
                direction,
                basis: curve(c, depth + 1)?,
            }
        }
        7 => {
            let origin = c.xyz("a point")?;
            let direction = c.xyz("a direction")?;
            Surface::Revolution {
                origin,
                direction,
                basis: curve(c, depth + 1)?,
            }
        }
        8 => {
            let ru = c.flag("the u rational flag")?;
            let rv = c.flag("the v rational flag")?;
            let du = degree(c)?;
            let dv = degree(c)?;
            let (poles, weights) = grid(c, du + 1, dv + 1, ru || rv)?;
            Surface::Bezier {
                count_u: du + 1,
                count_v: dv + 1,
                poles,
                weights,
            }
        }
        9 => {
            let ru = c.flag("the u rational flag")?;
            let rv = c.flag("the v rational flag")?;
            let pu = c.flag("the u periodic flag")?;
            let pv = c.flag("the v periodic flag")?;
            let du = degree(c)?;
            let dv = degree(c)?;
            let count_u = c.count("the number of u poles", c.left() / 6)?;
            let count_v = c.count("the number of v poles", c.left() / 6)?;
            let nku = c.count("the number of u knots", c.left() / 4)?;
            let nkv = c.count("the number of v knots", c.left() / 4)?;
            let (poles, weights) = grid(c, count_u, count_v, ru || rv)?;
            let (knots_u, mults_u) = knots(c, nku)?;
            let (knots_v, mults_v) = knots(c, nkv)?;
            add_up(c, du, pu, count_u, &mults_u, "u ")?;
            add_up(c, dv, pv, count_v, &mults_v, "v ")?;
            Surface::BSpline(Box::new(BSplineSurface {
                degree_u: du,
                degree_v: dv,
                periodic_u: pu,
                periodic_v: pv,
                count_u,
                count_v,
                poles,
                weights,
                knots_u,
                mults_u,
                knots_v,
                mults_v,
            }))
        }
        10 => {
            let u = (c.real("a parameter")?, c.real("a parameter")?);
            let v = (c.real("a parameter")?, c.real("a parameter")?);
            Surface::Trimmed {
                u,
                v,
                basis: Box::new(surface(c, depth + 1)?),
            }
        }
        11 => {
            let distance = c.real("an offset")?;
            Surface::Offset {
                distance,
                basis: Box::new(surface(c, depth + 1)?),
            }
        }
        other => return c.fail(&format!("surface kind {other} is not 1 to 11")),
    })
}

/// A flags token: seven `0`s and `1`s.
fn is_flags(token: &str) -> bool {
    token.len() == 7 && token.bytes().all(|b| b == b'0' || b == b'1')
}

fn shapes(c: &mut Cursor<'_>, brep: &mut Brep) -> Result<(), BrepError> {
    let n = c.count("the number of shapes", c.left() / 4)?;
    let mut out: Vec<TShape> = reserve(n);
    for written in 0..n {
        // The number the file gives this shape: n for the first written.
        let number = n - written;
        c.context = format!("shape {number}");
        let kind = match c.need("the kind of shape")? {
            "Ve" => {
                let tolerance = c.real("a tolerance")?;
                let point = c.xyz("a point")?;
                // Point representations, one per line, end with `0 0`.
                c.line();
                loop {
                    let Some(line) = c.line() else {
                        return c.fail("the file ends in a vertex");
                    };
                    let mut words = line.split_ascii_whitespace();
                    if words.next() == Some("0")
                        && words.next() == Some("0")
                        && words.next().is_none()
                    {
                        break;
                    }
                }
                Kind::Vertex { tolerance, point }
            }
            "Ed" => {
                let tolerance = c.real("a tolerance")?;
                let _same_parameter = c.flag("the same-parameter flag")?;
                let _same_range = c.flag("the same-range flag")?;
                let degenerated = c.flag("the degenerated flag")?;
                c.line();
                // Curve representations, one per line, end with `0`.
                let mut curve = None;
                loop {
                    let Some(line) = c.line() else {
                        return c.fail("the file ends in an edge");
                    };
                    let words: Vec<&str> = line.split_ascii_whitespace().take(6).collect();
                    match words.as_slice() {
                        ["0"] => break,
                        ["1", number, location, first, last, ..] if curve.is_none() => {
                            let read = |t: &str| t.parse::<usize>().ok();
                            let real = |t: &str| t.parse::<f64>().ok().filter(|x| x.is_finite());
                            let (Some(number), Some(location), Some(first), Some(last)) =
                                (read(number), read(location), real(first), real(last))
                            else {
                                return c.fail("an edge's 3D curve is not four numbers");
                            };
                            curve = Some(EdgeCurve {
                                curve: number,
                                location,
                                first,
                                last,
                            });
                        }
                        _ => {}
                    }
                }
                Kind::Edge {
                    tolerance,
                    degenerated,
                    curve,
                }
            }
            "Wi" => Kind::Wire,
            "Fa" => {
                let _natural = c.flag("the natural-restriction flag")?;
                let tolerance = c.real("a tolerance")?;
                let surface = c.count("a surface number", brep.surfaces.len())?;
                let location = c.count("a location number", brep.locations.len())?;
                Kind::Face {
                    tolerance,
                    surface,
                    location,
                }
            }
            "Sh" => Kind::Shell,
            "So" => Kind::Solid,
            "CS" => Kind::CompSolid,
            "Co" => Kind::Compound,
            other => return c.fail(&format!("`{}` is no kind of shape", clip(other))),
        };
        // Anything else before the flags (a face's triangulation) is
        // skipped; the flags come within a few tokens.
        let mut flags = None;
        for _ in 0..8 {
            let token = c.need("the shape's flags")?;
            if is_flags(token) {
                flags = Some(token);
                break;
            }
        }
        let Some(flags) = flags else {
            return c.fail("no flags where they should be");
        };
        let closed = flags.as_bytes().get(4) == Some(&b'1');
        let mut children = Vec::new();
        loop {
            let token = c.need("a sub-shape or `*`")?;
            if token == "*" {
                break;
            }
            let sub = sub(c, token, number, n, brep.locations.len())?;
            children.push(sub);
        }
        out.push(TShape {
            kind,
            closed,
            children,
        });
    }
    c.context = "the root shape".to_string();
    brep.shapes = out;
    brep.root = match c.token() {
        None => None,
        Some(token) => Some(sub(c, token, 0, n, brep.locations.len())?),
    };
    Ok(())
}

/// A sub-shape `+9 0`: `token` the orientation and number, then the
/// location. Shape `number` may only use shapes written before it, that
/// is numbers above its own.
fn sub(
    c: &mut Cursor<'_>,
    token: &str,
    number: usize,
    count: usize,
    locations: usize,
) -> Result<Sub, BrepError> {
    let mut chars = token.chars();
    let orientation = match chars.next() {
        Some('+') => Orientation::Forward,
        Some('-') => Orientation::Reversed,
        Some('i') => Orientation::Internal,
        Some('e') => Orientation::External,
        _ => return c.fail(&format!("`{}` is not a sub-shape", clip(token))),
    };
    let shape = match chars.as_str().parse::<usize>() {
        Ok(k) if k > number && k <= count => k,
        _ => {
            return c.fail(&format!(
                "sub-shape `{}` is not a shape written before this one",
                clip(token)
            ));
        }
    };
    let location = c.count("a location number", locations)?;
    Ok(Sub {
        orientation,
        shape,
        location,
    })
}

#[cfg(test)]
mod tests;
