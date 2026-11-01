//! A BRep shape (`craft3d-brep`) as `craft3d-topo` bodies: its solids,
//! and its shells and faces outside any solid as one sheet.
//!
//! **Placements.** Every use of a shape carries a location; a sub-shape's
//! place is its holder's place composed with its own location, and the
//! object's container placements (an `App::Part`) and the unit's scale
//! go first. Only rigid locations are read (a location that scales or
//! mirrors is refused).
//!
//! **Orientation.** A face's material normal is its surface's
//! `∂u × ∂v` (the format's parametrisation) when the face is used forward
//! and the opposite when reversed, the orientations of every holder
//! composed; its wires' edges, their orientations composed through the
//! wire and the face, run with the face on their left seen from that
//! normal, which is this kernel's rule for a loop. An edge runs from its
//! forward vertex at the start of its range to its reversed vertex at the
//! end. The elementary surfaces are built on the frame of their origin,
//! axis and x direction; where the file's frame is left-handed
//! (`x × y` against the axis) the file's normal is the opposite of ours
//! and the face is turned. Curves are built on the frame of their x and y
//! directions, so a circle's angle is the file's parameter and every
//! edge's range is the file's.
//!
//! **Degenerate edges** (a sphere's or a revolved profile's pole, a
//! cone's apex) are dropped from their loops: here a face needs no edge
//! there. A wire's edges are put in order by their vertices (a wire need
//! not list them in order), all of one wire in one loop where they join
//! up (a seam is crossed twice).

// `!(x > 0.0)` is deliberate: unlike `x <= 0.0` it is also true for NaN.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use crate::{Budget, err};
use craft3d_brep::{self as brep, Brep, Kind, Matrix, Orientation, Sub};
use craft3d_geom::{
    Circle, Cone, Curve3, Cylinder, Ellipse, Line, NurbsCurve, NurbsSurface, Plane, Sphere,
    Surface, Torus,
};
use craft3d_math::{Frame, Vec3};
use craft3d_stepread::ImportError;
use craft3d_stepread::spindle::{Normal, Spindle};
use craft3d_stepread::swept;
use craft3d_topo::{Body, BodyKind, Coedge, EdgeId, LoopId, VertexId};
use std::collections::BTreeMap;
use std::sync::Arc;

/// A use of a shape where it ends up: its number, its place (the scale
/// not yet applied) and its orientation, all holders composed.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Use {
    pub number: usize,
    pub m: Matrix,
    pub orientation: Orientation,
}

/// The key of a use for numbering: the shape and its place, bit for bit.
type Key = (usize, [u64; 12]);

fn key(u: &Use) -> Key {
    let mut bits = [0u64; 12];
    for (slot, x) in bits.iter_mut().zip(u.m.iter().flatten()) {
        *slot = x.to_bits();
    }
    (u.number, bits)
}

/// A shape read: the file, its locations multiplied out, and where the
/// object puts it.
pub(crate) struct Shape {
    pub brep: Arc<Brep>,
    matrices: Vec<Option<Matrix>>,
    /// The object's placement outside the shape (its containers').
    pub outer: Matrix,
    pub root: Option<Sub>,
}

/// What one object's shape becomes: its solids, and its loose shells and
/// faces as one sheet.
#[derive(Clone, Debug, Default)]
pub(crate) struct Parts {
    pub solids: Vec<Use>,
    /// Loose shells (and faces, each a shell of one face).
    pub sheet: Vec<SheetPart>,
}

#[derive(Clone, Debug)]
pub(crate) enum SheetPart {
    Shell(Use),
    Face(Use),
}

/// Every use reached from the root, counted against a bound: shared
/// sub-shapes placed many times in nested compounds multiply.
pub const MAX_USES: usize = 2_000_000;

impl Shape {
    pub fn new(brep: Arc<Brep>, outer: Matrix) -> Shape {
        let matrices = brep.matrices();
        let root = brep.root;
        Shape {
            brep,
            matrices,
            outer,
            root,
        }
    }

    fn matrix(&self, number: usize) -> Result<Matrix, ImportError> {
        if number == 0 {
            return Ok(brep::IDENTITY);
        }
        self.matrices
            .get(number - 1)
            .copied()
            .flatten()
            .ok_or_else(|| ImportError(format!("location {number} cannot be computed")))
    }

    fn tshape(&self, number: usize) -> Result<&brep::TShape, ImportError> {
        self.brep
            .shape(number)
            .ok_or_else(|| ImportError(format!("shape {number} does not exist")))
    }

    /// The uses of a use's sub-shapes.
    pub fn children(&self, u: &Use) -> Result<Vec<Use>, ImportError> {
        let t = self.tshape(u.number)?;
        let mut out = Vec::with_capacity(t.children.len());
        for sub in &t.children {
            let m = brep::multiply(&u.m, &self.matrix(sub.location)?);
            out.push(Use {
                number: sub.shape,
                m,
                orientation: sub.orientation.compose(u.orientation),
            });
        }
        Ok(out)
    }

    pub fn root_use(&self) -> Result<Option<Use>, ImportError> {
        let Some(root) = self.root else {
            return Ok(None);
        };
        Ok(Some(Use {
            number: root.shape,
            m: brep::multiply(&self.outer, &self.matrix(root.location)?),
            orientation: root.orientation,
        }))
    }

