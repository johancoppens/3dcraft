//! Points, placements, curves and surfaces (ISO 10303-42, the geometry
//! schema), read as `craft3d-geom` values in the caller's unit.
//!
//! The parametrisations are those `craft3d-io`'s export documents (the
//! elementary surfaces of Part 42 have the parametrisations of
//! `craft3d-geom`), so frames and radii go over unchanged, scaled by the
//! unit. A swept surface that is an elementary one is read as that, and
//! whether its normal (`∂u × ∂v` of the swept surface's own
//! parametrisation) is the elementary surface's or the opposite is
//! measured at a point of it.

use crate::{ImportError, Parsed, err, keywords, real};
use craft3d_geom::{
    Circle, Cone, Curve3, Cylinder, Ellipse, Line, NurbsCurve, NurbsSurface, Plane, Sphere,
    Surface, Torus,
};
use craft3d_math::{Frame, Vec3};
use craft3d_step::{Entity, Param, Record};

/// How deep a curve may be defined through others (a surface curve of a
/// surface curve, the curve a swept surface sweeps).
const MAX_CURVE_DEPTH: usize = 4;
/// Highest B-spline degree read.
const MAX_DEGREE: i64 = 32;
/// Most control points of one B-spline curve or surface.
const MAX_CONTROL_POINTS: usize = 4_000_000;

pub(crate) struct Reader<'a> {
    pub parsed: &'a Parsed,
    /// What a length of the file is multiplied by.
    pub scale: f64,
    /// Radians per angle unit of the file.
    pub radians: f64,
}

fn finite(v: Vec3) -> bool {
    v.x.is_finite() && v.y.is_finite() && v.z.is_finite()
}

fn triple(list: &[Param]) -> Option<Vec3> {
    match list {
        [x, y, z] => Some(Vec3::new(real(x)?, real(y)?, real(z)?)),
        _ => None,
    }
}

/// The parameters of an entity of a supertype chain in a simple instance
/// (all attributes in one record, the inherited first) or in a complex
/// one (one record per type): `own` are the attributes of `keyword`
/// itself, counted from its first own attribute.
struct Attrs<'a> {
    params: &'a [Param],
}

impl<'a> Attrs<'a> {
    fn get(&self, at: usize) -> Option<&'a Param> {
        self.params.get(at)
    }
}

impl<'a> Reader<'a> {
    pub fn new(parsed: &'a Parsed, scale: f64, radians: f64) -> Self {
        Self {
            parsed,
            scale,
            radians,
        }
    }

    fn describe(&self, id: u64) -> String {
        self.parsed.describe(id)
    }

    fn bad<T>(&self, id: u64, what: &str) -> Result<T, ImportError> {
        err(format!("{}: {what}", self.describe(id)))
    }

