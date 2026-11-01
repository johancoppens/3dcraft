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

// `!(x > 0.0)` is deliberate: unlike `x <= 0.0` it is also true for NaN.
#![allow(clippy::neg_cmp_op_on_partial_ord)]
use crate::spindle::{Sheet, Spindle, flipped};
use crate::{ImportError, Parsed, err, keywords, real};
use craft3d_geom::{
    Circle, Cone, Curve3, Cylinder, Ellipse, Line, NurbsCurve, NurbsSurface, Plane, Sphere,
    Surface, Torus,
};
use craft3d_math::Portable;
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
        if let Some(form) = Implicit::of_curve(entity) {
            return self.implicit_curve(id, entity, form);
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
        // The knots first: their multiplicities must fit the number of
        // control points before anything of that size is allocated.
        let knot_vector = self.knots(id, knots.get(0), knots.get(1), ids.len(), degree)?;
        self.finish_curve(id, degree, knot_vector, ids, weights)
    }

    /// A B-spline curve whose knots its type implies (`UNIFORM_CURVE`,
    /// `QUASI_UNIFORM_CURVE`, `BEZIER_CURVE`), simple
    /// (`UNIFORM_CURVE(name, degree, points, form, closed, self)`) or
    /// complex with `B_SPLINE_CURVE` and possibly
    /// `RATIONAL_B_SPLINE_CURVE`.
    fn implicit_curve(
        &self,
        id: u64,
        entity: &Entity,
        form: Implicit,
    ) -> Result<Curve3, ImportError> {
        let (curve, weights) = match entity {
            Entity::Simple(record) => {
                if record.params.len() < 6 {
                    return self.bad(id, &format!("{} has too few attributes", form.curve()));
                }
                (record.params.get(1..6).unwrap_or(&[]), None)
            }
            Entity::Complex(_) => {
                let curve = self.parsed.record(id, "B_SPLINE_CURVE")?;
                let weights = entity
                    .record("RATIONAL_B_SPLINE_CURVE")
                    .and_then(|r| r.params.first());
                (curve.params.as_slice(), weights)
            }
        };
        let degree = self.degree(id, curve.first())?;
        let ids = curve.get(1).and_then(Param::as_list).unwrap_or(&[]);
        if ids.is_empty() || ids.len() > MAX_CONTROL_POINTS {
            return self.bad(id, "the number of control points is out of range");
        }
        let knots = self.implied_knots(id, form, ids.len(), degree)?;
        self.finish_curve(id, degree, knots, ids, weights)
    }

    /// The knots ISO 10303-42 gives a B-spline of `count` control points
    /// of `degree` by its type (`b_spline_curve`'s derived knots):
    /// uniform, `−degree, …, count`; quasi-uniform, the ends repeated
    /// `degree + 1` times and every whole number between; Bézier, pieces
    /// of `degree` each, the inner knots repeated `degree` times.
    pub(crate) fn implied_knots(
        &self,
        id: u64,
        form: Implicit,
        count: usize,
        degree: usize,
    ) -> Result<Vec<f64>, ImportError> {
        if count <= degree {
            return self.bad(
                id,
                &format!("{count} control points are too few for degree {degree}"),
            );
        }
        let total = count + degree + 1;
        let knots: Vec<f64> = match form {
            Implicit::Uniform => (0..total).map(|k| k as f64 - degree as f64).collect(),
            Implicit::QuasiUniform => {
                let inner = count - degree;
                (0..total)
                    .map(|k| k.saturating_sub(degree).min(inner) as f64)
                    .collect()
            }
            Implicit::Bezier => {
                if !(count - 1).is_multiple_of(degree) {
                    return self.bad(
                        id,
                        &format!(
                            "a piecewise Bézier curve of degree {degree} needs a multiple of {degree} control points and one, not {count}"
                        ),
                    );
                }
                let pieces = (count - 1) / degree;
                let mut knots = vec![0.0; degree + 1];
                for piece in 1..pieces {
                    knots.extend(std::iter::repeat_n(piece as f64, degree));
                }
                knots.extend(std::iter::repeat_n(pieces as f64, degree + 1));
                knots
            }
        };
        if knots.len() == total {
            Ok(knots)
        } else {
            self.bad(id, "the implied knots do not fit the control points")
        }
    }

    /// The control points and weights of a B-spline curve, with its knots.
    fn finish_curve(
        &self,
        id: u64,
        degree: usize,
        knot_vector: Vec<f64>,
        ids: &[Param],
        weights: Option<&Param>,
    ) -> Result<Curve3, ImportError> {
        let mut points = Vec::with_capacity(ids.len());
        for p in ids {
            points.push(self.point(self.reference(id, Some(p), "control_points_list")?)?);
        }
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
    /// returned surface's; or a surface that needs the face's extent
    /// ([`Read::Extrusion`]).
    pub fn surface(&self, id: u64) -> Result<Read, ImportError> {
        self.surface_at(id, 0)
    }

    fn surface_at(&self, id: u64, depth: usize) -> Result<Read, ImportError> {
        if depth > MAX_CURVE_DEPTH {
            return self.bad(
                id,
                &format!("a surface defined through more than {MAX_CURVE_DEPTH} others"),
            );
        }
        let entity = &self.parsed.instance(id)?.entity;
        let geom = |e: craft3d_geom::GeomError| ImportError(format!("{}: {e}", self.describe(id)));
        if entity.is("PLANE") {
            let record = self.named(id, "PLANE", 1)?;
            let frame = self.placement(self.reference(id, record.params.get(1), "position")?)?;
            return Ok(Read::Done(Surface::Plane(Plane::new(frame)), false));
        }
        if entity.is("CYLINDRICAL_SURFACE") {
            let record = self.named(id, "CYLINDRICAL_SURFACE", 2)?;
            let frame = self.placement(self.reference(id, record.params.get(1), "position")?)?;
            let radius = self.length(id, record.params.get(2), "radius")?;
            return Ok(Read::Done(
                Surface::Cylinder(Cylinder::new(frame, radius).map_err(geom)?),
                false,
            ));
        }
        if entity.is("CONICAL_SURFACE") {
            let record = self.named(id, "CONICAL_SURFACE", 3)?;
            let frame = self.placement(self.reference(id, record.params.get(1), "position")?)?;
            let radius = self.length(id, record.params.get(2), "radius")?;
            let angle = self.number(id, record.params.get(3), "semi_angle")? * self.radians;
            return Ok(Read::Done(
                Surface::Cone(Cone::new(frame, radius, angle).map_err(geom)?),
                false,
            ));
        }
        if entity.is("SPHERICAL_SURFACE") {
            let record = self.named(id, "SPHERICAL_SURFACE", 2)?;
            let frame = self.placement(self.reference(id, record.params.get(1), "position")?)?;
            let radius = self.length(id, record.params.get(2), "radius")?;
            return Ok(Read::Done(
                Surface::Sphere(Sphere::new(frame, radius).map_err(geom)?),
                false,
            ));
        }
        if entity.is("TOROIDAL_SURFACE") || entity.is("DEGENERATE_TOROIDAL_SURFACE") {
            // A `DEGENERATE_TOROIDAL_SURFACE` (a subtype) has one more
            // attribute: which of its two parts it is.
            let degenerate = entity.is("DEGENERATE_TOROIDAL_SURFACE");
            let keyword = if degenerate {
                "DEGENERATE_TOROIDAL_SURFACE"
            } else {
                "TOROIDAL_SURFACE"
            };
            let record = self.named(id, keyword, if degenerate { 4 } else { 3 })?;
            let frame = self.placement(self.reference(id, record.params.get(1), "position")?)?;
            let major = self.length(id, record.params.get(2), "major_radius")?;
            let minor = self.length(id, record.params.get(3), "minor_radius")?;
            if major > minor || !(major > 0.0 && minor > 0.0) {
                // A ring torus (a degenerate one that is not degenerate is
                // read as what it is), or radii that are refused.
                return Ok(Read::Done(
                    Surface::Torus(Torus::new(frame, major, minor).map_err(geom)?),
                    false,
                ));
            }
            let select = if degenerate {
                match record.params.get(4).and_then(crate::boolean) {
                    Some(outer) => Some(outer),
                    None => return self.bad(id, "select_outer is not .T. or .F."),
                }
            } else {
                None
            };
            return Spindle::toroidal(frame, major, minor, select, false)
                .and_then(Spindle::read)
                .or_else(|e| self.bad(id, &e));
        }
        if entity.is("B_SPLINE_SURFACE_WITH_KNOTS") || Implicit::of_surface(entity).is_some() {
            return Ok(Read::Done(self.bspline_surface(id, entity)?, false));
        }
        if entity.is("SURFACE_OF_LINEAR_EXTRUSION") {
            return self.extrusion(id);
        }
        if entity.is("SURFACE_OF_REVOLUTION") {
            return self.revolution(id);
        }
        if entity.is("OFFSET_SURFACE") {
            return self.offset(id, depth);
        }
        err(format!(
            "#{id} {}: this kind of surface is not read",
            keywords(entity)
        ))
    }

    fn bspline_surface(&self, id: u64, entity: &Entity) -> Result<Surface, ImportError> {
        let implicit = if entity.is("B_SPLINE_SURFACE_WITH_KNOTS") {
            None
        } else {
            Implicit::of_surface(entity)
        };
        let (surface, knots, weights) = match (entity, implicit) {
            // UNIFORM_SURFACE(name, u_degree, v_degree, points, form,
            // u_closed, v_closed, self_intersect).
            (Entity::Simple(record), Some(form)) => {
                if record.params.len() < 8 {
                    return self.bad(id, &format!("{} has too few attributes", form.surface()));
                }
                (
                    Attrs {
                        params: record.params.get(1..8).unwrap_or(&[]),
                    },
                    Attrs { params: &[] },
                    None,
                )
            }
            (Entity::Complex(_), Some(_)) => {
                let surface = self.parsed.record(id, "B_SPLINE_SURFACE")?;
                let weights = entity
                    .record("RATIONAL_B_SPLINE_SURFACE")
                    .and_then(|r| r.params.first());
                (
                    Attrs {
                        params: &surface.params,
                    },
                    Attrs { params: &[] },
                    weights,
                )
            }
            (Entity::Simple(record), None) => {
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
            (Entity::Complex(_), None) => {
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
        let (knots_u, knots_v) = match implicit {
            Some(form) => (
                self.implied_knots(id, form, count_u, degree_u)?,
                self.implied_knots(id, form, count_v, degree_v)?,
            ),
            None => (
                self.knots(id, knots.get(0), knots.get(2), count_u, degree_u)?,
                self.knots(id, knots.get(1), knots.get(3), count_v, degree_v)?,
            ),
        };
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
    /// `C(u) + v V`, normal `C'(u) × V`. A line extruded is a plane, a
    /// circle along its axis a cylinder; any other curve (a circle
    /// slantwise, an ellipse, a polyline, a B-spline) is the exact
    /// B-spline surface of degree one across, once the faces on it say
    /// how far it reaches ([`Read::Extrusion`], [`extruded`]).
    fn extrusion(&self, id: u64) -> Result<Read, ImportError> {
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
            Curve3::Circle(circle)
                if circle.frame().z().cross(along).length() <= 1e-9 * along.length() =>
            {
                let cylinder = Cylinder::new(*circle.frame(), circle.radius())
                    .map_err(|e| ImportError(format!("{}: {e}", self.describe(id))))?;
                Surface::Cylinder(cylinder)
            }
            other => {
                let Some(nurbs) = as_nurbs(other) else {
                    return self.bad(
                        id,
                        &format!("an extrusion of a {} is not read", other.kind()),
                    );
                };
                // The B-spline's own parametrisation is the file's (u
                // along the curve, v along V), so its normal is the
                // file's.
                return Ok(Read::Extrusion {
                    id,
                    curve: nurbs,
                    along,
                });
            }
        };
        for (point, tangent) in Self::samples(&curve) {
            let normal = tangent.cross(along);
            if normal.length() > 0.0 && normal.length().is_finite() {
                return Ok(Read::Done(
                    surface.clone(),
                    self.opposite(id, &surface, point, normal)?,
                ));
            }
        }
        self.bad(id, "the swept curve has no direction")
    }

    /// `SURFACE_OF_REVOLUTION(name, swept_curve, axis_position)`: the curve
    /// turned by `u` about the axis, `v` the curve's parameter; normal
    /// `∂u × ∂v`, where `∂u = a × (C(v) − L)` at `u = 0`. A line or a
    /// circle in a plane through the axis turns into one of the elementary
    /// surfaces; any other curve (a B-spline, an ellipse, a circle out of
    /// such a plane) into the exact rational B-spline surface
    /// ([`revolved`]). A circle whose torus reaches the axis is a spindle
    /// or horn torus ([`Spindle`]), whose part a face chooses.
    fn revolution(&self, id: u64) -> Result<Read, ImportError> {
        let record = self.named(id, "SURFACE_OF_REVOLUTION", 2)?;
        let curve = self.curve_at(self.reference(id, record.params.get(1), "swept_curve")?, 1)?;
        let (at, axis) =
            self.axis1(self.reference(id, record.params.get(2), "axis_position")?)?;
        let geom = |e: craft3d_geom::GeomError| ImportError(format!("{}: {e}", self.describe(id)));
        // Small compared with the size of what is turned.
        let scale = |p: Vec3| (p - at).length().max(1e-300);
        let elementary = match &curve {
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
                    Some(Surface::Cylinder(
                        Cylinder::new(frame, radius).map_err(geom)?,
                    ))
                } else if axis.dot(d).abs() <= 1e-12 {
                    let frame = Frame::from_z(at + axis * off.dot(axis), axis)
                        .ok_or_else(|| ImportError(format!("{}: no plane", self.describe(id))))?;
                    Some(Surface::Plane(Plane::new(frame)))
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
                    let half_angle = (reach / height).patan();
                    let frame = Frame::from_z(apex, axis)
                        .ok_or_else(|| ImportError(format!("{}: no axis", self.describe(id))))?;
                    Some(Surface::Cone(
                        Cone::new(frame, 0.0, half_angle).map_err(geom)?,
                    ))
                }
            }
            Curve3::Circle(circle) => {
                let frame = circle.frame();
                let centre = frame.origin();
                let off = centre - at;
                let size = circle.radius() + off.length();
                if frame.z().dot(axis).abs() > 1e-9 || off.dot(frame.z()).abs() > 1e-9 * size {
                    // Out of every plane through the axis.
                    None
                } else {
                    let foot = at + axis * off.dot(axis);
                    let major = (centre - foot).length();
                    if major <= 1e-12 * size {
                        let frame = Frame::from_z(centre, axis).ok_or_else(|| {
                            ImportError(format!("{}: no axis", self.describe(id)))
                        })?;
                        Some(Surface::Sphere(
                            Sphere::new(frame, circle.radius()).map_err(geom)?,
                        ))
                    } else {
                        let frame = Frame::new(foot, axis, centre - foot).ok_or_else(|| {
                            ImportError(format!("{}: no axis", self.describe(id)))
                        })?;
                        if major > circle.radius() {
                            Some(Surface::Torus(
                                Torus::new(frame, major, circle.radius()).map_err(geom)?,
                            ))
                        } else {
                            // A horn or spindle torus: the file's normal
                            // against the standard torus's, at a point of
                            // the circle clear of the axis.
                            let flip =
                                Self::samples(&curve)
                                    .into_iter()
                                    .find_map(|(point, tangent)| {
                                        let normal = axis.cross(point - at).cross(tangent);
                                        flipped(&frame, major, point, normal)
                                    });
                            let Some(flip) = flip else {
                                return self.bad(
                                    id,
                                    "cannot tell which way the swept surface's normal points",
                                );
                            };
                            return Spindle::toroidal(frame, major, circle.radius(), None, flip)
                                .and_then(Spindle::read)
                                .or_else(|e| self.bad(id, &e));
                        }
                    }
                }
            }
            _ => None,
        };
        let surface = match elementary {
            Some(surface) => surface,
            None => {
                let Some(nurbs) = as_nurbs(&curve) else {
                    return self.bad(
                        id,
                        &format!("a revolution of a {} is not read", curve.kind()),
                    );
                };
                // u round the axis as the angle runs, v along the curve:
                // the file's normal, whatever the parametrisation of u.
                let surface = revolved(&nurbs, at, axis)
                    .map_err(|e| ImportError(format!("{}: {e}", self.describe(id))))?;
                return Ok(Read::Done(Surface::Nurbs(surface), false));
            }
        };
        for (point, tangent) in Self::samples(&curve) {
            let du = axis.cross(point - at);
            let normal = du.cross(tangent);
            if normal.length() > 1e-9 * scale(point) * tangent.length() {
                return Ok(Read::Done(
                    surface.clone(),
                    self.opposite(id, &surface, point, normal)?,
                ));
            }
        }
        self.bad(
            id,
            "cannot tell which way the swept surface's normal points",
        )
    }

    /// `OFFSET_SURFACE(name, basis_surface, distance, self_intersect)`:
    /// `S(u, v) + d N(u, v)`, `N` the unit normal of the basis's own
    /// parametrisation. An offset of a plane, a cylinder, a cone, a sphere
    /// or a torus is the same kind of surface again (a plane moved, a
    /// radius changed); an offset of a B-spline or a swept surface is no
    /// kind here and is refused, naming it. The offset runs the basis's
    /// way, so the file's normal is opposite to ours exactly when the
    /// basis's is.
    fn offset(&self, id: u64, depth: usize) -> Result<Read, ImportError> {
        let record = self.named(id, "OFFSET_SURFACE", 2)?;
        let basis = self.reference(id, record.params.get(1), "basis_surface")?;
        let distance = self.length(id, record.params.get(2), "distance")?;
        let (surface, opposite) = match self.surface_at(basis, depth + 1)? {
            Read::Done(surface, opposite) => (surface, opposite),
            Read::Spindle(spindle) => {
                return spindle.offset(distance).or_else(|e| self.bad(id, &e));
            }
            Read::Extrusion { .. } => {
                return self.bad(
                    id,
                    "an offset of an extruded curve is not read (no surface of that kind)",
                );
            }
        };
        let geom = |e: craft3d_geom::GeomError| ImportError(format!("{}: {e}", self.describe(id)));
        // The file's normal at a point of the basis, and whether it points
        // away from the axis or centre (`outward` of that point).
        let file_normal = |u: f64, v: f64| -> Result<Vec3, ImportError> {
            let ours = surface.normal(u, v).ok_or_else(|| {
                ImportError(format!("{}: the basis has no normal", self.describe(id)))
            })?;
            Ok(if opposite { -ours } else { ours })
        };
        let gone = || {
            ImportError(format!(
                "{}: the offset goes through the basis's axis or centre; it is not read",
                self.describe(id)
            ))
        };
        let moved = match &surface {
            Surface::Plane(plane) => {
                let n = file_normal(0.0, 0.0)?;
                let frame = plane.frame();
                let frame = frame.at(frame.origin() + n * distance).ok_or_else(gone)?;
                Surface::Plane(Plane::new(frame))
            }
            Surface::Cylinder(c) => {
                let p = surface.eval(0.0, 0.0);
                let radial = p
                    - c.frame().origin()
                    - c.frame().z() * (p - c.frame().origin()).dot(c.frame().z());
                let sign = file_normal(0.0, 0.0)?.dot(radial).signum();
                let radius = c.radius() + sign * distance;
                if !(radius > 0.0) {
                    return Err(gone());
                }
                Surface::Cylinder(Cylinder::new(*c.frame(), radius).map_err(geom)?)
            }
            Surface::Sphere(s) => {
                let p = surface.eval(0.0, 0.0);
                let sign = file_normal(0.0, 0.0)?.dot(p - s.frame().origin()).signum();
                let radius = s.radius() + sign * distance;
                if !(radius > 0.0) {
                    return Err(gone());
                }
                Surface::Sphere(Sphere::new(*s.frame(), radius).map_err(geom)?)
            }
            Surface::Torus(t) => {
                let (major, minor) = t.radii();
                // The tube's centre line at u = 0 is the frame's x axis
                // at the major radius.
                let p = surface.eval(0.0, 0.0);
                let centre = t.frame().point(major, 0.0, 0.0);
                let sign = file_normal(0.0, 0.0)?.dot(p - centre).signum();
                let minor = minor + sign * distance;
                if !(minor > 0.0) {
                    return Err(gone());
                }
                if minor >= major {
                    // The tube now reaches the axis: a horn or spindle
                    // torus, both of whose parts are where the ring's
                    // points went, along the same normal.
                    let sheet = Sheet {
                        minor,
                        outward: sign > 0.0,
                    };
                    return Spindle {
                        frame: *t.frame(),
                        major,
                        apple: sheet,
                        lemon: sheet,
                        select: None,
                    }
                    .offset(0.0)
                    .or_else(|e| self.bad(id, &e));
                }
                Surface::Torus(Torus::new(*t.frame(), major, minor).map_err(geom)?)
            }
            Surface::Cone(c) => {
                // A point of the cone's own nappe (where its radius is
                // positive: past the apex in the direction it opens), its
                // normal's part away from the axis: the radius at v = 0
                // grows by d / cos α.
                let v = c.apex_v() + c.half_angle().ptan().signum();
                let p = surface.eval(0.0, v);
                let o = c.frame().origin();
                let radial = p - o - c.frame().z() * (p - o).dot(c.frame().z());
                let sign = file_normal(0.0, v)?.dot(radial).signum();
                let radius = c.radius() + sign * distance / c.half_angle().pcos();
                if !(radius >= 0.0) {
                    return Err(gone());
                }
                Surface::Cone(Cone::new(*c.frame(), radius, c.half_angle()).map_err(geom)?)
            }
            Surface::Bezier(_) | Surface::Nurbs(_) => {
                return self.bad(
                    id,
                    "an offset of a B-spline surface is not read (no exact surface of that kind)",
                );
            }
        };
        Ok(Read::Done(moved, opposite))
    }
}

/// A surface as read: done, with whether the file's normal is opposite
/// to it; or an extrusion of a curve that is no elementary kind, which
/// becomes a B-spline surface once the faces on it say how far it
/// reaches (`SURFACE_OF_LINEAR_EXTRUSION` is unbounded along its
/// direction, a B-spline surface is not).
#[derive(Clone, Debug)]
pub(crate) enum Read {
    Done(Surface, bool),
    Extrusion {
        id: u64,
        curve: NurbsCurve,
        along: Vec3,
    },
    /// A horn or spindle torus: which of its parts a face is on, its edges
    /// tell ([`Spindle::sheet_of`]).
    Spindle(Spindle),
}

/// The B-spline types whose knots are implied by their type
/// (ISO 10303-42).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Implicit {
    Uniform,
    QuasiUniform,
    Bezier,
}

