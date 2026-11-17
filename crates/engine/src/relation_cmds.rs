//! The relation of two things (`measure.between`, and the assertion and
//! output measure `between`): what the window shows by itself when two
//! things are selected, and what an agent gets for the same two. Each
//! thing is a face, an edge, a vertex or a body, as `select.get` lists
//! the selection; what it is decides the relation, on the exact geometry:
//!
//! - two flat faces: parallel, the distance between their planes; at an
//!   angle, the angle between them, measured from the line their planes
//!   meet in toward each face;
//! - two axes (of cylindrical and conical faces), or the centres of
//!   circular edges and spheres: the centre distance, with its parts along
//!   X, Y and Z, drawn for two parallel axes where they open (at the end
//!   both reach);
//! - two straight edges: parallel, the distance between them; meeting (or
//!   they would, their lines crossing), the angle between them; skew, the
//!   least distance between the edges as they are, segments;
//! - an axis and a flat face: parallel, the distance between them; else
//!   the angle between them;
//! - two vertices: the distance between them;
//! - a vertex, a circular edge or a sphere and anything else: the least
//!   distance between the two as they are (the rim's curve, the sphere's
//!   surface; never from a centre: the review of 2026-11-17);
//! - two bodies: the least clearance (`measure.clearance`'s);
//! - anything else of faces and bodies: the least distance between them
//!   (`measure.distance`'s).
//!
//! Each answer has the two points a dimension line is drawn between (for
//! an angle, the corner and a point toward each side).

use crate::commands::{Run, length_unit, spec, vec3};
use crate::params::Params;
use crate::select_cmds::query;
use crate::selection::SelectionKind;
use crate::verify::{self, Part};
use crate::verify_cmds::{Ctx, Dim, Outcome, Sampled, feature_of, measured_document, only};
use crate::{CommandError, CommandSpec, Session};
use craft3d_doc::FeatureId;
use craft3d_geom::{Curve3, Surface};
use craft3d_math::{Portable, Vec3};
use craft3d_topo::Body;
use serde_json::{Map, Value, json};
use std::sync::Arc;

pub(crate) static SPECS: &[CommandSpec] = &[spec!(
    "measure.between",
    "Between Two",
    r#"{"from": item, "to": item, "relation": "distance|angle|centerDistance|leastDistance|clearance" = the one the two call for}"#,
    "The relation of two things, as the window shows it when two are selected: each an item {kind: face|edge|vertex|body, name, feature} as select.get lists the selection (a body also by its feature's name or id). Without relation it is chosen from what the two are, on the exact geometry: two flat faces, parallel: distance (between their planes; from and to are the ends of the dimension line, square to them, between their middles); at an angle: angle (between the half-planes from the line the two planes meet in toward each face, 0 to 180 degrees: two sides of a block meet at 90, a 45 degree chamfer meets the top at 135); two axes (of cylindrical and conical faces: holes, bosses) or the centres of circular edges and spheres: centerDistance (between the axes, or from a centre to an axis or a centre; parallel says whether two axes are; two parallel axes drawn at the end both reach, where the holes open); two straight edges: parallel, centerDistance (the distance between them); meeting, or their lines crossing, angle (between them, from where they meet toward each); skew, leastDistance (between the edges as segments); an axis and a flat face: distance when parallel, else angle (between the axis and the plane, 0 to 90); two vertices: distance; a vertex, a circular edge or a sphere with anything else: leastDistance (between the two as they are: the rim, the sphere's surface, never their centres); two bodies: clearance (as measure.clearance: null with overlap: true when they overlap); anything else of faces and bodies: leastDistance (as measure.distance). Returns relation, value (in the document's length unit; an angle in radians, with degrees), words (what it is, in a few words), from and to (the two ends of the dimension line; for an angle the points toward each side, with at, its corner), parts ([x, y, z], to less from, for a distance between points), parallel (two planes or axes), tolerance (the document's: linear, or angular in radians for an angle), atLeast (proven, where the least was searched for) and lengthUnit. With relation that one is measured, or refused when the two cannot have it (distance between faces that are not parallel). Read-only. As an assertion or output: measure between (from, to, relation), the value in the document's length unit or radians.",
    doc: true,
    measure_between
)];

/// What a relation is, as `relation` names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Relation {
    Distance,
    Angle,
    CenterDistance,
    LeastDistance,
    Clearance,
}

impl Relation {
    const ALL: [Relation; 5] = [
        Relation::Distance,
        Relation::Angle,
        Relation::CenterDistance,
        Relation::LeastDistance,
        Relation::Clearance,
    ];

    pub(crate) fn name(self) -> &'static str {
        match self {
            Relation::Distance => "distance",
            Relation::Angle => "angle",
            Relation::CenterDistance => "centerDistance",
            Relation::LeastDistance => "leastDistance",
            Relation::Clearance => "clearance",
        }
    }

    fn parse(text: &str) -> Result<Relation, String> {
        Relation::ALL
            .into_iter()
            .find(|r| r.name() == text)
            .ok_or_else(|| {
                format!(
                    "`relation` is distance, angle, centerDistance, leastDistance or clearance, not `{}`",
                    text.chars().take(40).collect::<String>()
                )
            })
    }
}

/// A thing as it was named: what kind, in which body, which of its faces,
/// edges or vertices.
struct Thing {
    kind: SelectionKind,
    feature: FeatureId,
    body: Arc<Body>,
    indices: Vec<usize>,
}