    /// The solids, and the shells and faces in no solid, of the shape.
    pub fn parts(&self, walks: &mut usize) -> Result<Parts, ImportError> {
        let mut parts = Parts::default();
        let Some(root) = self.root_use()? else {
            return Ok(parts);
        };
        let mut stack = vec![root];
        let mut seen = 0usize;
        while let Some(u) = stack.pop() {
            seen += 1;
            *walks = walks.checked_sub(1).ok_or_else(|| {
                ImportError("the document's shapes place their parts too many times".to_string())
            })?;
            if seen > MAX_USES {
                return err(format!(
                    "the shape places its parts more than {MAX_USES} times"
                ));
            }
            match self.tshape(u.number)?.kind {
                Kind::Compound | Kind::CompSolid => {
                    let mut c = self.children(&u)?;
                    c.reverse();
                    stack.extend(c);
                }
                Kind::Solid => parts.solids.push(u),
                Kind::Shell => parts.sheet.push(SheetPart::Shell(u)),
                Kind::Face { .. } => parts.sheet.push(SheetPart::Face(u)),
                Kind::Wire | Kind::Edge { .. } | Kind::Vertex { .. } => {}
            }
        }
        Ok(parts)
    }

    /// FreeCAD's numbers of the faces, edges and vertices of the whole
    /// shape (`Face3` is 3): each distinct use in the order a depth-first
    /// walk meets it, orientation aside.
    pub fn numbering(&self, walks: &mut usize) -> Result<Numbering, ImportError> {
        let mut n = Numbering::default();
        let Some(root) = self.root_use()? else {
            return Ok(n);
        };
        let mut stack = vec![root];
        let mut seen = 0usize;
        while let Some(u) = stack.pop() {
            seen += 1;
            *walks = walks.checked_sub(1).ok_or_else(|| {
                ImportError("the document's shapes place their parts too many times".to_string())
            })?;
            if seen > MAX_USES {
                return err(format!(
                    "the shape places its parts more than {MAX_USES} times"
                ));
            }
            let map = match self.tshape(u.number)?.kind {
                Kind::Face { .. } => Some(&mut n.faces),
                Kind::Edge { .. } => Some(&mut n.edges),
                Kind::Vertex { .. } => Some(&mut n.vertices),
                _ => None,
            };
            if let Some(map) = map {
                let next = map.len() as u64 + 1;
                let k = key(&u);
                if map.contains_key(&k) {
                    continue;
                }
                map.insert(k, next);
            }
            let mut c = self.children(&u)?;
            c.reverse();
            stack.extend(c);
        }
        Ok(n)
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Numbering {
    faces: BTreeMap<Key, u64>,
    edges: BTreeMap<Key, u64>,
    vertices: BTreeMap<Key, u64>,
}

/// A rigid placement followed by a uniform scale (the unit).
#[derive(Clone, Copy)]
pub(crate) struct Place {
    m: Matrix,
    scale: f64,
}

impl Place {
    fn new(m: Matrix, scale: f64) -> Result<Place, ImportError> {
        // Rigid: orthonormal rows, determinant +1.
        let r = |i: usize| {
            let row = m.get(i).copied().unwrap_or([0.0; 4]);
            Vec3::new(row[0], row[1], row[2])
        };
        let (a, b, c) = (r(0), r(1), r(2));
        let ok = (a.length() - 1.0).abs() < 1e-9
            && (b.length() - 1.0).abs() < 1e-9
            && (c.length() - 1.0).abs() < 1e-9
            && a.dot(b).abs() < 1e-9
            && a.dot(c).abs() < 1e-9
            && b.dot(c).abs() < 1e-9
            && a.cross(b).dot(c) > 0.0;
        if !ok {
            return err("a location that scales or mirrors is not read");
        }
        Ok(Place { m, scale })
    }

    fn point(&self, p: brep::Xyz) -> Vec3 {
        let [x, y, z] = p;
        let row = |i: usize| self.m.get(i).copied().unwrap_or([0.0; 4]);
        let f = |r: [f64; 4]| (r[0] * x + r[1] * y + r[2] * z + r[3]) * self.scale;
        Vec3::new(f(row(0)), f(row(1)), f(row(2)))
    }

    fn direction(&self, d: brep::Xyz) -> Vec3 {
        let [x, y, z] = d;
        let row = |i: usize| self.m.get(i).copied().unwrap_or([0.0; 4]);
        let f = |r: [f64; 4]| r[0] * x + r[1] * y + r[2] * z;
        Vec3::new(f(row(0)), f(row(1)), f(row(2)))
    }
}

fn geom(e: craft3d_geom::GeomError) -> ImportError {
    ImportError(e.to_string())
}

/// The frame of a placement: at its origin, `z` along `z_of`, `x` along
/// its x direction.
fn frame(origin: Vec3, z: Vec3, x: Vec3) -> Result<Frame, ImportError> {
    Frame::new(origin, z, x).ok_or_else(|| ImportError("a degenerate placement".to_string()))
}

/// The flat knot vector of distinct knots with multiplicities.
fn flat(knots: &[f64], mults: &[usize]) -> Vec<f64> {
    let mut out = Vec::new();
    for (&k, &m) in knots.iter().zip(mults) {
        out.extend(std::iter::repeat_n(k, m));
    }
    out
}

/// A periodic B-spline's poles and knots as an unclamped one over
/// `periods` periods from its first knot (one for a surface or a closed
/// edge, so it closes on itself where the file's does; two for an edge
/// that crosses the first knot): the poles of one period are those the knots of the period
/// count (sum of the multiplicities but the last), repeated. Returns the
/// flat knots, the index into the file's poles of each pole, and the
/// period.
fn unperiodic(
    degree: usize,
    count: usize,
    knots: &[f64],
    mults: &[usize],
    periods: usize,
) -> Result<(Vec<f64>, Vec<usize>, f64), ImportError> {
    let (Some(&first), Some(&last)) = (knots.first(), knots.last()) else {
        return err("a periodic B-spline without knots");
    };
    let period = last - first;
    let inner: usize = mults.iter().take(mults.len().saturating_sub(1)).sum();
    if inner != count || count == 0 || !(period > 0.0) {
        return err(format!(
            "a periodic B-spline whose knots count {inner} poles for its {count}"
        ));
    }
    // The flat knots of one period, from the first knot.
    let period_knots = flat(
        knots.get(..knots.len() - 1).unwrap_or(&[]),
        mults.get(..mults.len() - 1).unwrap_or(&[]),
    );
    let n = period_knots.len();
    let at = |j: i64| -> f64 {
        let n = n as i64;
        let wraps = j.div_euclid(n);
        let k = j.rem_euclid(n) as usize;
        period_knots.get(k).copied().unwrap_or(0.0) + wraps as f64 * period
    };
    let p = degree as i64;
    let poles = periods * n + degree;
    let flat_knots: Vec<f64> = (0..(poles + degree + 1) as i64)
        .map(|j| at(j - p))
        .collect();
    // Which of the file's poles goes with the basis function that starts
    // at knot `j`: the format's periodic knots put the first knot's
    // multiplicity `m` at the end of the first `degree + 1` places, so the
    // first pole's function starts `degree + 1 − m` knots before it
    // (checked against FreeCAD's evaluation of a periodic curve).
    let m1 = mults.first().copied().unwrap_or(1) as i64;
    let index: Vec<usize> = (0..poles as i64)
        .map(|j| (j + 1 - m1).rem_euclid(n as i64) as usize)
        .collect();
    Ok((flat_knots, index, period))
}

/// Inserts knot `u` once into `knots` and every net in `nets` (each the
/// homogeneous control points `(w x, w y, w z, w)` along the knots), by
/// Boehm's algorithm (*The NURBS Book* §5.2): the curve stays the same.
fn insert_knot(degree: usize, knots: &mut Vec<f64>, nets: &mut [Vec<[f64; 4]>], u: f64) {
    let p = degree;
    // The span: the last knot at or below `u` (an unclamped vector has
    // knots past both ends of its domain).
    let Some(k) = knots.iter().rposition(|&t| t <= u) else {
        return;
    };
    if k < p || k + 1 >= knots.len() {
        return;
    }
    let s = knots.iter().filter(|&&t| t == u).count();
    for net in nets.iter_mut() {
        let mut out = Vec::with_capacity(net.len() + 1);
        for i in 0..=net.len() {
            let q = if i + p <= k {
                net.get(i).copied()
            } else if i + s <= k {
                let (Some(ti), Some(tip)) = (knots.get(i), knots.get(i + p)) else {
                    return;
                };
                let a = if tip > ti { (u - ti) / (tip - ti) } else { 0.0 };
                match (net.get(i), i.checked_sub(1).and_then(|j| net.get(j))) {
                    (Some(pi), Some(pj)) => Some([
                        a * pi[0] + (1.0 - a) * pj[0],
                        a * pi[1] + (1.0 - a) * pj[1],
                        a * pi[2] + (1.0 - a) * pj[2],
                        a * pi[3] + (1.0 - a) * pj[3],
                    ]),
                    _ => None,
                }
            } else {
                i.checked_sub(1).and_then(|j| net.get(j)).copied()
            };
            if let Some(q) = q {
                out.push(q);
            }
        }
        *net = out;
    }
    knots.insert(k + 1, u);
}

/// An unclamped B-spline (knots and nets as for [`insert_knot`]) made
/// clamped over its own domain `[t[p], t[n]]`, the same curve there: the
/// ends' knots raised to the degree, the poles outside dropped.
fn clamp(degree: usize, knots: &mut Vec<f64>, nets: &mut [Vec<[f64; 4]>]) -> Option<()> {
    let p = degree;
    let a = *knots.get(p)?;
    let n = knots.len().checked_sub(p + 1)?;
    let b = *knots.get(n)?;
    for end in [a, b] {
        for _ in 0..p {
            let mult = knots.iter().filter(|&&t| t == end).count();
            if mult >= p {
                break;
            }
            insert_knot(p, knots, nets, end);
        }
    }
    let j = knots.iter().position(|&t| t == a)?;
    let l = knots.iter().rposition(|&t| t == b)?;
    let first = j.checked_sub(1)?;
    let last = l.checked_sub(p)?;
    let mut clamped = vec![a];
    clamped.extend(knots.get(j..=l)?.iter().copied());
    clamped.push(b);
    for net in nets.iter_mut() {
        *net = net.get(first..=last)?.to_vec();
    }
    *knots = clamped;
    Some(())
}

/// Homogeneous points from points and weights, and back.
fn homogeneous(points: &[Vec3], weights: &[f64]) -> Vec<[f64; 4]> {
    points
        .iter()
        .zip(weights)
        .map(|(p, &w)| [p.x * w, p.y * w, p.z * w, w])
        .collect()
}

fn cartesian(net: &[[f64; 4]]) -> (Vec<Vec3>, Vec<f64>) {
    net.iter()
        .map(|h| (Vec3::new(h[0] / h[3], h[1] / h[3], h[2] / h[3]), h[3]))
        .unzip()
}

/// The curve and the range of an edge.
fn curve(
    c: &brep::Curve,
    place: &Place,
    range: (f64, f64),
    depth: usize,
) -> Result<(Curve3, (f64, f64)), ImportError> {
    if depth > brep::MAX_NESTING {
        return err("curves nested too deep");
    }
    let s = place.scale;
    Ok(match c {
        brep::Curve::Line { origin, direction } => {
            let line =
                Line::new(place.point(*origin), place.direction(*direction)).map_err(geom)?;
            // The parameter is the distance from the origin, scaled.
            let len = Vec3::new(direction[0], direction[1], direction[2]).length();
            if !(len > 0.0) {
                return err("a line without a direction");
            }
            (Curve3::Line(line), (range.0 * s * 1.0, range.1 * s * 1.0))
        }
        brep::Curve::Circle { axes, radius } => {
            let x = place.direction(axes.x);
            let y = place.direction(axes.y);
            let f = frame(place.point(axes.origin), x.cross(y), x)?;
            (
                Curve3::Circle(Circle::new(f, radius * s).map_err(geom)?),
                range,
            )
        }
        brep::Curve::Ellipse { axes, major, minor } => {
            let x = place.direction(axes.x);
            let y = place.direction(axes.y);
            let f = frame(place.point(axes.origin), x.cross(y), x)?;
            (
                Curve3::Ellipse(Ellipse::new(f, major * s, minor * s).map_err(geom)?),
                range,
            )
        }
        brep::Curve::Bezier { poles, weights } => {
            let degree = poles.len().saturating_sub(1);
            let mut knots = vec![0.0; degree + 1];
            knots.extend(vec![1.0; degree + 1]);
            let ctrl = poles.iter().map(|p| place.point(*p)).collect();
            let w = if weights.is_empty() {
                vec![1.0; poles.len()]
            } else {
                weights.clone()
            };
            (
                Curve3::Nurbs(NurbsCurve::new(degree, knots, ctrl, w).map_err(geom)?),
                range,
            )
        }
        brep::Curve::BSpline(b) => {
            let w_of = |i: usize| {
                if b.weights.is_empty() {
                    1.0
                } else {
                    b.weights.get(i).copied().unwrap_or(1.0)
                }
            };
            if b.periodic {
                let first = b.knots.first().copied().unwrap_or(0.0);
                let last = b.knots.last().copied().unwrap_or(0.0);
                let period = last - first;
                if !(period > 0.0) {
                    return err("a periodic B-spline without a period");
                }
                // Shift the range by whole periods to start in the first.
                let shift = ((range.0 - first) / period).floor() * period;
                let range = (range.0 - shift, range.1 - shift);
                let periods = if range.1 <= last + 1e-9 * period {
                    1
                } else {
                    2
                };
                if range.1 > first + 2.0 * period + 1e-9 * period {
                    return err("an edge longer than its periodic curve");
                }
                let (knots, index, _) =
                    unperiodic(b.degree, b.poles.len(), &b.knots, &b.mults, periods)?;
                let ctrl = index
                    .iter()
                    .map(|&i| b.poles.get(i).map(|p| place.point(*p)))
                    .collect::<Option<Vec<_>>>()
                    .ok_or_else(|| ImportError("a pole is missing".to_string()))?;
                let w: Vec<f64> = index.iter().map(|&i| w_of(i)).collect();
                let mut knots = knots;
                let mut nets = vec![homogeneous(&ctrl, &w)];
                clamp(b.degree, &mut knots, &mut nets).ok_or_else(|| {
                    ImportError("a periodic B-spline cannot be clamped".to_string())
                })?;
                let (ctrl, w) = cartesian(nets.first().map_or(&[][..], Vec::as_slice));
                (
                    Curve3::Nurbs(NurbsCurve::new(b.degree, knots, ctrl, w).map_err(geom)?),
                    range,
                )
            } else {
                let knots = flat(&b.knots, &b.mults);
                let ctrl = b.poles.iter().map(|p| place.point(*p)).collect();
                let w = (0..b.poles.len()).map(w_of).collect();
                (
                    Curve3::Nurbs(NurbsCurve::new(b.degree, knots, ctrl, w).map_err(geom)?),
                    range,
                )
            }
        }
        brep::Curve::Trimmed { basis, .. } => curve(basis, place, range, depth + 1)?,
        brep::Curve::Parabola { .. } => return err("a parabola is not read"),
        brep::Curve::Hyperbola { .. } => return err("a hyperbola is not read"),
        brep::Curve::Offset { .. } => return err("an offset curve is not read"),
    })
}

/// A curve on its own, for a swept surface (its whole domain).
fn basis_curve(c: &brep::Curve, place: &Place) -> Result<Curve3, ImportError> {
    // A periodic B-spline over exactly one period, so the surface swept
    // from it closes on itself.
    let mut basis = c;
    for _ in 0..=brep::MAX_NESTING {
        match basis {
            brep::Curve::Trimmed { basis: b, .. } => basis = b,
            _ => break,
        }
    }
    let range = match basis {
        brep::Curve::BSpline(b) if b.periodic => (
            b.knots.first().copied().unwrap_or(0.0),
            b.knots.last().copied().unwrap_or(1.0),
        ),
        _ => (0.0, 1.0),
    };
    curve(c, place, range, 0).map(|(c, _)| c)
}

/// A surface read: done, with whether the file's normal is opposite to
/// ours; an extrusion that waits for its face's edges; or a horn or
/// spindle torus whose part its face's edges choose (`stepread::swept`).
use craft3d_stepread::swept::Swept as Read;

fn surface(s: &brep::Surface, place: &Place, depth: usize) -> Result<Read, ImportError> {
    if depth > brep::MAX_NESTING {
        return err("surfaces nested too deep");
    }
    let k = place.scale;
    // An elementary surface on the frame of its axis and x direction;
    // the file's normal is ours unless its frame is left-handed.
    let elementary = |axes: &brep::Axes| -> Result<(Frame, bool), ImportError> {
        let n = place.direction(axes.n);
        let x = place.direction(axes.x);
        let y = place.direction(axes.y);
        let f = frame(place.point(axes.origin), n, x)?;
        Ok((f, x.cross(y).dot(n) < 0.0))
    };
    Ok(match s {
        brep::Surface::Plane { axes } => {
            let (f, flip) = elementary(axes)?;
            Read::Done(Surface::Plane(Plane::new(f)), flip)
        }
        brep::Surface::Cylinder { axes, radius } => {
            let (f, flip) = elementary(axes)?;
            Read::Done(
                Surface::Cylinder(Cylinder::new(f, radius * k).map_err(geom)?),
                flip,
            )
        }
        brep::Surface::Cone {
            axes,
            radius,
            semi_angle,
        } => {
            let (f, flip) = elementary(axes)?;
            Read::Done(
                Surface::Cone(Cone::new(f, radius * k, *semi_angle).map_err(geom)?),
                flip,
            )
        }
        brep::Surface::Sphere { axes, radius } => {
            let (f, flip) = elementary(axes)?;
            Read::Done(
                Surface::Sphere(Sphere::new(f, radius * k).map_err(geom)?),
                flip,
            )
        }
        brep::Surface::Torus { axes, major, minor } => {
            let (f, flip) = elementary(axes)?;
            if major <= minor {
                // A horn or spindle torus: its normal is `∂u × ∂v` of the
                // format's torus, the standard one's on a right-handed
                // frame (the same code as a STEP revolution's).
                return Spindle::toroidal(f, major * k, minor * k, None, Normal::Swept { flip })
                    .and_then(Spindle::read)
                    .map_err(ImportError);
            }
            Read::Done(
                Surface::Torus(Torus::new(f, major * k, minor * k).map_err(geom)?),
                flip,
            )
        }
        brep::Surface::Extrusion { direction, basis } => {
            let c = basis_curve(basis, place)?;
            let along = place
                .direction(*direction)
                .normalized()
                .ok_or_else(|| ImportError("an extrusion without a direction".to_string()))?;
            swept::extrusion(&c, along).map_err(ImportError)?
        }
        brep::Surface::Revolution {
            origin,
            direction,
            basis,
        } => {
            let c = basis_curve(basis, place)?;
            let axis = place
                .direction(*direction)
                .normalized()
                .ok_or_else(|| ImportError("a revolution without an axis".to_string()))?;
            swept::revolution(&c, place.point(*origin), axis).map_err(ImportError)?
        }
        brep::Surface::Bezier {
            count_u,
            count_v,
            poles,
            weights,
        } => {
            let (du, dv) = (count_u.saturating_sub(1), count_v.saturating_sub(1));
            let clamped = |d: usize| {
                let mut k = vec![0.0; d + 1];
                k.extend(vec![1.0; d + 1]);
                k
            };
            let ctrl = poles.iter().map(|p| place.point(*p)).collect();
            let w = if weights.is_empty() {
                vec![1.0; poles.len()]
            } else {
                weights.clone()
            };
            Read::Done(
                Surface::Nurbs(
                    NurbsSurface::new(
                        du,
                        dv,
                        clamped(du),
                        clamped(dv),
                        *count_u,
                        *count_v,
                        ctrl,
                        w,
                    )
                    .map_err(geom)?,
                ),
                false,
            )
        }
        brep::Surface::BSpline(b) => {
            let (mut knots_u, index_u) = if b.periodic_u {
                let (k, i, _) = unperiodic(b.degree_u, b.count_u, &b.knots_u, &b.mults_u, 1)?;
                (k, i)
            } else {
                (flat(&b.knots_u, &b.mults_u), (0..b.count_u).collect())
            };
            let (mut knots_v, index_v) = if b.periodic_v {
                let (k, i, _) = unperiodic(b.degree_v, b.count_v, &b.knots_v, &b.mults_v, 1)?;
                (k, i)
            } else {
                (flat(&b.knots_v, &b.mults_v), (0..b.count_v).collect())
            };
            // Rows along v, one per u.
            let mut rows: Vec<Vec<[f64; 4]>> = Vec::with_capacity(index_u.len());
            for &i in &index_u {
                let mut row = Vec::with_capacity(index_v.len());
                for &j in &index_v {
                    let at = i * b.count_v + j;
                    let p = b
                        .poles
                        .get(at)
                        .ok_or_else(|| ImportError("a pole is missing".to_string()))?;
                    let q = place.point(*p);
                    let w = if b.weights.is_empty() {
                        1.0
                    } else {
                        b.weights.get(at).copied().unwrap_or(1.0)
                    };
                    row.push([q.x * w, q.y * w, q.z * w, w]);
                }
                rows.push(row);
            }
            let unclamped =
                || ImportError("a periodic B-spline surface cannot be clamped".to_string());
            if b.periodic_v {
                let mut k = knots_v.clone();
                for row in rows.iter_mut() {
                    k = knots_v.clone();
                    let mut one = vec![std::mem::take(row)];
                    clamp(b.degree_v, &mut k, &mut one).ok_or_else(unclamped)?;
                    *row = one.pop().unwrap_or_default();
                }
                knots_v = k;
            }
            if b.periodic_u {
                // Columns along u, one per v.
                let count_v = rows.first().map_or(0, Vec::len);
                let mut columns: Vec<Vec<[f64; 4]>> = (0..count_v)
                    .map(|j| rows.iter().filter_map(|r| r.get(j).copied()).collect())
                    .collect();
                clamp(b.degree_u, &mut knots_u, &mut columns).ok_or_else(unclamped)?;
                let count_u = columns.first().map_or(0, Vec::len);
                rows = (0..count_u)
                    .map(|i| columns.iter().filter_map(|c| c.get(i).copied()).collect())
                    .collect();
            }
            let count_u = rows.len();
            let count_v = rows.first().map_or(0, Vec::len);
            let flat_net: Vec<[f64; 4]> = rows.into_iter().flatten().collect();
            let (ctrl, w) = cartesian(&flat_net);
            Read::Done(
                Surface::Nurbs(
                    NurbsSurface::new(
                        b.degree_u, b.degree_v, knots_u, knots_v, count_u, count_v, ctrl, w,
                    )
                    .map_err(geom)?,
                ),
                false,
            )
        }
        brep::Surface::Trimmed { basis, .. } => surface(basis, place, depth + 1)?,
        brep::Surface::Offset { distance, basis } => match surface(basis, place, depth + 1)? {
            Read::Done(s, flip) => swept::offset(&s, flip, distance * k).map_err(ImportError)?,
            Read::Spindle(spindle) => spindle.offset(distance * k).map_err(ImportError)?,
            Read::Unbounded { .. } => {
                return err("an offset of an extruded curve is not read");
            }
        },
    })
}

/// Builds one body.
pub(crate) struct Builder<'a> {
    shape: &'a Shape,
    numbering: &'a Numbering,
    scale: f64,
    merge: f64,
    pub body: Body,
    /// The vertices of each vertex shape, at the places it was used.
    vertices: BTreeMap<usize, Vec<(Vec3, VertexId)>>,
    edges: BTreeMap<(usize, u32, u32), EdgeId>,
    pub face_ids: Vec<u64>,
    pub edge_ids: Vec<u64>,
    pub vertex_ids: Vec<u64>,
    budget: &'a mut Budget,
}

impl<'a> Builder<'a> {
    pub fn new(
        shape: &'a Shape,
        numbering: &'a Numbering,
        kind: BodyKind,
        scale: f64,
        merge: f64,
        budget: &'a mut Budget,
    ) -> Self {
        Builder {
            shape,
            numbering,
            scale,
            merge,
            body: Body::new(kind),
            vertices: BTreeMap::new(),
            edges: BTreeMap::new(),
            face_ids: Vec::new(),
            edge_ids: Vec::new(),
            vertex_ids: Vec::new(),
            budget,
        }
    }