impl Implicit {
    pub(crate) fn of_curve(entity: &Entity) -> Option<Implicit> {
        if entity.is("UNIFORM_CURVE") {
            Some(Implicit::Uniform)
        } else if entity.is("QUASI_UNIFORM_CURVE") {
            Some(Implicit::QuasiUniform)
        } else if entity.is("BEZIER_CURVE") {
            Some(Implicit::Bezier)
        } else {
            None
        }
    }

    fn of_surface(entity: &Entity) -> Option<Implicit> {
        if entity.is("UNIFORM_SURFACE") {
            Some(Implicit::Uniform)
        } else if entity.is("QUASI_UNIFORM_SURFACE") {
            Some(Implicit::QuasiUniform)
        } else if entity.is("BEZIER_SURFACE") {
            Some(Implicit::Bezier)
        } else {
            None
        }
    }

    fn curve(self) -> &'static str {
        match self {
            Implicit::Uniform => "UNIFORM_CURVE",
            Implicit::QuasiUniform => "QUASI_UNIFORM_CURVE",
            Implicit::Bezier => "BEZIER_CURVE",
        }
    }

    fn surface(self) -> &'static str {
        match self {
            Implicit::Uniform => "UNIFORM_SURFACE",
            Implicit::QuasiUniform => "QUASI_UNIFORM_SURFACE",
            Implicit::Bezier => "BEZIER_SURFACE",
        }
    }
}