/// What a thing is, geometrically.
#[derive(Clone, Copy, Debug)]
enum Shape {
    /// A flat face: a point of its plane, its outward unit normal, and the
    /// middle of its boundary laid onto the plane.
    Plane {
        point: Vec3,
        normal: Vec3,
        middle: Vec3,
    },
    /// An axis (of a round face) or a straight edge: a point of the line,
    /// its unit direction, and the point of it at the thing's middle.
    Line {
        direction: Vec3,
        middle: Vec3,
        edge: bool,
        /// How far the thing reaches along `direction`, as `x · direction`
        /// from its least to its most (a hole from end to end, an edge
        /// from end to end).
        ends: (f64, f64),
    },
    /// A vertex, or the centre of a circular edge or a sphere.
    Point { at: Vec3, centre: bool },
    /// Any other face, or several.
    Faces,
    /// A curved edge that is not a circle.
    Curve,
    /// A whole body.
    Body,
}

/// What was measured.
struct Found {
    relation: Relation,
    value: Option<f64>,
    words: &'static str,
    from: Vec3,
    to: Vec3,
    at: Option<Vec3>,
    parallel: Option<bool>,
    at_least: Option<f64>,
    sampled: bool,
    overlap: bool,
}

impl Found {
    fn exact(relation: Relation, value: f64, words: &'static str, from: Vec3, to: Vec3) -> Self {
        Found {
            relation,
            value: Some(value),
            words,
            from,
            to,
            at: None,
            parallel: None,
            at_least: None,
            sampled: false,
            overlap: false,
        }
    }
}

/// Points along a face's edges, `n` along each.
fn boundary(body: &Body, faces: &[usize], n: u32) -> Vec<Vec3> {
    faces
        .iter()
        .flat_map(|&f| verify::face_edges(body, f))
        .flat_map(|e| verify::edge_points(e, n))
        .filter(|p| p.is_finite())
        .take(100_000)
        .collect()
}

fn mean(points: &[Vec3]) -> Option<Vec3> {
    if points.is_empty() {
        return None;
    }
    let n = points.len() as f64;
    let m = points
        .iter()
        .fold(Vec3::ZERO, |acc, &p| acc + p * (1.0 / n));
    m.is_finite().then_some(m)
}

/// The point of the line through `point` along the unit `direction`
/// nearest to `p`.
fn foot(point: Vec3, direction: Vec3, p: Vec3) -> Vec3 {
    point + direction * (p - point).dot(direction)
}

/// From the least to the most of `x · d` over `points`; `(0, 0)` for none.
fn reach_along(points: &[Vec3], d: Vec3) -> (f64, f64) {
    let along = points.iter().map(|p| p.dot(d)).filter(|x| x.is_finite());
    let lo = along.clone().fold(f64::INFINITY, f64::min);
    let hi = along.fold(f64::NEG_INFINITY, f64::max);
    if lo <= hi { (lo, hi) } else { (0.0, 0.0) }
}

/// The two ends of a straight edge `t`, when it is one.
fn segment(t: &Thing) -> Option<(Vec3, Vec3)> {
    if t.kind != SelectionKind::Edge {
        return None;
    }
    let edge = t.indices.first().and_then(|&e| t.body.edges.get(e))?;
    let Curve3::Line(_) = &edge.curve else {
        return None;
    };
    let (t0, t1) = edge.range;
    let (a, b) = (edge.curve.eval(t0), edge.curve.eval(t1));
    (a.is_finite() && b.is_finite()).then_some((a, b))
}

/// The closest points of the segments from `a0` to `a1` and from `b0` to
/// `b1` (any lengths, a point among them).
fn closest_segments(a0: Vec3, a1: Vec3, b0: Vec3, b1: Vec3) -> (Vec3, Vec3) {
    let (u, v, w) = (a1 - a0, b1 - b0, a0 - b0);
    let (a, b, c, d, e) = (u.dot(u), u.dot(v), v.dot(v), u.dot(w), v.dot(w));
    let clamp = |x: f64| {
        if x.is_finite() {
            x.clamp(0.0, 1.0)
        } else {
            0.0
        }
    };
    let denominator = a * c - b * b;
    // The parameter along the first, then the second's for it, then the
    // first's again for that (the textbook's clamped solution).
    let mut s = if denominator > 1e-300 * (a * c).max(1e-300) {
        clamp((b * e - c * d) / denominator)
    } else {
        0.0
    };
    let mut t = if c > 0.0 { (b * s + e) / c } else { 0.0 };
    if !(0.0..=1.0).contains(&t) {
        t = clamp(t);
        s = if a > 0.0 { clamp((b * t - d) / a) } else { 0.0 };
    }
    (a0 + u * s, b0 + v * t)
}

/// How two straight edges lie: parallel, meeting (their lines crossing,
/// within the tolerance), or skew.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EdgePair {
    Parallel,
    Meeting,
    Skew,
}