    fn place(&self, m: Matrix) -> Result<Place, ImportError> {
        Place::new(m, self.scale)
    }

    fn vertex(&mut self, u: &Use) -> Result<VertexId, ImportError> {
        let t = self.shape.tshape(u.number)?;
        let Kind::Vertex { point, .. } = t.kind else {
            return err(format!(
                "shape {} is a {}, not a vertex",
                u.number,
                t.kind.name()
            ));
        };
        let p = self.place(u.m)?.point(point);
        let list = self.vertices.entry(u.number).or_default();
        if let Some(&(_, v)) = list.iter().find(|(q, _)| q.distance(p) <= self.merge) {
            return Ok(v);
        }
        let v = self.body.add_vertex(p);
        list.push((p, v));
        self.vertex_ids
            .push(self.numbering.vertices.get(&key(u)).copied().unwrap_or(0));
        Ok(v)
    }

    /// The edge of a use (`None` for a degenerate one), with its start and
    /// end vertices as the edge runs.
    fn edge(&mut self, u: &Use) -> Result<Option<(EdgeId, VertexId, VertexId)>, ImportError> {
        let t = self.shape.tshape(u.number)?;
        let Kind::Edge {
            degenerated,
            curve: rep,
            ..
        } = t.kind.clone()
        else {
            return err(format!(
                "shape {} is a {}, not an edge",
                u.number,
                t.kind.name()
            ));
        };
        if degenerated {
            return Ok(None);
        }
        let mut start = None;
        let mut end = None;
        for child in &t.children {
            let cu = Use {
                number: child.shape,
                m: brep::multiply(&u.m, &self.shape.matrix(child.location)?),
                orientation: child.orientation,
            };
            match child.orientation {
                Orientation::Forward if start.is_none() => start = Some(self.vertex(&cu)?),
                Orientation::Reversed if end.is_none() => end = Some(self.vertex(&cu)?),
                _ => {}
            }
        }
        let (Some(a), Some(b)) = (start, end) else {
            return err(format!(
                "edge {} has no forward and reversed vertex (an infinite or open edge is not read)",
                u.number
            ));
        };
        let k = (u.number, a.0, b.0);
        if let Some(&e) = self.edges.get(&k) {
            return Ok(Some((e, a, b)));
        }
        let Some(rep) = rep else {
            return err(format!("edge {} has no 3D curve", u.number));
        };
        let c = self.shape.brep.curve(rep.curve).ok_or_else(|| {
            ImportError(format!(
                "edge {}: curve {} does not exist",
                u.number, rep.curve
            ))
        })?;
        self.budget.curve(c)?;
        let place = self.place(brep::multiply(&u.m, &self.shape.matrix(rep.location)?))?;
        let (c, mut range) = curve(c, &place, (rep.first, rep.last), 0)
            .map_err(|e| ImportError(format!("edge {}: {e}", u.number)))?;
        // A range a rounding outside a bounded curve's domain is its end.
        let (d0, d1) = c.domain();
        if c.period().is_none() && d0.is_finite() && d1.is_finite() {
            let slack = 1e-9 * (d1 - d0).abs().max(1.0);
            if range.0 < d0 && range.0 >= d0 - slack {
                range.0 = d0;
            }
            if range.1 > d1 && range.1 <= d1 + slack {
                range.1 = d1;
            }
            if range.0 < d0 || range.1 > d1 {
                return err(format!(
                    "edge {}: its range [{}, {}] leaves its curve's [{d0}, {d1}]",
                    u.number, range.0, range.1
                ));
            }
        }
        if !(range.1 > range.0) {
            return err(format!("edge {}: an empty range", u.number));
        }
        let e = self.body.add_edge(c, range, a, b);
        self.edges.insert(k, e);
        self.edge_ids
            .push(self.numbering.edges.get(&key(u)).copied().unwrap_or(0));
        Ok(Some((e, a, b)))
    }