/// The rational quadratic circle of *The NURBS Book* §7.5 in the plane
/// `centre + x a + y b` (`a`, `b` the semi-axes as vectors, any two
/// vectors: an ellipse, a circle, or one point when both are zero):
/// nine control points `(x, y, weight)`, the knots of four quarters.
const RING: [(f64, f64, bool); 9] = [
    (1.0, 0.0, false),
    (1.0, 1.0, true),
    (0.0, 1.0, false),
    (-1.0, 1.0, true),
    (-1.0, 0.0, false),
    (-1.0, -1.0, true),
    (0.0, -1.0, false),
    (1.0, -1.0, true),
    (1.0, 0.0, false),
];
const RING_KNOTS: [f64; 12] = [
    0.0, 0.0, 0.0, 0.25, 0.25, 0.5, 0.5, 0.75, 0.75, 1.0, 1.0, 1.0,
];

fn ring_weight(corner: bool) -> f64 {
    if corner {
        std::f64::consts::FRAC_1_SQRT_2
    } else {
        1.0
    }
}

/// A bounded curve as the exact B-spline it is: a circle or an ellipse
/// as the rational quadratic of four quarters (starting on its x axis, as
/// its own parameter does), a B-spline as itself. `None` for a line,
/// which has no ends.
fn as_nurbs(curve: &Curve3) -> Option<NurbsCurve> {
    let (frame, a, b) = match curve {
        Curve3::Nurbs(n) => return Some(n.clone()),
        Curve3::Line(_) => return None,
        Curve3::Circle(c) => (*c.frame(), c.radius(), c.radius()),
        Curve3::Ellipse(e) => {
            let (a, b) = e.semi_axes();
            (*e.frame(), a, b)
        }
    };
    let points = RING
        .iter()
        .map(|&(x, y, _)| frame.point(x * a, y * b, 0.0))
        .collect();
    let weights = RING.iter().map(|&(_, _, c)| ring_weight(c)).collect();
    NurbsCurve::new(2, RING_KNOTS.to_vec(), points, weights).ok()
}