/// What the thing `t` is.
fn shape_of(t: &Thing) -> Shape {
    let body = &t.body;
    match t.kind {
        SelectionKind::Face => {
            let Some(first) = t.indices.first().and_then(|&f| body.faces.get(f)) else {
                return Shape::Faces;
            };
            let edge_points = boundary(body, &t.indices, 8);
            let middle = mean(&edge_points);
            match &first.surface {
                Surface::Plane(plane) => {
                    let frame = plane.frame();
                    let (point, z) = (frame.origin(), frame.z());
                    let normal = if first.reversed { Vec3::ZERO - z } else { z };
                    // Every piece on the one plane.
                    let one = t.indices.iter().all(|&f| {
                        body.faces.get(f).is_some_and(|face| match &face.surface {
                            Surface::Plane(other) => {
                                let o = other.frame();
                                o.z().dot(z).abs() > 1.0 - 1e-12
                                    && (o.origin() - point).dot(z).abs()
                                        <= 1e-9 * (1.0 + point.length())
                            }
                            _ => false,
                        })
                    });
                    match middle {
                        Some(m) if one => Shape::Plane {
                            point,
                            normal,
                            middle: m - z * (m - point).dot(z),
                        },
                        _ => Shape::Faces,
                    }
                }
                Surface::Cylinder(_) | Surface::Cone(_) => {
                    match (verify::axis_of(&first.surface), middle) {
                        (Some((point, direction)), Some(m)) => match direction.normalized() {
                            Some(d) => Shape::Line {
                                direction: d,
                                middle: foot(point, d, m),
                                edge: false,
                                ends: reach_along(&edge_points, d),
                            },
                            None => Shape::Faces,
                        },
                        _ => Shape::Faces,
                    }
                }
                Surface::Sphere(sphere) => Shape::Point {
                    at: sphere.frame().origin(),
                    centre: true,
                },
                _ => Shape::Faces,
            }
        }
        SelectionKind::Edge => {
            let Some(edge) = t.indices.first().and_then(|&e| body.edges.get(e)) else {
                return Shape::Curve;
            };
            match &edge.curve {
                Curve3::Line(_) => {
                    let (t0, t1) = edge.range;
                    let (a, b) = (edge.curve.eval(t0), edge.curve.eval(t1));
                    match (b - a).normalized() {
                        Some(d) if a.is_finite() && b.is_finite() => Shape::Line {
                            direction: d,
                            middle: a.lerp(b, 0.5),
                            edge: true,
                            ends: reach_along(&[a, b], d),
                        },
                        _ => Shape::Curve,
                    }
                }
                Curve3::Circle(circle) => Shape::Point {
                    at: circle.frame().origin(),
                    centre: true,
                },
                _ => Shape::Curve,
            }
        }
        SelectionKind::Vertex => match t.indices.first().and_then(|&v| body.vertices.get(v)) {
            Some(v) => Shape::Point {
                at: v.point,
                centre: false,
            },
            None => Shape::Curve,
        },
        _ => Shape::Body,
    }
}

/// The relation two shapes call for (`edges`: how two straight edges
/// lie, when both are).
fn chosen(
    a: &Shape,
    b: &Shape,
    parallel_planes: bool,
    parallel_axis_plane: bool,
    edges: Option<EdgePair>,
) -> Relation {
    use Shape::{Body, Line, Plane, Point};
    match (a, b) {
        (Plane { .. }, Plane { .. }) => {
            if parallel_planes {
                Relation::Distance
            } else {
                Relation::Angle
            }
        }
        (Line { edge: true, .. }, Line { edge: true, .. }) => match edges {
            Some(EdgePair::Meeting) => Relation::Angle,
            Some(EdgePair::Skew) => Relation::LeastDistance,
            _ => Relation::CenterDistance,
        },
        (Line { .. }, Line { .. }) => Relation::CenterDistance,
        (Point { centre: true, .. }, Point { centre: true, .. })
        | (Point { centre: true, .. }, Line { edge: false, .. })
        | (Line { edge: false, .. }, Point { centre: true, .. }) => Relation::CenterDistance,
        // Two vertices: the distance between them. A vertex and a rim or
        // a ball: the least distance to the rim or the ball, not to its
        // centre.
        (Point { centre: false, .. }, Point { centre: false, .. }) => Relation::Distance,
        (Line { .. }, Plane { .. }) | (Plane { .. }, Line { .. }) => {
            if parallel_axis_plane {
                Relation::Distance
            } else {
                Relation::Angle
            }
        }
        (Body, Body) => Relation::Clearance,
        _ => Relation::LeastDistance,
    }
}

/// Whether two unit directions lie along one line within `angular`
/// radians, either way round.
fn along(a: Vec3, b: Vec3, angular: f64) -> bool {
    a.cross(b).length() <= angular.psin().max(1e-12)
}

/// The angle between two unit vectors, 0 to π.
fn angle_between(a: Vec3, b: Vec3) -> f64 {
    if a.length() == 0.0 || b.length() == 0.0 {
        return f64::NAN;
    }
    let y = a.cross(b).length();
    let x = a.dot(b);
    y.patan2(x)
}

/// A unit vector square to the unit `n`.
fn square_to(n: Vec3) -> Vec3 {
    let axis = if n.x.abs() < 0.9 { Vec3::X } else { Vec3::Y };
    n.cross(axis).normalized().unwrap_or(Vec3::Z)
}

/// The unit vector of `v` with its part along the unit `d` taken out,
/// or `fallback` where nothing is left.
fn across(v: Vec3, d: Vec3, fallback: Vec3) -> Vec3 {
    (v - d * v.dot(d)).normalized().unwrap_or(fallback)
}

/// A thing named by `value`: an item `{kind, name, feature}` as
/// `select.get` lists the selection, or a body by its feature's name or
/// id.
fn thing(ctx: &Ctx<'_>, value: &Value, key: &str) -> Result<Thing, String> {
    let body_of = |id: FeatureId| -> Result<Thing, String> {
        Ok(Thing {
            kind: SelectionKind::Body,
            feature: id,
            body: Arc::clone(ctx.body(id)?),
            indices: Vec::new(),
        })
    };
    let text = |v: Option<&Value>, what: &str| -> Result<String, String> {
        match v {
            Some(Value::String(s)) if !s.is_empty() => Ok(s.chars().take(4000).collect()),
            _ => Err(format!("`{key}` needs {what}")),
        }
    };
    match value {
        Value::String(name) => body_of(feature_of(ctx.doc, name)?),
        Value::Object(map) => {
            if let Some(extra) = map
                .keys()
                .find(|k| !matches!(k.as_str(), "kind" | "name" | "feature"))
            {
                return Err(format!(
                    "`{key}` is an item {{kind, name, feature}}, not `{}`",
                    extra.chars().take(40).collect::<String>()
                ));
            }
            let kind = text(map.get("kind"), "a kind: face, edge, vertex or body")?;
            match kind.as_str() {
                "body" | "feature" => {
                    let named = map
                        .get("feature")
                        .or_else(|| map.get("name"))
                        .ok_or_else(|| format!("`{key}` needs the body's feature"))?;
                    body_of(feature_of(
                        ctx.doc,
                        &text(Some(named), "the body's feature")?,
                    )?)
                }
                "face" | "edge" | "vertex" => {
                    let name = text(map.get("name"), "the persistent name")?;
                    let feature = text(map.get("feature"), "the feature of its body")?;
                    let found = query(
                        ctx.doc,
                        ctx.regen,
                        Some(&Value::from(kind.as_str())),
                        Some(&json!({"in": feature, "name": name})),
                        16,
                    )?;
                    let [one] = found.as_slice() else {
                        return Err(if found.is_empty() {
                            format!(
                                "there is no {kind} `{}` in `{feature}` (see document.inspect)",
                                name.chars().take(120).collect::<String>()
                            )
                        } else {
                            format!("`{key}` names {} {kind}s; name one", found.len())
                        });
                    };
                    Ok(Thing {
                        kind: one.kind,
                        feature: one.feature,
                        body: Arc::clone(ctx.body(one.feature)?),
                        indices: one.indices.clone(),
                    })
                }
                other => Err(format!(
                    "`{key}`: a kind is face, edge, vertex or body, not `{}`",
                    other.chars().take(40).collect::<String>()
                )),
            }
        }
        _ => Err(format!(
            "`{key}` is an item {{kind, name, feature}} or a body's name"
        )),
    }
}