    /// The loops of a wire use in a face used with `face`'s orientation.
    fn wire(&mut self, w: &Use) -> Result<Vec<Vec<Coedge>>, ImportError> {
        // (coedge, from, to)
        let mut uses: Vec<(Coedge, VertexId, VertexId)> = Vec::new();
        for e in self.shape.children(w)? {
            let reversed = match e.orientation {
                Orientation::Forward => false,
                Orientation::Reversed => true,
                // Edges inside or outside the face bound nothing.
                Orientation::Internal | Orientation::External => continue,
            };
            // The vertices of the edge are taken at the edge's own place,
            // its own orientation aside.
            let own = Use {
                orientation: Orientation::Forward,
                ..e
            };
            let Some((edge, a, b)) = self.edge(&own)? else {
                continue;
            };
            let (from, to) = if reversed { (b, a) } else { (a, b) };
            uses.push((Coedge { edge, reversed }, from, to));
        }
        chain(&uses).ok_or_else(|| ImportError(format!("wire {} does not close", w.number)))
    }

    /// Adds a face use; `None` when it has no loop left and its surface
    /// needs one.
    pub fn face(&mut self, f: &Use) -> Result<(), ImportError> {
        let t = self.shape.tshape(f.number)?;
        let Kind::Face {
            surface: number,
            location,
            ..
        } = t.kind
        else {
            return err(format!(
                "shape {} is a {}, not a face",
                f.number,
                t.kind.name()
            ));
        };
        let face_reversed = match f.orientation {
            Orientation::Forward => false,
            Orientation::Reversed => true,
            Orientation::Internal | Orientation::External => {
                return err(format!(
                    "face {} is used inside or outside its shell",
                    f.number
                ));
            }
        };
        let mut loops: Vec<LoopId> = Vec::new();
        for w in self.shape.children(f)? {
            if !matches!(self.shape.tshape(w.number)?.kind, Kind::Wire) {
                continue;
            }
            for coedges in self.wire(&w)? {
                loops.push(self.body.add_loop(coedges));
            }
        }
        let s = self
            .shape
            .brep
            .surface(number)
            .ok_or_else(|| ImportError(format!("face {}: it has no surface", f.number)))?;
        self.budget.surface(s)?;
        let place = self.place(brep::multiply(&f.m, &self.shape.matrix(location)?))?;
        let (surface, flip) = match surface(s, &place, 0)
            .map_err(|e| ImportError(format!("face {}: {e}", f.number)))?
        {
            Read::Done(s, flip) => (s, flip),
            Read::Unbounded { curve, along } => (
                swept::bounded_extrusion(&self.body, &curve, along, &loops)
                    .map_err(|e| ImportError(format!("face {}: {e}", f.number)))?,
                false,
            ),
            Read::Spindle(spindle) => swept::spindle_part(&self.body, &spindle, &loops, self.merge)
                .map_err(|e| ImportError(format!("face {}: {e}", f.number)))?,
        };
        let closed = match &surface {
            Surface::Sphere(_) | Surface::Torus(_) => true,
            other => other.period_u().is_some() && other.period_v().is_some(),
        };
        if loops.is_empty() && !closed {
            return err(format!(
                "face {}: a face without edges on a {}, which does not close all around",
                f.number,
                surface.kind()
            ));
        }
        let id = self.body.add_face(surface, face_reversed ^ flip, loops);
        let _ = id;
        self.face_ids
            .push(self.numbering.faces.get(&key(f)).copied().unwrap_or(0));
        Ok(())
    }