/// The surface `curve` sweeps turning once about the axis through `at`
/// along `axis` (a unit vector), as the exact rational B-spline (*The
/// NURBS Book* §8.5): u round the axis in four rational quadratic
/// quarters, counter-clockwise seen from the axis's tip, v the curve's
/// own parameter. A control point on the axis gives a row that is one
/// point (a pole).
pub(crate) fn revolved(
    curve: &NurbsCurve,
    at: Vec3,
    axis: Vec3,
) -> Result<NurbsSurface, craft3d_geom::GeomError> {
    let net = curve.control_points();
    let own = curve.weights();
    let mut points = Vec::with_capacity(RING.len().saturating_mul(net.len()));
    let mut weights = Vec::with_capacity(points.capacity());
    for &(x, y, corner) in &RING {
        let ring = ring_weight(corner);
        for (&p, &w) in net.iter().zip(own) {
            let off = p - at;
            let foot = at + axis * off.dot(axis);
            let radial = p - foot;
            points.push(foot + radial * x + axis.cross(radial) * y);
            weights.push(ring * w);
        }
    }
    NurbsSurface::new(
        2,
        curve.degree(),
        RING_KNOTS.to_vec(),
        curve.knots().to_vec(),
        RING.len(),
        net.len(),
        points,
        weights,
    )
}

/// The surface `curve` sweeps along `along` from `v0` to `v1` (in units
/// of `along`), as the exact B-spline of degree one across: u the curve's
/// own parameter, v along `along`, so the normal is `C'(u) × V` as the
/// file's is.
pub(crate) fn extruded(
    curve: &NurbsCurve,
    along: Vec3,
    (v0, v1): (f64, f64),
) -> Result<NurbsSurface, craft3d_geom::GeomError> {
    let net = curve.control_points();
    let own = curve.weights();
    let mut points = Vec::with_capacity(net.len().saturating_mul(2));
    let mut weights = Vec::with_capacity(points.capacity());
    for (&p, &w) in net.iter().zip(own) {
        points.push(p + along * v0);
        points.push(p + along * v1);
        weights.extend([w, w]);
    }
    NurbsSurface::new(
        curve.degree(),
        1,
        curve.knots().to_vec(),
        vec![v0, v0, v1, v1],
        net.len(),
        2,
        points,
        weights,
    )
}