/// The faces of a thing taking part in a least distance: a face's
/// pieces, or every face of a body.
fn faces_of(t: &Thing) -> Option<Vec<usize>> {
    match t.kind {
        SelectionKind::Face => Some(t.indices.clone()),
        SelectionKind::Body => Some((0..t.body.faces.len()).collect()),
        _ => None,
    }
}

/// The least distance from `p` to the thing `t`: to its faces, or to its
/// edge; `None` for a thing that has neither.
fn from_point(ctx: &Ctx<'_>, p: Vec3, t: &Thing) -> Result<Option<(f64, Vec3, bool)>, String> {
    if let Some(faces) = faces_of(t) {
        let probe = craft3d_kernel::boolean::probe::Probe::new(&t.body, &ctx.tolerance)
            .map_err(|e| e.to_string())?;
        let mut best: Option<(f64, Vec3)> = None;
        let mut freeform = false;
        for &f in &faces {
            let Some(face) = t.body.faces.get(f) else {
                continue;
            };
            freeform |= matches!(face.surface, Surface::Bezier(_) | Surface::Nurbs(_));
            let q = verify::nearest_on_face(&probe, f, p)?;
            let d = q.distance(p);
            if d.is_finite() && best.is_none_or(|(b, _)| d < b) {
                best = Some((d, q));
            }
        }
        return Ok(best.map(|(d, q)| (d, q, freeform)));
    }
    if t.kind == SelectionKind::Edge
        && let Some(edge) = t.indices.first().and_then(|&e| t.body.edges.get(e))
    {
        let freeform = matches!(edge.curve, Curve3::Nurbs(_));
        return Ok(verify::nearest_on_edge(edge, p).map(|(q, _)| (q.distance(p), q, freeform)));
    }
    if t.kind == SelectionKind::Vertex
        && let Some(v) = t.indices.first().and_then(|&i| t.body.vertices.get(i))
    {
        return Ok(Some((v.point.distance(p), v.point, false)));
    }
    Ok(None)
}

fn refused(relation: Relation, why: &str) -> String {
    format!("no {} here: {why}", relation.name())
}