    /// Adds a shell use's faces as a shell.
    pub fn shell(&mut self, s: &Use) -> Result<(), ImportError> {
        let first = self.body.faces.len();
        for f in self.shape.children(s)? {
            if matches!(self.shape.tshape(f.number)?.kind, Kind::Face { .. }) {
                self.face(&f)?;
            }
        }
        self.close_shell(first)
    }

    /// Makes the faces added since `first` a shell.
    pub fn close_shell(&mut self, first: usize) -> Result<(), ImportError> {
        let faces: Vec<_> = (first..self.body.faces.len())
            .map(|k| craft3d_topo::FaceId(k as u32))
            .collect();
        if faces.is_empty() {
            return err("a shell without faces");
        }
        self.body.add_shell(faces);
        Ok(())
    }

    /// Drops the edges no loop uses and the vertices no edge uses (those
    /// only degenerate edges reached), keeping the order.
    pub fn compact(&mut self) {
        let mut used = vec![false; self.body.edges.len()];
        for l in &self.body.loops {
            for c in &l.coedges {
                if let Some(u) = used.get_mut(c.edge.0 as usize) {
                    *u = true;
                }
            }
        }
        let mut renumber = vec![u32::MAX; used.len()];
        let mut kept = Vec::new();
        let mut kept_ids = Vec::new();
        for (i, edge) in std::mem::take(&mut self.body.edges).into_iter().enumerate() {
            if used.get(i).copied().unwrap_or(false) {
                if let Some(r) = renumber.get_mut(i) {
                    *r = kept.len() as u32;
                }
                kept.push(edge);
                kept_ids.push(self.edge_ids.get(i).copied().unwrap_or(0));
            }
        }
        self.body.edges = kept;
        self.edge_ids = kept_ids;
        for l in &mut self.body.loops {
            for c in &mut l.coedges {
                if let Some(&n) = renumber.get(c.edge.0 as usize) {
                    c.edge = EdgeId(n);
                }
            }
        }
        let mut vused = vec![false; self.body.vertices.len()];
        for e in &self.body.edges {
            for v in [e.start, e.end] {
                if let Some(u) = vused.get_mut(v.0 as usize) {
                    *u = true;
                }
            }
        }
        let mut vrenumber = vec![u32::MAX; vused.len()];
        let mut vkept = Vec::new();
        let mut vkept_ids = Vec::new();
        for (i, v) in std::mem::take(&mut self.body.vertices)
            .into_iter()
            .enumerate()
        {
            if vused.get(i).copied().unwrap_or(false) {
                if let Some(r) = vrenumber.get_mut(i) {
                    *r = vkept.len() as u32;
                }
                vkept.push(v);
                vkept_ids.push(self.vertex_ids.get(i).copied().unwrap_or(0));
            }
        }
        self.body.vertices = vkept;
        self.vertex_ids = vkept_ids;
        for e in &mut self.body.edges {
            if let Some(&n) = vrenumber.get(e.start.0 as usize) {
                e.start = VertexId(n);
            }
            if let Some(&n) = vrenumber.get(e.end.0 as usize) {
                e.end = VertexId(n);
            }
        }
    }
}

/// The coedges of a wire put in order by their vertices: one loop per
/// set that joins up (an Euler circuit, so a seam crossed twice and a
/// closed edge in the middle stay in one loop), never turning straight
/// back along the edge just used while another way is open. `None` when
/// some coedge cannot be part of a closed loop. The coedges are indexed
/// by the vertex they start at, so a wire of many edges costs about its
/// length.
fn chain(uses: &[(Coedge, VertexId, VertexId)]) -> Option<Vec<Vec<Coedge>>> {
    // Unused coedges by start vertex, in the wire's order (the last
    // entry of each list is the next to take, so they are reversed).
    let mut from: BTreeMap<VertexId, Vec<usize>> = BTreeMap::new();
    for (i, u) in uses.iter().enumerate().rev() {
        from.entry(u.1).or_default().push(i);
    }
    let mut used = vec![false; uses.len()];
    let mut next_first = 0usize;
    let mut loops = Vec::new();
    // Takes an unused coedge from `vertex`, not along `avoid` if another
    // is open.
    let take = |from: &mut BTreeMap<VertexId, Vec<usize>>,
                used: &mut [bool],
                vertex: VertexId,
                avoid: Option<EdgeId>|
     -> Option<usize> {
        let list = from.get_mut(&vertex)?;
        while list
            .last()
            .is_some_and(|&i| used.get(i).copied().unwrap_or(true))
        {
            list.pop();
        }
        let pick = list
            .iter()
            .rev()
            .take(4)
            .position(|&i| {
                !used.get(i).copied().unwrap_or(true) && uses.get(i).map(|u| u.0.edge) != avoid
            })
            .map(|k| list.len() - 1 - k)
            .or_else(|| list.len().checked_sub(1))?;
        let i = list.remove(pick);
        if let Some(u) = used.get_mut(i) {
            *u = true;
        }
        Some(i)
    };
    loop {
        while used.get(next_first).copied() == Some(true) {
            next_first += 1;
        }
        if next_first >= uses.len() {
            break;
        }
        let first = next_first;
        if let Some(u) = used.get_mut(first) {
            *u = true;
        }
        let mut stack: Vec<usize> = vec![first];
        let mut circuit: Vec<usize> = Vec::new();
        let mut current = uses.get(first)?.2;
        let mut steps = 0usize;
        loop {
            steps += 1;
            if steps > 4 * uses.len() + 8 {
                return None;
            }
            let last_edge = stack.last().and_then(|&i| uses.get(i)).map(|u| u.0.edge);
            if let Some(i) = take(&mut from, &mut used, current, last_edge) {
                stack.push(i);
                current = uses.get(i)?.2;
            } else if let Some(i) = stack.pop() {
                circuit.push(i);
                current = uses.get(i)?.1;
            } else {
                break;
            }
        }
        circuit.reverse();
        // A closed circuit ends where it starts.
        let (Some(&a), Some(&b)) = (circuit.first(), circuit.last()) else {
            return None;
        };
        if uses.get(a)?.1 != uses.get(b)?.2 {
            return None;
        }
        loops.push(
            circuit
                .iter()
                .filter_map(|&i| uses.get(i).map(|u| u.0))
                .collect(),
        );
    }
    Some(loops)
}

#[cfg(test)]
pub(crate) fn tests_place() -> Place {
    Place {
        m: brep::IDENTITY,
        scale: 1.0,
    }
}

#[cfg(test)]
pub(crate) fn tests_curve(
    c: &brep::Curve,
    place: &Place,
    range: (f64, f64),
) -> Result<(Curve3, (f64, f64)), ImportError> {
    curve(c, place, range, 0)
}

#[cfg(test)]
pub(crate) fn tests_chain(uses: &[(Coedge, VertexId, VertexId)]) -> Option<Vec<Vec<Coedge>>> {
    chain(uses)
}

#[cfg(test)]
pub(crate) fn tests_surface_kind(s: &brep::Surface) -> Result<&'static str, ImportError> {
    Ok(match surface(s, &tests_place(), 0)? {
        Read::Done(s, _) => s.kind(),
        Read::Unbounded { .. } => "extrusion",
        Read::Spindle(_) => "spindle",
    })
}