    /// A record with `name` first, then `count` more attributes at least.
    fn named(&self, id: u64, keyword: &str, count: usize) -> Result<&'a Record, ImportError> {
        let record = self.parsed.record(id, keyword)?;
        if record.params.len() < count + 1 {
            return self.bad(id, &format!("{keyword} has too few attributes"));
        }
        Ok(record)
    }

    fn reference(&self, id: u64, param: Option<&Param>, what: &str) -> Result<u64, ImportError> {
        param
            .and_then(Param::as_ref)
            .map_or_else(|| self.bad(id, &format!("{what} is not a reference")), Ok)
    }

    fn number(&self, id: u64, param: Option<&Param>, what: &str) -> Result<f64, ImportError> {
        match param.and_then(real) {
            Some(x) if x.is_finite() => Ok(x),
            _ => self.bad(id, &format!("{what} is not a finite number")),
        }
    }

    fn length(&self, id: u64, param: Option<&Param>, what: &str) -> Result<f64, ImportError> {
        let value = self.number(id, param, what)? * self.scale;
        if value.is_finite() {
            Ok(value)
        } else {
            self.bad(id, &format!("{what} is too large"))
        }
    }

    pub fn point(&self, id: u64) -> Result<Vec3, ImportError> {
        let record = self.named(id, "CARTESIAN_POINT", 1)?;
        let coordinates = record
            .params
            .get(1)
            .and_then(Param::as_list)
            .and_then(triple);
        match coordinates {
            Some(p) if finite(p * self.scale) => Ok(p * self.scale),
            Some(_) => self.bad(id, "a coordinate is not finite"),
            None => self.bad(id, "not a point with three coordinates"),
        }
    }

    pub fn direction(&self, id: u64) -> Result<Vec3, ImportError> {
        let record = self.named(id, "DIRECTION", 1)?;
        let ratios = record
            .params
            .get(1)
            .and_then(Param::as_list)
            .and_then(triple);
        match ratios.and_then(|d| d.normalized()) {
            Some(d) => Ok(d),
            None => self.bad(id, "not a direction with three finite ratios, not all zero"),
        }
    }

    /// `AXIS2_PLACEMENT_3D`: `axis` is z and `ref_direction` x, made
    /// perpendicular to z (Part 42's `build_axes`); either may be omitted:
    /// z is then (0, 0, 1) and x the first of (1, 0, 0), (0, 1, 0) not
    /// parallel to z.
    pub fn placement(&self, id: u64) -> Result<Frame, ImportError> {
        let record = self.named(id, "AXIS2_PLACEMENT_3D", 1)?;
        let origin = self.point(self.reference(id, record.params.get(1), "location")?)?;
        let z = match record.params.get(2) {
            Some(Param::Ref(axis)) => self.direction(*axis)?,
            _ => Vec3::Z,
        };
        let x = match record.params.get(3) {
            Some(Param::Ref(reference)) => self.direction(*reference)?,
            _ if z.cross(Vec3::X).length() > 1e-6 => Vec3::X,
            _ => Vec3::Y,
        };
        Frame::new(origin, z, x).map_or_else(
            || self.bad(id, "its ref_direction is parallel to its axis"),
            Ok,
        )
    }

    /// `AXIS1_PLACEMENT`: a point and a direction ((0, 0, 1) when
    /// omitted).
    fn axis1(&self, id: u64) -> Result<(Vec3, Vec3), ImportError> {
        let record = self.named(id, "AXIS1_PLACEMENT", 1)?;
        let origin = self.point(self.reference(id, record.params.get(1), "location")?)?;
        let axis = match record.params.get(2) {
            Some(Param::Ref(axis)) => self.direction(*axis)?,
            _ => Vec3::Z,
        };
        Ok((origin, axis))
    }

    /// A `VECTOR`: its direction times its magnitude (a length).
    fn vector(&self, id: u64) -> Result<Vec3, ImportError> {
        let record = self.named(id, "VECTOR", 2)?;
        let direction =
            self.direction(self.reference(id, record.params.get(1), "orientation")?)?;
        let magnitude = self.length(id, record.params.get(2), "magnitude")?;
        if magnitude <= 0.0 {
            return self.bad(id, "the magnitude of a vector must be positive");
        }
        Ok(direction * magnitude)
    }

    /// The curve an edge lies on.
    pub fn curve(&self, id: u64) -> Result<Curve3, ImportError> {
        self.curve_at(id, 0)
    }

    fn curve_at(&self, id: u64, depth: usize) -> Result<Curve3, ImportError> {
        if depth > MAX_CURVE_DEPTH {
            return self.bad(
                id,
                &format!("a curve defined through more than {MAX_CURVE_DEPTH} others"),
            );
        }
        let entity = &self.parsed.instance(id)?.entity;
        let geom = |e: craft3d_geom::GeomError| ImportError(format!("{}: {e}", self.describe(id)));
        if entity.is("LINE") {
            let record = self.named(id, "LINE", 2)?;
            let origin = self.point(self.reference(id, record.params.get(1), "pnt")?)?;
            let along = self.vector(self.reference(id, record.params.get(2), "dir")?)?;
            return Ok(Curve3::Line(Line::new(origin, along).map_err(geom)?));
        }
        if entity.is("CIRCLE") {
            let record = self.named(id, "CIRCLE", 2)?;
            let frame = self.placement(self.reference(id, record.params.get(1), "position")?)?;
            let radius = self.length(id, record.params.get(2), "radius")?;
            return Ok(Curve3::Circle(Circle::new(frame, radius).map_err(geom)?));
        }
        if entity.is("ELLIPSE") {
            let record = self.named(id, "ELLIPSE", 3)?;
            let frame = self.placement(self.reference(id, record.params.get(1), "position")?)?;
            let a = self.length(id, record.params.get(2), "semi_axis_1")?;
            let b = self.length(id, record.params.get(3), "semi_axis_2")?;
            return Ok(Curve3::Ellipse(Ellipse::new(frame, a, b).map_err(geom)?));
        }
        for keyword in ["SURFACE_CURVE", "SEAM_CURVE", "INTERSECTION_CURVE"] {
            if entity.is(keyword) {
                let record = self.named(id, keyword, 1)?;
                let curve = self.reference(id, record.params.get(1), "curve_3d")?;
                return self.curve_at(curve, depth + 1);
            }
        }
        if entity.is("POLYLINE") {
            let record = self.named(id, "POLYLINE", 1)?;
            let ids = record.params.get(1).and_then(Param::as_list).unwrap_or(&[]);
            if ids.len() < 2 || ids.len() > MAX_CONTROL_POINTS {
                return self.bad(id, "a polyline needs 2 points or more (and not millions)");
            }
            let mut points = Vec::with_capacity(ids.len());
            for p in ids {
                points.push(self.point(self.reference(id, Some(p), "points")?)?);
            }
            // Degree 1 on the knots 0, 0, 1, 2, …, n-1, n-1.
            let last = (points.len() - 1) as f64;
            let mut knots = vec![0.0];
            knots.extend((0..points.len()).map(|i| i as f64));
            knots.push(last);
            return Ok(Curve3::Nurbs(
                NurbsCurve::polynomial(1, knots, points).map_err(geom)?,
            ));
        }
        if entity.is("B_SPLINE_CURVE_WITH_KNOTS") {
            return self.bspline_curve(id, entity);
        }
        self.bad(id, "this kind of curve is not read")
    }

    /// The attributes of `B_SPLINE_CURVE` (degree, control points, form,
    /// closed, self-intersect), `B_SPLINE_CURVE_WITH_KNOTS`
    /// (multiplicities, knots, spec) and the weights of a rational one.
    fn bspline_curve(&self, id: u64, entity: &Entity) -> Result<Curve3, ImportError> {
        let (curve, knots, weights) = match entity {
            Entity::Simple(record) => {
                if record.params.len() < 9 {
                    return self.bad(id, "B_SPLINE_CURVE_WITH_KNOTS has too few attributes");
                }
                let all = &record.params;
                (
                    Attrs {
                        params: all.get(1..6).unwrap_or(&[]),
                    },
                    Attrs {
                        params: all.get(6..).unwrap_or(&[]),
                    },
                    None,
                )
            }
            Entity::Complex(_) => {
                let curve = self.parsed.record(id, "B_SPLINE_CURVE")?;
                let knots = self.parsed.record(id, "B_SPLINE_CURVE_WITH_KNOTS")?;
                let weights = entity
                    .record("RATIONAL_B_SPLINE_CURVE")
                    .and_then(|r| r.params.first());
                (
                    Attrs {
                        params: &curve.params,
                    },
                    Attrs {
                        params: &knots.params,
                    },
                    weights,
                )
            }
        };
        let degree = self.degree(id, curve.get(0))?;
        let ids = curve.get(1).and_then(Param::as_list).unwrap_or(&[]);
        if ids.is_empty() || ids.len() > MAX_CONTROL_POINTS {
            return self.bad(id, "the number of control points is out of range");
        }
        let mut points = Vec::with_capacity(ids.len());
        for p in ids {
            points.push(self.point(self.reference(id, Some(p), "control_points_list")?)?);
        }
        let knot_vector = self.knots(id, knots.get(0), knots.get(1), points.len(), degree)?;
        let weights = match weights {
            None => vec![1.0; points.len()],
            Some(list) => {
                let values = self.reals(id, list, "weights_data")?;
                if values.len() != points.len() {
                    return self.bad(id, "there are not as many weights as control points");
                }
                values
            }
        };
        NurbsCurve::new(degree, knot_vector, points, weights)
            .map(Curve3::Nurbs)
            .map_err(|e| ImportError(format!("{}: {e}", self.describe(id))))
    }

    fn degree(&self, id: u64, param: Option<&Param>) -> Result<usize, ImportError> {
        match param.and_then(Param::as_integer) {
            Some(d) if (1..=MAX_DEGREE).contains(&d) => Ok(d as usize),
            _ => self.bad(
                id,
                &format!("the degree must be a whole number from 1 to {MAX_DEGREE}"),
            ),
        }
    }

    fn reals(&self, id: u64, param: &Param, what: &str) -> Result<Vec<f64>, ImportError> {
        let Some(list) = param.as_list() else {
            return self.bad(id, &format!("{what} is not a list"));
        };
        let mut out = Vec::with_capacity(list.len().min(MAX_CONTROL_POINTS));
        for item in list {
            out.push(self.number(id, Some(item), what)?);
        }
        Ok(out)
    }

    /// The knot vector from distinct knots and their multiplicities, which
    /// must add up to `count + degree + 1` (checked before anything is
    /// allocated).
    fn knots(
        &self,
        id: u64,
        multiplicities: Option<&Param>,
        knots: Option<&Param>,
        count: usize,
        degree: usize,
    ) -> Result<Vec<f64>, ImportError> {
        let (Some(multiplicities), Some(knots)) = (
            multiplicities.and_then(Param::as_list),
            knots.and_then(Param::as_list),
        ) else {
            return self.bad(id, "the knots or their multiplicities are not lists");
        };
        if multiplicities.len() != knots.len() {
            return self.bad(id, "there are not as many multiplicities as knots");
        }
        let expected = count.saturating_add(degree).saturating_add(1);
        let mut total: usize = 0;
        for m in multiplicities {
            match m.as_integer() {
                Some(m) if m >= 1 => {
                    total = total.saturating_add(usize::try_from(m).unwrap_or(usize::MAX));
                }
                _ => return self.bad(id, "a knot multiplicity is not a positive whole number"),
            }
        }
        if total != expected {
            return self.bad(
                id,
                &format!(
                    "the multiplicities add up to {total} knots, where {count} control points of degree {degree} need {expected}"
                ),
            );
        }
        let mut out = Vec::with_capacity(expected);
        for (m, k) in multiplicities.iter().zip(knots) {
            let k = self.number(id, Some(k), "knots")?;
            let m = m
                .as_integer()
                .and_then(|m| usize::try_from(m).ok())
                .unwrap_or(0);
            out.extend(std::iter::repeat_n(k, m));
        }
        Ok(out)
    }

    /// The surface of a face, and whether the file's normal for it (the
    /// `∂u × ∂v` of the file's parametrisation) is opposite to the
    /// returned surface's.
    pub fn surface(&self, id: u64) -> Result<(Surface, bool), ImportError> {
        let entity = &self.parsed.instance(id)?.entity;
        let geom = |e: craft3d_geom::GeomError| ImportError(format!("{}: {e}", self.describe(id)));
        if entity.is("PLANE") {
            let record = self.named(id, "PLANE", 1)?;
            let frame = self.placement(self.reference(id, record.params.get(1), "position")?)?;
            return Ok((Surface::Plane(Plane::new(frame)), false));
        }
        if entity.is("CYLINDRICAL_SURFACE") {
            let record = self.named(id, "CYLINDRICAL_SURFACE", 2)?;
            let frame = self.placement(self.reference(id, record.params.get(1), "position")?)?;
            let radius = self.length(id, record.params.get(2), "radius")?;
            return Ok((
                Surface::Cylinder(Cylinder::new(frame, radius).map_err(geom)?),
                false,
            ));
        }
        if entity.is("CONICAL_SURFACE") {
            let record = self.named(id, "CONICAL_SURFACE", 3)?;
            let frame = self.placement(self.reference(id, record.params.get(1), "position")?)?;
            let radius = self.length(id, record.params.get(2), "radius")?;
            let angle = self.number(id, record.params.get(3), "semi_angle")? * self.radians;
            return Ok((
                Surface::Cone(Cone::new(frame, radius, angle).map_err(geom)?),
                false,
            ));
        }
        if entity.is("SPHERICAL_SURFACE") {
            let record = self.named(id, "SPHERICAL_SURFACE", 2)?;
            let frame = self.placement(self.reference(id, record.params.get(1), "position")?)?;
            let radius = self.length(id, record.params.get(2), "radius")?;
            return Ok((
                Surface::Sphere(Sphere::new(frame, radius).map_err(geom)?),
                false,
            ));
        }
        if entity.is("TOROIDAL_SURFACE") && !entity.is("DEGENERATE_TOROIDAL_SURFACE") {
            let record = self.named(id, "TOROIDAL_SURFACE", 3)?;
            let frame = self.placement(self.reference(id, record.params.get(1), "position")?)?;
            let major = self.length(id, record.params.get(2), "major_radius")?;
            let minor = self.length(id, record.params.get(3), "minor_radius")?;
            return Ok((
                Surface::Torus(Torus::new(frame, major, minor).map_err(geom)?),
                false,
            ));
        }
        if entity.is("B_SPLINE_SURFACE_WITH_KNOTS") {
            return Ok((self.bspline_surface(id, entity)?, false));
        }
        if entity.is("SURFACE_OF_LINEAR_EXTRUSION") {
            return self.extrusion(id);
        }
        if entity.is("SURFACE_OF_REVOLUTION") {
            return self.revolution(id);
        }
        err(format!(
            "#{id} {}: this kind of surface is not read",
            keywords(entity)
        ))
    }

    fn bspline_surface(&self, id: u64, entity: &Entity) -> Result<Surface, ImportError> {
        let (surface, knots, weights) = match entity {
            Entity::Simple(record) => {
                if record.params.len() < 13 {
                    return self.bad(id, "B_SPLINE_SURFACE_WITH_KNOTS has too few attributes");
                }
                let all = &record.params;
                (
                    Attrs {
                        params: all.get(1..8).unwrap_or(&[]),
                    },
                    Attrs {
                        params: all.get(8..).unwrap_or(&[]),
                    },
                    None,
                )
            }
            Entity::Complex(_) => {
                let surface = self.parsed.record(id, "B_SPLINE_SURFACE")?;
                let knots = self.parsed.record(id, "B_SPLINE_SURFACE_WITH_KNOTS")?;
                let weights = entity
                    .record("RATIONAL_B_SPLINE_SURFACE")
                    .and_then(|r| r.params.first());
                (
                    Attrs {
                        params: &surface.params,
                    },
                    Attrs {
                        params: &knots.params,
                    },
                    weights,
                )
            }
        };
        let degree_u = self.degree(id, surface.get(0))?;
        let degree_v = self.degree(id, surface.get(1))?;
        let rows = surface.get(2).and_then(Param::as_list).unwrap_or(&[]);
        let count_u = rows.len();
        let count_v = rows
            .first()
            .and_then(Param::as_list)
            .map_or(0, <[Param]>::len);
        if count_u == 0 || count_v == 0 || count_u.saturating_mul(count_v) > MAX_CONTROL_POINTS {
            return self.bad(id, "the net of control points is empty or too large");
        }
        let mut points = Vec::with_capacity(count_u * count_v);
        for row in rows {
            let row = row.as_list().unwrap_or(&[]);
            if row.len() != count_v {
                return self.bad(id, "the rows of control points differ in length");
            }
            for p in row {
                points.push(self.point(self.reference(id, Some(p), "control_points_list")?)?);
            }
        }
        let knots_u = self.knots(id, knots.get(0), knots.get(2), count_u, degree_u)?;
        let knots_v = self.knots(id, knots.get(1), knots.get(3), count_v, degree_v)?;
        let weights = match weights {
            None => vec![1.0; points.len()],
            Some(list) => {
                let rows = list.as_list().unwrap_or(&[]);
                if rows.len() != count_u {
                    return self.bad(id, "the weights are not one row per row of control points");
                }
                let mut out = Vec::with_capacity(points.len());
                for row in rows {
                    let values = self.reals(id, row, "weights_data")?;
                    if values.len() != count_v {
                        return self.bad(id, "a row of weights differs in length from its points");
                    }
                    out.extend(values);
                }
                out
            }
        };
        NurbsSurface::new(
            degree_u, degree_v, knots_u, knots_v, count_u, count_v, points, weights,
        )
        .map(Surface::Nurbs)
        .map_err(|e| ImportError(format!("{}: {e}", self.describe(id))))
    }

    /// Whether `normal`, the file's normal at `point`, is opposite to
    /// `surface`'s normal there.
    fn opposite(
        &self,
        id: u64,
        surface: &Surface,
        point: Vec3,
        normal: Vec3,
    ) -> Result<bool, ImportError> {
        let ours = surface
            .project(point)
            .ok()
            .and_then(|(u, v)| surface.normal(u, v));
        match ours {
            Some(ours) if ours.dot(normal).abs() > 1e-9 * normal.length() => {
                Ok(ours.dot(normal) < 0.0)
            }
            _ => self.bad(
                id,
                "cannot tell which way the swept surface's normal points",
            ),
        }
    }

    /// Points along a curve where its derivative is not zero, for the
    /// normal of a surface swept from it.
    fn samples(curve: &Curve3) -> Vec<(Vec3, Vec3)> {
        let (t0, t1) = match curve.domain() {
            (a, b) if a.is_finite() && b.is_finite() => (a, b),
            _ => (0.0, 1.0),
        };
        [0.37, 0.61, 0.13, 0.89]
            .iter()
            .map(|s| curve.derivatives(t0 + (t1 - t0) * s))
            .collect()
    }

    /// `SURFACE_OF_LINEAR_EXTRUSION(name, swept_curve, extrusion_axis)`:
    /// `C(u) + v V`, normal `C'(u) × V`.
    fn extrusion(&self, id: u64) -> Result<(Surface, bool), ImportError> {
        let record = self.named(id, "SURFACE_OF_LINEAR_EXTRUSION", 2)?;
        let curve = self.curve_at(self.reference(id, record.params.get(1), "swept_curve")?, 1)?;
        let along = self.vector(self.reference(id, record.params.get(2), "extrusion_axis")?)?;
        let surface = match &curve {
            Curve3::Line(line) => {
                let normal = line.direction().cross(along);
                let frame = Frame::new(line.origin(), normal, line.direction());
                match frame {
                    Some(frame) => Surface::Plane(Plane::new(frame)),
                    None => return self.bad(id, "a line extruded along itself"),
                }
            }
            Curve3::Circle(circle) => {
                let frame = circle.frame();
                if frame.z().cross(along).length() > 1e-9 * along.length() {
                    return self.bad(
                        id,
                        "a circle extruded slantwise (an elliptic cylinder) is not read",
                    );
                }
                let cylinder = Cylinder::new(*frame, circle.radius())
                    .map_err(|e| ImportError(format!("{}: {e}", self.describe(id))))?;
                Surface::Cylinder(cylinder)
            }
            other => {
                return self.bad(
                    id,
                    &format!(
                        "an extrusion of a {} is not read (no surface of that kind)",
                        other.kind()
                    ),
                );
            }
        };
        for (point, tangent) in Self::samples(&curve) {
            let normal = tangent.cross(along);
            if normal.length() > 0.0 && normal.length().is_finite() {
                return Ok((surface.clone(), self.opposite(id, &surface, point, normal)?));
            }
        }
        self.bad(id, "the swept curve has no direction")
    }

    /// `SURFACE_OF_REVOLUTION(name, swept_curve, axis_position)`: the curve
    /// turned by `u` about the axis, `v` the curve's parameter; normal
    /// `∂u × ∂v`, where `∂u = a × (C(v) − L)` at `u = 0`.
    fn revolution(&self, id: u64) -> Result<(Surface, bool), ImportError> {
        let record = self.named(id, "SURFACE_OF_REVOLUTION", 2)?;
        let curve = self.curve_at(self.reference(id, record.params.get(1), "swept_curve")?, 1)?;
        let (at, axis) =
            self.axis1(self.reference(id, record.params.get(2), "axis_position")?)?;
        let geom = |e: craft3d_geom::GeomError| ImportError(format!("{}: {e}", self.describe(id)));
        // Small compared with the size of what is turned.
        let scale = |p: Vec3| (p - at).length().max(1e-300);
        let surface = match &curve {
            Curve3::Line(line) => {
                let d = line.direction();
                let p = line.origin();
                let off = p - at;
                if off.dot(axis.cross(d)).abs() > 1e-9 * scale(p) {
                    return self.bad(
                        id,
                        "a line turned about an axis it does not meet (a hyperboloid) is not read",
                    );
                }
                let radial = off - axis * off.dot(axis);
                if axis.cross(d).length() <= 1e-12 {
                    let radius = radial.length();
                    let frame = Frame::new(at, axis, radial).ok_or_else(|| {
                        ImportError(format!("{}: a line on its axis", self.describe(id)))
                    })?;
                    Surface::Cylinder(Cylinder::new(frame, radius).map_err(geom)?)
                } else if axis.dot(d).abs() <= 1e-12 {
                    let frame = Frame::from_z(at + axis * off.dot(axis), axis)
                        .ok_or_else(|| ImportError(format!("{}: no plane", self.describe(id))))?;
                    Surface::Plane(Plane::new(frame))
                } else {
                    // The apex: where the line meets the axis.
                    let along = d.dot(axis);
                    let t = -radial.dot(d - axis * along) / (d - axis * along).length_squared();
                    let apex_on_line = p + d * t;
                    let apex = at + axis * (apex_on_line - at).dot(axis);
                    // The nappe the line's own point is on (a cone here
                    // ends at its apex).
                    let far = if (p - apex_on_line).length() > 1e-9 * scale(p) {
                        p
                    } else {
                        p + d * scale(p).max(1.0)
                    };
                    let height = (far - apex).dot(axis);
                    let reach = (far - apex - axis * height).length();
                    let half_angle = (reach / height).atan();
                    let frame = Frame::from_z(apex, axis)
                        .ok_or_else(|| ImportError(format!("{}: no axis", self.describe(id))))?;
                    Surface::Cone(Cone::new(frame, 0.0, half_angle).map_err(geom)?)
                }
            }
            Curve3::Circle(circle) => {
                let frame = circle.frame();
                let centre = frame.origin();
                let off = centre - at;
                let size = circle.radius() + off.length();
                if frame.z().dot(axis).abs() > 1e-9 || off.dot(frame.z()).abs() > 1e-9 * size {
                    return self.bad(
                        id,
                        "a circle turned about an axis outside its plane is not read",
                    );
                }
                let foot = at + axis * off.dot(axis);
                let major = (centre - foot).length();
                if major <= 1e-12 * size {
                    let frame = Frame::from_z(centre, axis)
                        .ok_or_else(|| ImportError(format!("{}: no axis", self.describe(id))))?;
                    Surface::Sphere(Sphere::new(frame, circle.radius()).map_err(geom)?)
                } else {
                    let frame = Frame::new(foot, axis, centre - foot)
                        .ok_or_else(|| ImportError(format!("{}: no axis", self.describe(id))))?;
                    Surface::Torus(Torus::new(frame, major, circle.radius()).map_err(geom)?)
                }
            }
            other => {
                return self.bad(
                    id,
                    &format!(
                        "a revolution of a {} is not read (no surface of that kind)",
                        other.kind()
                    ),
                );
            }
        };
        for (point, tangent) in Self::samples(&curve) {
            let du = axis.cross(point - at);
            let normal = du.cross(tangent);
            if normal.length() > 1e-9 * scale(point) * tangent.length() {
                return Ok((surface.clone(), self.opposite(id, &surface, point, normal)?));
            }
        }
        self.bad(
            id,
            "cannot tell which way the swept surface's normal points",
        )
    }
}