/// The relation `wanted` (or the one the two call for) of `a` and `b`.
fn measure(ctx: &Ctx<'_>, a: &Thing, b: &Thing, wanted: Option<Relation>) -> Result<Found, String> {
    use Shape::{Line, Plane, Point};
    let (sa, sb) = (shape_of(a), shape_of(b));
    let angular = ctx.tolerance.angular;
    let parallel_planes = match (&sa, &sb) {
        (Plane { normal: na, .. }, Plane { normal: nb, .. }) => along(*na, *nb, angular),
        _ => false,
    };
    let parallel_axis_plane = match (&sa, &sb) {
        (Line { direction, .. }, Plane { normal, .. })
        | (Plane { normal, .. }, Line { direction, .. }) => {
            direction.dot(*normal).abs() <= angular.psin().max(1e-12)
        }
        _ => false,
    };
    let edges = match ((&sa, segment(a)), (&sb, segment(b))) {
        (
            (Line { direction: da, .. }, Some((a0, _))),
            (Line { direction: db, .. }, Some((b0, _))),
        ) => Some(if along(*da, *db, angular) {
            EdgePair::Parallel
        } else {
            let (p, q) = closest(a0, *da, b0, *db);
            if p.distance(q) <= ctx.tolerance.linear {
                EdgePair::Meeting
            } else {
                EdgePair::Skew
            }
        }),
        _ => None,
    };
    let relation =
        wanted.unwrap_or_else(|| chosen(&sa, &sb, parallel_planes, parallel_axis_plane, edges));
    match relation {
        Relation::Distance => match (&sa, &sb) {
            (
                Plane {
                    point: pa,
                    normal: na,
                    middle: ma,
                },
                Plane {
                    point: pb,
                    normal: nb,
                    middle: mb,
                },
            ) => {
                if !parallel_planes {
                    let apart = angle_between(*na, *nb);
                    let apart = apart.min(std::f64::consts::PI - apart);
                    return Err(refused(
                        relation,
                        &format!(
                            "the faces are not parallel ({:.3} degrees apart)",
                            apart.to_degrees()
                        ),
                    ));
                }
                let m = ma.lerp(*mb, 0.5);
                let from = m - *na * (m - *pa).dot(*na);
                let to = m - *na * (m - *pb).dot(*na);
                let mut found = Found::exact(
                    relation,
                    from.distance(to),
                    "Distance between parallel faces",
                    from,
                    to,
                );
                found.parallel = Some(true);
                Ok(found)
            }
            (Line { middle, .. }, Plane { point, normal, .. })
            | (Plane { point, normal, .. }, Line { middle, .. }) => {
                if !parallel_axis_plane {
                    return Err(refused(relation, "the axis is not parallel to the face"));
                }
                let on = *middle - *normal * (*middle - *point).dot(*normal);
                let (from, to) = if matches!(sa, Line { .. }) {
                    (*middle, on)
                } else {
                    (on, *middle)
                };
                let mut found = Found::exact(
                    relation,
                    from.distance(to),
                    "Distance from the axis to the face",
                    from,
                    to,
                );
                found.parallel = Some(true);
                Ok(found)
            }
            (Point { at: p, .. }, Point { at: q, .. }) => Ok(Found::exact(
                relation,
                p.distance(*q),
                "Distance between the points",
                *p,
                *q,
            )),
            _ => Err(refused(
                relation,
                "a distance is between parallel flat faces, an axis and a face parallel to it, or two points",
            )),
        },
        Relation::Angle => {
            let (corner, ua, ub, words) = match (&sa, &sb) {
                (
                    Plane {
                        point: pa,
                        normal: na,
                        middle: ma,
                    },
                    Plane {
                        point: pb,
                        normal: nb,
                        middle: mb,
                    },
                ) => {
                    let d = na.cross(*nb);
                    let Some(dir) = d.normalized().filter(|_| !parallel_planes) else {
                        return Err(refused(relation, "the faces are parallel"));
                    };
                    // The point of the line the planes meet in nearest to
                    // the middle between the faces.
                    let m = ma.lerp(*mb, 0.5);
                    let c = na.dot(*nb);
                    let det = 1.0 - c * c;
                    let (da, db) = (na.dot(*pa - m), nb.dot(*pb - m));
                    let alpha = (da - c * db) / det;
                    let beta = (db - c * da) / det;
                    let corner = m + *na * alpha + *nb * beta;
                    let ua = across(*ma - corner, dir, dir.cross(*na));
                    let ub = across(*mb - corner, dir, dir.cross(*nb));
                    (corner, ua, ub, "Angle between the faces")
                }
                (
                    Line {
                        direction, middle, ..
                    },
                    Plane { point, normal, .. },
                )
                | (
                    Plane { point, normal, .. },
                    Line {
                        direction, middle, ..
                    },
                ) => {
                    let s = direction.dot(*normal);
                    if s.abs() <= angular.psin().max(1e-12) {
                        return Err(refused(relation, "the axis is parallel to the face"));
                    }
                    let t = (*point - *middle).dot(*normal) / s;
                    let corner = *middle + *direction * t;
                    let toward = if (*middle - corner).length() > 0.0 {
                        (*middle - corner).normalized().unwrap_or(*direction)
                    } else {
                        *direction
                    };
                    let flat = across(toward, *normal, square_to(*normal));
                    let (ua, ub) = if matches!(sa, Line { .. }) {
                        (toward, flat)
                    } else {
                        (flat, toward)
                    };
                    (corner, ua, ub, "Angle between the axis and the face")
                }
                (
                    Line {
                        direction: da,
                        middle: ma,
                        ..
                    },
                    Line {
                        direction: db,
                        middle: mb,
                        ..
                    },
                ) => {
                    let (p, q) = closest(*ma, *da, *mb, *db);
                    let corner = p.lerp(q, 0.5);
                    if edges.is_some() {
                        // Edges: from where they meet toward each one's
                        // middle, so the angle is the one between them.
                        let ua = (*ma - corner).normalized().unwrap_or(*da);
                        let ub = (*mb - corner).normalized().unwrap_or(*db);
                        (corner, ua, ub, "Angle between the edges")
                    } else {
                        (corner, *da, *db, "Angle between the axes")
                    }
                }
                _ => {
                    return Err(refused(
                        relation,
                        "an angle is between flat faces, axes, or an axis and a flat face",
                    ));
                }
            };
            let value = angle_between(ua, ub);
            // Points toward each side, at the faces' reach from the corner.
            let reach = match (&sa, &sb) {
                (Plane { middle: ma, .. }, _) | (Line { middle: ma, .. }, _) => ma.distance(corner),
                _ => 1.0,
            };
            let reach = if reach.is_finite() && reach > 0.0 {
                reach
            } else {
                1.0
            };
            let mut found = Found::exact(
                relation,
                value,
                words,
                corner + ua * reach,
                corner + ub * reach,
            );
            found.at = Some(corner);
            found.parallel = Some(false);
            Ok(found)
        }
        Relation::CenterDistance => {
            let (from, to, parallel) = match (&sa, &sb) {
                (
                    Line {
                        direction: da,
                        middle: ma,
                        edge: ea,
                        ends: (lo_a, hi_a),
                    },
                    Line {
                        direction: db,
                        middle: mb,
                        ends: (lo_b, hi_b),
                        ..
                    },
                ) => {
                    if along(*da, *db, angular) {
                        // The second's reach along the first's direction.
                        let (lo_b, hi_b) = if da.dot(*db) >= 0.0 {
                            (*lo_b, *hi_b)
                        } else {
                            (-*hi_b, -*lo_b)
                        };
                        // Two holes: where they open, at the end both
                        // reach (the review of 2026-11-17: at mid-depth
                        // the line floated inside the block). Two edges:
                        // the middle of where they run side by side.
                        let level = if *ea {
                            let (from, to) = (lo_a.max(lo_b), hi_a.min(hi_b));
                            if from <= to {
                                (from + to) * 0.5
                            } else {
                                ma.lerp(*mb, 0.5).dot(*da)
                            }
                        } else {
                            hi_a.min(hi_b)
                        };
                        let at = |m: Vec3| m + *da * (level - m.dot(*da));
                        let (from, to) = (at(*ma), at(*mb));
                        if from.is_finite() && to.is_finite() {
                            (from, to, Some(true))
                        } else {
                            let m = ma.lerp(*mb, 0.5);
                            (foot(*ma, *da, m), foot(*mb, *db, m), Some(true))
                        }
                    } else {
                        let (p, q) = closest(*ma, *da, *mb, *db);
                        (p, q, Some(false))
                    }
                }
                (
                    Point { at, .. },
                    Line {
                        direction, middle, ..
                    },
                ) => (*at, foot(*middle, *direction, *at), None),
                (
                    Line {
                        direction, middle, ..
                    },
                    Point { at, .. },
                ) => (foot(*middle, *direction, *at), *at, None),
                (Point { at: p, .. }, Point { at: q, .. }) => (*p, *q, None),
                _ => {
                    return Err(refused(
                        relation,
                        "a centre distance is between axes and centres (holes, bosses, round edges)",
                    ));
                }
            };
            let words = if edges == Some(EdgePair::Parallel) {
                "Distance between the parallel edges"
            } else {
                "Centre distance"
            };
            let mut found = Found::exact(relation, from.distance(to), words, from, to);
            found.parallel = parallel;
            Ok(found)
        }
        Relation::LeastDistance => least(ctx, a, b, &sa, &sb),
        Relation::Clearance => {
            if a.kind != SelectionKind::Body || b.kind != SelectionKind::Body {
                return Err(refused(relation, "a clearance is between two bodies"));
            }
            let mut p = Map::new();
            p.insert("from".into(), a.feature.to_string().into());
            p.insert("to".into(), b.feature.to_string().into());
            let out = crate::verify_cmds::take(ctx, "clearance", &p)?;
            let least = out.detail.get("least");
            let point = |side: &str| -> Option<Vec3> {
                let v = least?.get(side)?.get("point")?.as_array()?;
                Some(Vec3::new(
                    v.first()?.as_f64()?,
                    v.get(1)?.as_f64()?,
                    v.get(2)?.as_f64()?,
                ))
            };
            let (from, to) = (point("from"), point("to"));
            Ok(Found {
                relation,
                value: out.number,
                words: "Clearance",
                from: from.unwrap_or(Vec3::ZERO),
                to: to.or(from).unwrap_or(Vec3::ZERO),
                at: None,
                parallel: None,
                at_least: out.lower,
                sampled: false,
                overlap: out.number.is_none(),
            })
        }
    }
}

/// The closest points of the line through `pa` along `da` and the one
/// through `pb` along `db` (unit directions, not parallel).
fn closest(pa: Vec3, da: Vec3, pb: Vec3, db: Vec3) -> (Vec3, Vec3) {
    let w = pa - pb;
    let (b, d, e) = (da.dot(db), da.dot(w), db.dot(w));
    let denominator = 1.0 - b * b;
    if !(denominator > 1e-15 && denominator.is_finite()) {
        let m = pa.lerp(pb, 0.5);
        return (foot(pa, da, m), foot(pb, db, m));
    }
    let s = (b * e - d) / denominator;
    let t = (e - b * d) / denominator;
    (pa + da * s, pb + db * t)
}

/// The least distance between two things as they are: from a vertex;
/// between two straight edges' segments; between faces (or bodies);
/// along an edge (a rim, a curve, a straight edge) to the other thing.
/// Never from a centre (the review of 2026-11-17: a rim measured from its
/// centre said 15 where it is 12).
fn least(ctx: &Ctx<'_>, a: &Thing, b: &Thing, sa: &Shape, sb: &Shape) -> Result<Found, String> {
    let words = "Least distance";
    let vertex = |t: &Thing, s: &Shape| -> Option<Vec3> {
        match (t.kind, s) {
            (SelectionKind::Vertex, Shape::Point { at, .. }) => Some(*at),
            _ => None,
        }
    };
    if let Some(p) = vertex(a, sa) {
        let (d, q, freeform) = from_point(ctx, p, b)?
            .ok_or_else(|| refused(Relation::LeastDistance, "nothing to measure to"))?;
        let mut found = Found::exact(Relation::LeastDistance, d, words, p, q);
        found.sampled = freeform;
        return Ok(found);
    }
    if let Some(p) = vertex(b, sb) {
        let (d, q, freeform) = from_point(ctx, p, a)?
            .ok_or_else(|| refused(Relation::LeastDistance, "nothing to measure from"))?;
        let mut found = Found::exact(Relation::LeastDistance, d, words, q, p);
        found.sampled = freeform;
        return Ok(found);
    }
    if let (Some((a0, a1)), Some((b0, b1))) = (segment(a), segment(b)) {
        let (p, q) = closest_segments(a0, a1, b0, b1);
        return Ok(Found::exact(
            Relation::LeastDistance,
            p.distance(q),
            "Least distance between the edges",
            p,
            q,
        ));
    }
    if let (Some(fa), Some(fb)) = (faces_of(a), faces_of(b)) {
        let pa = Part::new(&a.body, Some(&fa), &ctx.tolerance)?;
        let pb = Part::new(&b.body, Some(&fb), &ctx.tolerance)?;
        let near = verify::distance_to(&[pa], &[pb], ctx.stop)?;
        let mut found = Found::exact(
            Relation::LeastDistance,
            near.distance,
            words,
            near.a,
            near.b,
        );
        found.at_least = Some(near.lower);
        return Ok(found);
    }
    // An edge and something else: the least, along the edge, of the
    // distance to the other.
    let (edge, other, flipped) = if a.kind == SelectionKind::Edge {
        (a, b, false)
    } else if b.kind == SelectionKind::Edge {
        (b, a, true)
    } else {
        return Err(refused(Relation::LeastDistance, "nothing to measure"));
    };
    let (d, p, q, freeform) = along_edge(ctx, edge, other)?
        .ok_or_else(|| refused(Relation::LeastDistance, "nothing to measure to"))?;
    let (from, to) = if flipped { (q, p) } else { (p, q) };
    let mut found = Found::exact(Relation::LeastDistance, d, words, from, to);
    found.sampled = freeform;
    Ok(found)
}

/// How finely an edge is looked along for the least distance from it:
/// samples, then halvings of the step about the best.
const ALONG_SAMPLES: u32 = 96;
const ALONG_REFINE: u32 = 60;

/// The least distance from the edge `edge` to `other` (faces, a body or
/// an edge): sampled along the edge, then narrowed about the best sample
/// by golden sections; the distance, the point of the edge, the point of
/// the other, and whether either is freeform (then a least that may lie
/// between the samples).
fn along_edge(
    ctx: &Ctx<'_>,
    edge: &Thing,
    other: &Thing,
) -> Result<Option<(f64, Vec3, Vec3, bool)>, String> {
    let Some(curve) = edge.indices.first().and_then(|&e| edge.body.edges.get(e)) else {
        return Ok(None);
    };
    let probe = match faces_of(other) {
        Some(_) => Some(
            craft3d_kernel::boolean::probe::Probe::new(&other.body, &ctx.tolerance)
                .map_err(|e| e.to_string())?,
        ),
        None => None,
    };
    let faces = faces_of(other).unwrap_or_default();
    let other_edge = (other.kind == SelectionKind::Edge)
        .then(|| other.indices.first().and_then(|&e| other.body.edges.get(e)))
        .flatten();
    let mut freeform = matches!(curve.curve, Curve3::Nurbs(_));
    if let Some(e) = other_edge {
        freeform |= matches!(e.curve, Curve3::Nurbs(_));
    }
    for &f in &faces {
        if let Some(face) = other.body.faces.get(f) {
            freeform |= matches!(face.surface, Surface::Bezier(_) | Surface::Nurbs(_));
        }
    }
    // The nearest point of the other to `p`.
    let nearest = |p: Vec3| -> Result<Option<(f64, Vec3)>, String> {
        let mut best: Option<(f64, Vec3)> = None;
        if let Some(probe) = &probe {
            for &f in &faces {
                let q = verify::nearest_on_face(probe, f, p)?;
                let d = q.distance(p);
                if d.is_finite() && best.is_none_or(|(b, _)| d < b) {
                    best = Some((d, q));
                }
            }
        }
        if let Some(e) = other_edge
            && let Some((q, _)) = verify::nearest_on_edge(e, p)
        {
            let d = q.distance(p);
            if d.is_finite() && best.is_none_or(|(b, _)| d < b) {
                best = Some((d, q));
            }
        }
        Ok(best)
    };
    let (t0, t1) = curve.range;
    if !(t0.is_finite() && t1.is_finite()) {
        return Ok(None);
    }
    let at = |t: f64| curve.curve.eval(t);
    let mut best: Option<(f64, f64, Vec3, Vec3)> = None;
    for k in 0..=ALONG_SAMPLES {
        let t = t0 + (t1 - t0) * f64::from(k) / f64::from(ALONG_SAMPLES);
        let p = at(t);
        if let Some((d, q)) = nearest(p)?
            && best.is_none_or(|(b, ..)| d < b)
        {
            best = Some((d, t, p, q));
        }
    }
    let Some((mut d, t, mut p, mut q)) = best else {
        return Ok(None);
    };
    // Golden sections about the best sample, a step either side.
    let step = (t1 - t0) / f64::from(ALONG_SAMPLES);
    let (mut lo, mut hi) = ((t - step).max(t0.min(t1)), (t + step).min(t0.max(t1)));
    // The golden section, (√5 − 1) / 2.
    let g = 0.618_033_988_749_894_9;
    for _ in 0..ALONG_REFINE {
        let (x1, x2) = (hi - (hi - lo) * g, lo + (hi - lo) * g);
        let f = |x: f64| -> Result<f64, String> {
            Ok(nearest(at(x))?.map_or(f64::INFINITY, |(d, _)| d))
        };
        if f(x1)? < f(x2)? {
            hi = x2;
        } else {
            lo = x1;
        }
    }
    let t = 0.5 * (lo + hi);
    let point = at(t);
    if let Some((dd, qq)) = nearest(point)?
        && dd < d
    {
        (d, p, q) = (dd, point, qq);
    }
    Ok(Some((d, p, q, freeform)))
}

/// The measure `name` of an assertion or an output: `between`, a length
/// (any relation but an angle), or `angleBetween`, the angle.
pub(crate) fn take_measure(
    ctx: &Ctx<'_>,
    name: &str,
    p: &Map<String, Value>,
) -> Result<Outcome, String> {
    if name == "angleBetween" {
        only(p, &["from", "to"])?;
        let mut with = p.clone();
        with.insert("relation".into(), Relation::Angle.name().into());
        return take(ctx, &with);
    }
    if p.get("relation").and_then(Value::as_str) == Some(Relation::Angle.name()) {
        return Err("an angle between two things is the measure angleBetween".into());
    }
    let out = take(ctx, p)?;
    if out.detail.get("relation").and_then(Value::as_str) == Some(Relation::Angle.name()) {
        return Err(
            "these two make an angle, the measure angleBetween; between is a length".into(),
        );
    }
    Ok(out)
}

/// The measure itself, for a command, an assertion or an output.
pub(crate) fn take(ctx: &Ctx<'_>, p: &Map<String, Value>) -> Result<Outcome, String> {
    only(p, &["from", "to", "relation"])?;
    let from = p.get("from").ok_or("`from` is required: the first thing")?;
    let to = p.get("to").ok_or("`to` is required: the second thing")?;
    let wanted = match p.get("relation") {
        None | Some(Value::Null) => None,
        Some(Value::String(text)) => Some(Relation::parse(text)?),
        Some(_) => return Err("`relation` is a word: distance, angle, ...".into()),
    };
    let (a, b) = (thing(ctx, from, "from")?, thing(ctx, to, "to")?);
    let found = measure(ctx, &a, &b, wanted)?;
    let angle = found.relation == Relation::Angle;
    let dim = if angle {
        Dim::Of(craft3d_expr::Dimension::ANGLE)
    } else {
        Dim::Length(1)
    };
    let mut out = match found.value {
        Some(v) if v.is_finite() => Outcome::number(dim, v),
        Some(_) => return Err("the measure is not a number".into()),
        None => {
            let mut out = Outcome::number(dim, 0.0);
            out.number = None;
            out.upper = None;
            out.lower = None;
            out.note = Some("overlap".into());
            out
        }
    };
    if let Some(at_least) = found.at_least {
        out.lower = Some(at_least);
        out.detail.insert("atLeast".into(), at_least.into());
    }
    if found.sampled {
        out.sampled = Sampled::OpenBelow;
    }
    out.detail
        .insert("relation".into(), found.relation.name().into());
    out.detail.insert("words".into(), found.words.into());
    out.detail.insert("from".into(), vec3(found.from));
    out.detail.insert("to".into(), vec3(found.to));
    if let Some(at) = found.at {
        out.detail.insert("at".into(), vec3(at));
    }
    if let Some(parallel) = found.parallel {
        out.detail.insert("parallel".into(), parallel.into());
    }
    if angle {
        if let Some(v) = found.value {
            out.detail.insert("degrees".into(), v.to_degrees().into());
        }
        out.detail
            .insert("tolerance".into(), ctx.tolerance.angular.into());
    } else {
        out.detail
            .insert("tolerance".into(), ctx.tolerance.linear.into());
        if found.relation != Relation::Clearance {
            out.detail
                .insert("parts".into(), vec3(found.to - found.from));
        }
    }
    if found.overlap {
        out.detail.insert("overlap".into(), true.into());
    }
    Ok(out)
}

const ID: &str = "measure.between";

/// What `measure.between` answers for `map`, from the document `doc` and
/// what was derived from it.
fn answered(
    doc: &craft3d_doc::Document,
    regen: &craft3d_regen::Regen,
    placed: Option<&Arc<crate::asm::Placed>>,
    map: &Map<String, Value>,
) -> Result<Value, String> {
    let mut ctx = Ctx::new(doc, regen).map_err(|e| format!("{ID}: {e}"))?;
    ctx.placed = placed;
    let out = take(&ctx, map).map_err(|e| format!("{ID}: {e}"))?;
    let mut result = out.detail;
    result.insert("value".into(), out.number.into());
    result.insert("lengthUnit".into(), length_unit(doc).into());
    Ok(Value::Object(result))
}

fn measure_between(session: &mut Session, params: &Value) -> Run {
    Params::new(ID, params, &["from", "to", "relation"])?;
    let map = match params {
        Value::Object(map) => map.clone(),
        _ => Map::new(),
    };
    let open = session.document_mut()?;
    let regen = open.regen();
    let (doc, placed) = measured_document(open);
    answered(&doc, &regen, placed.as_ref(), &map).map_err(CommandError::new)
}

/// What a [`BetweenJob`] measures, and its answer once it is measured.
struct Asked {
    doc: Arc<craft3d_doc::Document>,
    regen: Arc<craft3d_regen::Regen>,
    placed: Option<Arc<crate::asm::Placed>>,
    map: Map<String, Value>,
    answer: std::sync::Mutex<Option<Result<Value, String>>>,
    done: std::sync::atomic::AtomicBool,
}

impl Asked {
    fn run(&self) {
        let answer = answered(&self.doc, &self.regen, self.placed.as_ref(), &self.map);
        *self
            .answer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(answer);
        self.done.store(true, std::sync::atomic::Ordering::Release);
    }
}

/// `measure.between` measured on a thread of its own, for the window (the
/// review of 2026-11-17: on the window's thread two gear bodies held each
/// new revision up about 180 ms): the document and its bodies as they
/// were when it was asked. Inline where there are no threads.
pub struct BetweenJob {
    asked: Arc<Asked>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl BetweenJob {
    /// The answer, `measure.between`'s or its error, once measured; `None`
    /// while it is being measured.
    pub fn answer(&self) -> Option<Result<Value, String>> {
        if !self.asked.done.load(std::sync::atomic::Ordering::Acquire) {
            // A thread that ended without an answer (it cannot, but a
            // window must never wait for one forever).
            if self.thread.as_ref().is_some_and(|t| t.is_finished()) {
                return Some(Err(format!("{ID}: not measured")));
            }
            return None;
        }
        self.asked
            .answer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

impl Session {
    /// `measure.between` with `params` on a thread of its own: refused at
    /// once for params it does not take or without a document, else
    /// measured while the caller goes on ([`BetweenJob::answer`]).
    pub fn between_later(&mut self, params: &Value) -> Result<BetweenJob, CommandError> {
        Params::new(ID, params, &["from", "to", "relation"])?;
        let map = match params {
            Value::Object(map) => map.clone(),
            _ => Map::new(),
        };
        let open = self.document_mut()?;
        let regen = open.regen();
        let (doc, placed) = measured_document(open);
        let asked = Arc::new(Asked {
            doc,
            regen,
            placed,
            map,
            answer: std::sync::Mutex::new(None),
            done: std::sync::atomic::AtomicBool::new(false),
        });
        #[cfg(not(target_arch = "wasm32"))]
        {
            let for_thread = Arc::clone(&asked);
            if let Ok(thread) = std::thread::Builder::new()
                .name("craft3d-between".into())
                .spawn(move || {
                    craft3d_math::pool::mark_background();
                    for_thread.run();
                })
            {
                return Ok(BetweenJob {
                    asked,
                    thread: Some(thread),
                });
            }
        }
        asked.run();
        Ok(BetweenJob {
            asked,
            thread: None,
        })
    }

    /// The active document's linear tolerance, in its length unit: what a
    /// length is shown to. `None` without a document.
    pub fn linear_tolerance(&self) -> Option<f64> {
        let doc = self.document().ok()?.history.current();
        Some(doc.tolerance.linear).filter(|t| t.is_finite() && *t > 0.0)
    }
}
