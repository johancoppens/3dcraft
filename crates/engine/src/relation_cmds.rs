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
use crate::selection::SelectionKind;
use crate::verify::{self, Part};
use crate::verify_cmds::{Ctx, Dim, Outcome, Sampled, feature_of, measured_document, only};
use crate::{CommandError, CommandSpec, Session};
use craft3d_doc::FeatureId;
use craft3d_geom::{Curve3, Surface};
use craft3d_math::{Portable, Vec3};
use craft3d_topo::Body;
use serde_json::{Map, Value};
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
    rims_parallel: bool,
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
        // Centres and axes: their centre distance (a hole's or a boss's
        // pitch); a circle at an angle to the other's axis, the least
        // distance as the things are (the review of 2026-11-17).
        (Point { centre: true, .. }, Point { centre: true, .. })
        | (Point { centre: true, .. }, Line { edge: false, .. })
        | (Line { edge: false, .. }, Point { centre: true, .. }) => {
            if rims_parallel {
                Relation::CenterDistance
            } else {
                Relation::LeastDistance
            }
        }
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
                    // By the item itself, as the selection finds it: never
                    // through a name pattern, whose length is capped (the
                    // review of 2026-11-17: a gear's faces have names of
                    // over 512 characters).
                    let name = map
                        .get("name")
                        .and_then(Value::as_str)
                        .filter(|n| !n.is_empty())
                        .ok_or_else(|| format!("`{key}` needs the persistent name"))?;
                    let feature = text(map.get("feature"), "the feature of its body")?;
                    let shown: String = name.chars().take(120).collect();
                    let (id, found, index) = crate::selection::Resolver::new(ctx.doc, ctx.regen)
                        .locate(name, Some(feature.as_str()))
                        .map_err(|e| {
                            if e.starts_with("no body has a") {
                                format!(
                                    "`{key}`: there is no {kind} `{shown}` in `{feature}` (see document.inspect)"
                                )
                            } else {
                                format!("`{key}`: {e}")
                            }
                        })?;
                    if found.name() != kind {
                        return Err(format!(
                            "`{key}`: `{shown}` is a {}, not a {kind}",
                            found.name()
                        ));
                    }
                    // A face a boolean cut into pieces that kept its name:
                    // every piece.
                    let mut indices = vec![index];
                    if found == SelectionKind::Face
                        && let Some(names) = ctx.regen.feature(id).and_then(|f| f.names.as_ref())
                        && let Some(own) = names.faces.get(index)
                    {
                        indices = (0..names.faces.len())
                            .filter(|&k| names.faces.get(k) == Some(own))
                            .collect();
                    }
                    Ok(Thing {
                        kind: found,
                        feature: id,
                        body: Arc::clone(ctx.body(id)?),
                        indices,
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
    // A circular edge's axis with the other's axis (a round face's, or
    // another circular edge's): whether a centre distance means a pitch.
    let axis_of = |t: &Thing, s: &Shape| match s {
        Line {
            direction,
            edge: false,
            ..
        } => Some(*direction),
        _ => circle_axis(t),
    };
    let rim_axes = match (circle_axis(a), circle_axis(b)) {
        (Some(n), _) => axis_of(b, &sb).map(|m| (n, m)),
        (None, Some(m)) => axis_of(a, &sa).map(|n| (n, m)),
        (None, None) => None,
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
    let relation = wanted.unwrap_or_else(|| {
        chosen(
            &sa,
            &sb,
            parallel_planes,
            parallel_axis_plane,
            edges,
            rim_axes.is_none_or(|(n, m)| along(n, m, angular)),
        )
    });
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
                        let same_way = da.dot(*db) >= 0.0;
                        let along_a = |x: f64| if same_way { x } else { -x };
                        let (lo_bb, hi_bb) = if same_way {
                            (*lo_b, *hi_b)
                        } else {
                            (-*hi_b, -*lo_b)
                        };
                        // Two edges: the middle of where they run side by
                        // side. Two holes: where they open, each at its
                        // rim on a face a person sees, whichever way its
                        // axis points (the review of 2026-11-17: at
                        // mid-depth, at the floors of holes drilled from
                        // above, inside the material for one from each
                        // side; round three: whichever was selected first,
                        // a top another body covers, slanted holes): the
                        // centres of the rims in view, a level both open
                        // at, else either's; the upper of two (by Z, then
                        // Y, then X: where a model is mostly seen from).
                        let level = if *ea {
                            let (from, to) = (lo_a.max(lo_bb), hi_a.min(hi_bb));
                            if from <= to {
                                (from + to) * 0.5
                            } else {
                                ma.lerp(*mb, 0.5).dot(*da)
                            }
                        } else {
                            let ends_a = openings(ctx, a, *ma, *da, (*lo_a, *hi_a));
                            let ends_b = openings(ctx, b, *mb, *db, (*lo_b, *hi_b));
                            let open_a: Vec<f64> = ends_a
                                .into_iter()
                                .filter_map(|(open, x)| open.then_some(x))
                                .collect();
                            let open_b: Vec<f64> = ends_b
                                .into_iter()
                                .filter_map(|(open, x)| open.then_some(along_a(x)))
                                .collect();
                            let tol = ctx.tolerance.linear;
                            let close = |x: f64, y: f64| (x - y).abs() <= tol.max(1e-9 * x.abs());
                            let common: Vec<f64> = open_a
                                .iter()
                                .copied()
                                .filter(|x| open_b.iter().any(|y| close(*x, *y)))
                                .collect();
                            // Of two levels, the upper (by Z, then Y, then
                            // X): a through hole opens at both ends, and
                            // the line goes where a model is mostly seen
                            // from, whichever thing came first.
                            let point = |x: f64| *ma + *da * (x - ma.dot(*da));
                            let upper = |xs: &[f64]| {
                                xs.iter()
                                    .copied()
                                    .max_by(|x, y| upper_order(point(*x), point(*y), tol))
                            };
                            let either: Vec<f64> = open_a.iter().chain(&open_b).copied().collect();
                            upper(&common)
                                .or_else(|| upper(&either))
                                .unwrap_or_else(|| hi_a.min(hi_bb))
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
                ) => (
                    *at,
                    foot(*middle, *direction, *at),
                    rim_axes.map(|(n, m)| along(n, m, angular)),
                ),
                (
                    Line {
                        direction, middle, ..
                    },
                    Point { at, .. },
                ) => (
                    foot(*middle, *direction, *at),
                    *at,
                    rim_axes.map(|(n, m)| along(n, m, angular)),
                ),
                (Point { at: p, .. }, Point { at: q, .. }) => {
                    (*p, *q, rim_axes.map(|(n, m)| along(n, m, angular)))
                }
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

/// The ends of the round face `t` (a hole's or a boss's wall), its axis
/// through `middle` along the unit `d`, reaching from `lo` to `hi` along
/// it: the far end first, then the near one, each with whether it opens to
/// the air in view and the level along `d` of its rim's centre. An end
/// opens where, just past it (a tenth of its radius), on the axis and near
/// the wall, lies outside the face's body and every other body shown (a
/// hole meets the air at its rim on a face a person sees; a hole's floor,
/// flat or a drill's cone, does not, nor a top another body covers). Its
/// rim: the face's edges wholly on that end's side of the middle (not a
/// seam), their points' mean along `d` (a slanted hole's rim is an
/// ellipse whose centre is on the axis, not its far tip); the end itself
/// where there are none. Closed where the bodies cannot tell.
fn openings(
    ctx: &Ctx<'_>,
    t: &Thing,
    middle: Vec3,
    d: Vec3,
    (lo, hi): (f64, f64),
) -> [(bool, f64); 2] {
    use craft3d_kernel::boolean::probe::{Place, Probe};
    let points = boundary(&t.body, &t.indices, 8);
    let at = |x: f64| middle + d * (x - middle.dot(d));
    // The rim's centre at the end `end` (of `lo` and `hi`).
    let half = 0.5 * (lo + hi);
    let rim = |high: bool| -> f64 {
        let mut sum = 0.0;
        let mut count = 0_u32;
        for &f in &t.indices {
            for edge in verify::face_edges(&t.body, f) {
                let (t0, t1) = edge.range;
                let samples: Vec<f64> = (0..16)
                    .map(|k| {
                        edge.curve
                            .eval(t0 + (t1 - t0) * (f64::from(k) + 0.5) / 16.0)
                            .dot(d)
                    })
                    .collect();
                let side = samples.iter().all(|x| x.is_finite() && (*x > half) == high);
                if side {
                    sum += samples.iter().sum::<f64>();
                    count += 16;
                }
            }
        }
        let centre = sum / f64::from(count.max(1));
        if count > 0 && centre.is_finite() {
            centre
        } else if high {
            hi
        } else {
            lo
        }
    };
    // From the axis to the wall at the end `end`: by the boundary's point
    // nearest that end.
    let radial = |end: f64| {
        points
            .iter()
            .filter(|p| p.is_finite())
            .min_by(|p, q| (p.dot(d) - end).abs().total_cmp(&(q.dot(d) - end).abs()))
            .map(|p| *p - at(p.dot(d)))
    };
    let own = Probe::new(&t.body, &ctx.tolerance).ok();
    // The other bodies shown, each with its box; probed only where a point
    // lies in that box.
    let others: Vec<(craft3d_math::Bbox3, &Arc<Body>)> = ctx
        .regen
        .bodies()
        .filter(|(id, _)| *id != t.feature && !ctx.doc.is_hidden(*id))
        .filter_map(|(_, body)| Some((body.bbox().ok()?, body)))
        .collect();
    let margin = 10.0 * ctx.tolerance.linear;
    let outside = |p: Vec3| {
        let Some(own) = &own else {
            return false;
        };
        p.is_finite()
            && matches!(own.place(p), Ok(Place::Outside))
            && others.iter().all(|(bounds, body)| {
                !bounds.contains(p, margin)
                    || Probe::new(body, &ctx.tolerance)
                        .is_ok_and(|probe| matches!(probe.place(p), Ok(Place::Outside)))
            })
    };
    let open = |end: f64, sign: f64| {
        let Some(r) = radial(end) else {
            return false;
        };
        let step = (0.1 * r.length()).max(10.0 * ctx.tolerance.linear);
        let past = at(end + sign * step);
        outside(past) && outside(past + r * 0.95)
    };
    [(open(hi, 1.0), rim(true)), (open(lo, -1.0), rim(false))]
}

/// Which of two places is the upper: by Z, then Y, then X, each within
/// `tol` taken as level (where a model is mostly seen from).
fn upper_order(p: Vec3, q: Vec3, tol: f64) -> std::cmp::Ordering {
    let by = |u: f64, v: f64| {
        if (u - v).abs() <= tol {
            std::cmp::Ordering::Equal
        } else {
            u.total_cmp(&v)
        }
    };
    by(p.z, q.z).then(by(p.y, q.y)).then(by(p.x, q.x))
}

/// Where the round face `face` of the body of `feature` (a hole's or a
/// boss's wall) opens in view: the centre of its rim at an end that meets
/// the air past every body shown, as two holes' line is drawn
/// ([`openings`]); the upper of two. `None` for a face that is not round
/// or opens nowhere in view. Where the window draws one hole's diameter
/// (the review of 2026-11-17: drawn at mid-depth it floated on the top
/// face below the opening).
pub(crate) fn opening_of(
    doc: &craft3d_doc::Document,
    regen: &craft3d_regen::Regen,
    feature: FeatureId,
    face: usize,
) -> Option<Vec3> {
    let ctx = Ctx::new(doc, regen).ok()?;
    let body = Arc::clone(ctx.body(feature).ok()?);
    let t = Thing {
        kind: SelectionKind::Face,
        feature,
        body,
        indices: vec![face],
    };
    let Shape::Line {
        direction,
        middle,
        ends,
        edge: false,
    } = shape_of(&t)
    else {
        return None;
    };
    let tol = ctx.tolerance.linear;
    let at = |x: f64| middle + direction * (x - middle.dot(direction));
    openings(&ctx, &t, middle, direction, ends)
        .into_iter()
        .filter(|&(open, _)| open)
        .map(|(_, x)| at(x))
        .filter(|p| p.is_finite())
        .max_by(|p, q| upper_order(*p, *q, tol))
}

/// The unit axis of a circular edge (its circle's normal); `None` for
/// anything else.
fn circle_axis(t: &Thing) -> Option<Vec3> {
    if t.kind != SelectionKind::Edge {
        return None;
    }
    match &t.indices.first().and_then(|&e| t.body.edges.get(e))?.curve {
        Curve3::Circle(circle) => circle.frame().z().normalized(),
        _ => None,
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
        // From a point the nearest place is exact: proven, but on freeform.
        found.at_least = (!freeform).then_some(d);
        return Ok(found);
    }
    if let Some(p) = vertex(b, sb) {
        let (d, q, freeform) = from_point(ctx, p, a)?
            .ok_or_else(|| refused(Relation::LeastDistance, "nothing to measure from"))?;
        let mut found = Found::exact(Relation::LeastDistance, d, words, q, p);
        found.sampled = freeform;
        found.at_least = (!freeform).then_some(d);
        return Ok(found);
    }
    if let (Some((a0, a1)), Some((b0, b1))) = (segment(a), segment(b)) {
        let (p, q) = closest_segments(a0, a1, b0, b1);
        let mut found = Found::exact(
            Relation::LeastDistance,
            p.distance(q),
            "Least distance between the edges",
            p,
            q,
        );
        found.at_least = Some(found.value.unwrap_or(0.0));
        return Ok(found);
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
    let along = along_edge(ctx, edge, other)?
        .ok_or_else(|| refused(Relation::LeastDistance, "nothing to measure to"))?;
    let (from, to) = if flipped {
        (along.q, along.p)
    } else {
        (along.p, along.q)
    };
    let mut found = Found::exact(Relation::LeastDistance, along.d, words, from, to);
    found.at_least = along.at_least;
    found.sampled = along.freeform || along.at_least.is_none();
    Ok(found)
}

/// How finely an edge is looked along first: samples, then golden
/// sections about the best; at most this many pieces of an edge, pairs of
/// a piece and a triangle, and pieces narrowed exactly in the proof.
const ALONG_SAMPLES: u32 = 96;
const ALONG_REFINE: u32 = 60;
const ALONG_PIECES: usize = 2048;
const ALONG_PAIRS: u64 = 4_000_000;
const ALONG_NARROWED: usize = 48;
/// Most comparisons of a piece and a triangle in tightening an arc's
/// bound, and most pieces waiting to be halved.
const ALONG_TIGHTEN: u64 = 2_000_000;
const ALONG_HALVES: usize = 8192;

/// The least distance from an edge to another thing, as `along_edge`
/// finds it.
struct Along {
    /// Between two real places: the point of the edge and of the other.
    d: f64,
    p: Vec3,
    q: Vec3,
    /// No less than this, proven (from the edge's pieces and the other's
    /// triangles, each with how far the exact thing can be from it); none
    /// where it could not be had (a freeform curve, too much to compare).
    at_least: Option<f64>,
    /// Either thing is freeform: what is proven rests on samples.
    freeform: bool,
}

/// The point of the triangle `t` nearest `p` (the regions of its corners,
/// sides and inside in turn); a degenerate one by its corners and sides.
fn on_triangle(p: Vec3, t: &[Vec3; 3]) -> Vec3 {
    let [a, b, c] = *t;
    let (ab, ac, ap) = (b - a, c - a, p - a);
    let (d1, d2) = (ab.dot(ap), ac.dot(ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = p - b;
    let (d3, d4) = (ab.dot(bp), ac.dot(bp));
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 && d1 - d3 != 0.0 {
        return a + ab * (d1 / (d1 - d3));
    }
    let cp = p - c;
    let (d5, d6) = (ab.dot(cp), ac.dot(cp));
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 && d2 - d6 != 0.0 {
        return a + ac * (d2 / (d2 - d6));
    }
    let va = d3 * d6 - d5 * d4;
    let (e, f) = (d4 - d3, d5 - d6);
    if va <= 0.0 && e >= 0.0 && f >= 0.0 && e + f != 0.0 {
        return b + (c - b) * (e / (e + f));
    }
    let sum = va + vb + vc;
    let inside = a + ab * (vb / sum) + ac * (vc / sum);
    if sum != 0.0 && inside.is_finite() {
        return inside;
    }
    // Degenerate: the nearest of its sides.
    let mut best = a;
    for (x, y) in [(a, b), (b, c), (c, a)] {
        let (q, _) = closest_segments(p, p, x, y);
        if q.distance(p) < best.distance(p) {
            best = q;
        }
    }
    best
}

/// The distance between the segment from `a0` to `a1` and the triangle
/// `t` (0 where it passes through it), and where along the segment it is
/// least (0 at `a0`, 1 at `a1`).
fn segment_to_triangle(a0: Vec3, a1: Vec3, t: &[Vec3; 3]) -> (f64, f64) {
    let [a, b, c] = *t;
    let n = (b - a).cross(c - a);
    let (s0, s1) = ((a0 - a).dot(n), (a1 - a).dot(n));
    if n.length_squared() > 0.0 && s0 * s1 <= 0.0 && s0 != s1 {
        let along = s0 / (s0 - s1);
        let x = a0 + (a1 - a0) * along;
        if on_triangle(x, t).distance(x) <= 1e-12 * (1.0 + x.length()) {
            return (0.0, along);
        }
    }
    let span = a1 - a0;
    let fraction = |p: Vec3| {
        let f = (p - a0).dot(span) / span.length_squared();
        if f.is_finite() {
            f.clamp(0.0, 1.0)
        } else {
            0.0
        }
    };
    let mut best = (on_triangle(a0, t).distance(a0), 0.0);
    let at_end = on_triangle(a1, t).distance(a1);
    if at_end < best.0 {
        best = (at_end, 1.0);
    }
    for (x, y) in [(a, b), (b, c), (c, a)] {
        let (p, q) = closest_segments(a0, a1, x, y);
        let d = p.distance(q);
        if d < best.0 {
            best = (d, fraction(p));
        }
    }
    best
}

/// The box of the points `points`.
fn box_of(points: &[Vec3]) -> craft3d_math::Bbox3 {
    craft3d_math::Bbox3::from_points(points.iter().copied())
}

/// How far apart two boxes are (0 where they meet).
fn boxes_apart(a: &craft3d_math::Bbox3, b: &craft3d_math::Bbox3) -> f64 {
    match (a.corners(), b.corners()) {
        (Some((alo, ahi)), Some((blo, bhi))) => {
            let gap = |l0: f64, h0: f64, l1: f64, h1: f64| (l1 - h0).max(l0 - h1).max(0.0);
            let (x, y, z) = (
                gap(alo.x, ahi.x, blo.x, bhi.x),
                gap(alo.y, ahi.y, blo.y, bhi.y),
                gap(alo.z, ahi.z, blo.z, bhi.z),
            );
            (x * x + y * y + z * z).sqrt()
        }
        _ => f64::INFINITY,
    }
}

/// The edge `edge` as pieces of a line: the parameters where each starts
/// and ends, and how far the edge can be from them; `true` with it when
/// that far is proven (a straight edge: exact; an arc: its sagitta), not
/// estimated (other curves).
fn pieces_of(edge: &craft3d_topo::Edge, chord: f64) -> (Vec<f64>, f64, bool) {
    let (t0, t1) = edge.range;
    match &edge.curve {
        Curve3::Line(_) => {
            // Exact in one; in pieces of eight chords (at most 64), so each
            // has its own place to narrow from.
            let length = (edge.curve.eval(t1) - edge.curve.eval(t0)).length();
            let n = (length / (8.0 * chord)).ceil();
            let n = if n.is_finite() && n >= 1.0 {
                (n as usize).min(64)
            } else {
                1
            };
            let step = (t1 - t0) / n as f64;
            ((0..=n).map(|k| t0 + step * k as f64).collect(), 0.0, true)
        }
        Curve3::Circle(circle) => {
            let r = circle.radius();
            let span = (t1 - t0).abs();
            // Pieces short enough that their sagitta is within a quarter of
            // the chord.
            let sagitta = 0.25 * chord;
            let most = if r > 0.0 && sagitta > 0.0 && sagitta < r {
                2.0 * (1.0 - sagitta / r).clamp(-1.0, 1.0).pacos()
            } else {
                span / 8.0
            };
            let n = if most.is_finite() && most > 0.0 {
                ((span / most).ceil() as usize).clamp(8, ALONG_PIECES)
            } else {
                64
            };
            let step = (t1 - t0) / n as f64;
            let sagitta = r * (1.0 - (0.5 * step.abs()).pcos());
            (
                (0..=n).map(|k| t0 + step * k as f64).collect(),
                sagitta.max(0.0),
                true,
            )
        }
        _ => {
            let n = 512_usize;
            let ts: Vec<f64> = (0..=n)
                .map(|k| t0 + (t1 - t0) * k as f64 / n as f64)
                .collect();
            // Estimated: the farthest a piece's middle lies from its chord,
            // twice.
            let mut far = 0.0_f64;
            for pair in ts.windows(2) {
                if let [a, b] = *pair {
                    let (pa, pb, pm) = (
                        edge.curve.eval(a),
                        edge.curve.eval(b),
                        edge.curve.eval(0.5 * (a + b)),
                    );
                    let (_, on) = closest_segments(pm, pm, pa, pb);
                    far = far.max(on.distance(pm));
                }
            }
            (ts, 2.0 * far, false)
        }
    }
}

/// The least distance from the edge `edge` to `other` (faces, a body or
/// an edge). Found first by samples along the edge narrowed by golden
/// sections; then proven, as `measure.distance` proves its own: the edge
/// in pieces of a line (each within its sagitta of the arc) against the
/// other's triangles (each within the mesh's error of its face) bound the
/// least from below; every piece whose bound lies under what was found is
/// narrowed on the exact geometry, so a narrow least the samples stepped
/// over (a thin pin beside a wall: the review of 2026-11-17) is found.
/// `None` when there is nothing to measure.
fn along_edge(ctx: &Ctx<'_>, edge: &Thing, other: &Thing) -> Result<Option<Along>, String> {
    let Some(curve) = edge.indices.first().and_then(|&e| edge.body.edges.get(e)) else {
        return Ok(None);
    };
    let faces = faces_of(other).unwrap_or_default();
    let part = if faces.is_empty() {
        None
    } else {
        Some(Part::new(&other.body, Some(&faces), &ctx.tolerance)?)
    };
    let other_edge = (other.kind == SelectionKind::Edge)
        .then(|| other.indices.first().and_then(|&e| other.body.edges.get(e)))
        .flatten();
    let mut freeform = matches!(curve.curve, Curve3::Nurbs(_) | Curve3::Ellipse(_));
    if let Some(e) = other_edge {
        freeform |= matches!(e.curve, Curve3::Nurbs(_) | Curve3::Ellipse(_));
    }
    // The other's freeform faces: what is found near them rests on
    // samples (those far from the least found do not matter).
    let freeform_faces: std::collections::BTreeSet<usize> =
        faces
            .iter()
            .copied()
            .filter(|&f| {
                other.body.faces.get(f).is_some_and(|face| {
                    matches!(face.surface, Surface::Bezier(_) | Surface::Nurbs(_))
                })
            })
            .collect();
    // The nearest point of the other to `p`, over `near` of its faces (all
    // when `None`) and its edge; a face whose nearest point cannot be had
    // is left to the proof, whose bound then says how near it may come.
    let nearest = |p: Vec3, near: Option<&[usize]>| -> Result<Option<(f64, Vec3)>, String> {
        let mut best: Option<(f64, Vec3)> = None;
        if let Some(part) = &part {
            for &f in near.unwrap_or(&faces) {
                let Ok(q) = verify::nearest_on_face(&part.probe, f, p) else {
                    continue;
                };
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
    // Narrowed by golden sections between `lo` and `hi`: the least found.
    let narrow =
        |lo: f64, hi: f64, near: Option<&[usize]>| -> Result<Option<(f64, Vec3, Vec3)>, String> {
            // The golden section, (√5 − 1) / 2.
            let g = 0.618_033_988_749_894_9;
            let (mut lo, mut hi) = (lo.min(hi), lo.max(hi));
            let d = |x: f64| -> Result<f64, String> {
                Ok(nearest(at(x), near)?.map_or(f64::INFINITY, |(d, _)| d))
            };
            for _ in 0..ALONG_REFINE {
                let (x1, x2) = (hi - (hi - lo) * g, lo + (hi - lo) * g);
                if d(x1)? < d(x2)? {
                    hi = x2;
                } else {
                    lo = x1;
                }
            }
            let t = 0.5 * (lo + hi);
            let p = at(t);
            Ok(nearest(p, near)?.map(|(d, q)| (d, p, q)))
        };
    // The least found so far, kept where `found` is less.
    fn keep(best: &mut Option<(f64, Vec3, Vec3)>, found: Option<(f64, Vec3, Vec3)>) {
        if let Some((d, p, q)) = found
            && d.is_finite()
            && best.is_none_or(|(b, ..)| d < b)
        {
            *best = Some((d, p, q));
        }
    }
    let mut best: Option<(f64, Vec3, Vec3)> = None;
    let mut first: Option<(f64, f64)> = None;
    for k in 0..=ALONG_SAMPLES {
        let t = t0 + (t1 - t0) * f64::from(k) / f64::from(ALONG_SAMPLES);
        let p = at(t);
        if let Some((d, q)) = nearest(p, None)? {
            if first.is_none_or(|(b, _)| d < b) {
                first = Some((d, t));
            }
            keep(&mut best, Some((d, p, q)));
        }
    }
    let Some((_, t)) = first else {
        return Ok(None);
    };
    let step = (t1 - t0) / f64::from(ALONG_SAMPLES);
    keep(
        &mut best,
        narrow(
            (t - step).clamp(t0.min(t1), t0.max(t1)),
            (t + step).clamp(t0.min(t1), t0.max(t1)),
            None,
        )?,
    );
    // The proof: the edge's pieces against the other's triangles.
    let chord = part.as_ref().map_or(ctx.tolerance.linear, |p| p.chord);
    let (ts, edge_err, edge_proven) = pieces_of(curve, chord);
    let pieces: Vec<(f64, f64, Vec3, Vec3)> = ts
        .windows(2)
        .filter_map(|w| match *w {
            [a, b] => Some((a, b, at(a), at(b))),
            _ => None,
        })
        .collect();
    let other_pieces: Vec<(Vec3, Vec3)> = match other_edge {
        Some(e) => {
            let (ots, _, _) = pieces_of(e, chord);
            ots.windows(2)
                .filter_map(|w| match *w {
                    [a, b] => Some((e.curve.eval(a), e.curve.eval(b))),
                    _ => None,
                })
                .collect()
        }
        None => Vec::new(),
    };
    let other_err = other_edge.map_or(0.0, |e| pieces_of(e, chord).1);
    let upper = |best: &Option<(f64, Vec3, Vec3)>| best.map_or(f64::INFINITY, |(d, ..)| d);
    let reach = pieces
        .iter()
        .map(|(_, _, a, b)| a.length().max(b.length()))
        .fold(1.0_f64, f64::max);
    let slack = 16.0 * f64::EPSILON * reach;
    let mut work = 0_u64;
    let mut abandoned = false;
    // Per piece: its least bound, the faces near enough to matter, and
    // where along it (0 to 1) the triangle or piece that bounds it is
    // nearest: where its narrowing starts.
    let mut bounds: Vec<(f64, usize, Vec<usize>, f64)> = Vec::new();
    let limit = upper(&best) + 2.0 * chord;
    for (i, (_, _, a, b)) in pieces.iter().enumerate() {
        let piece_box = box_of(&[*a, *b]);
        let mut least = f64::INFINITY;
        let mut seed = 0.5;
        let mut near: Vec<usize> = Vec::new();
        if let Some(part) = &part {
            for &f in &faces {
                let Some(face_box) = part.boxes.get(&f) else {
                    continue;
                };
                if boxes_apart(&piece_box, face_box) > limit + edge_err {
                    continue;
                }
                near.push(f);
                let err = part.err(f);
                for t in part.triangles.get(&f).map(Vec::as_slice).unwrap_or(&[]) {
                    if boxes_apart(&piece_box, &box_of(t)) > limit + edge_err + err {
                        continue;
                    }
                    work = work.saturating_add(1);
                    if work > ALONG_PAIRS {
                        abandoned = true;
                        break;
                    }
                    let (d, along) = segment_to_triangle(*a, *b, t);
                    let d = d - edge_err - err - slack;
                    if d < least {
                        (least, seed) = (d, along);
                    }
                }
                if abandoned {
                    break;
                }
            }
        }
        for (x, y) in &other_pieces {
            let (p, q) = closest_segments(*a, *b, *x, *y);
            let d = p.distance(q) - edge_err - other_err - slack;
            if d < least {
                let span = *b - *a;
                let along = (p - *a).dot(span) / span.length_squared();
                (least, seed) = (d, if along.is_finite() { along } else { 0.5 });
            }
        }
        if abandoned {
            break;
        }
        bounds.push((least, i, near, seed));
    }
    // Every piece whose bound lies under what was found, the lowest first:
    // narrowed on the exact geometry, near its own faces: about where its
    // bound is nearest (a narrow least, a thin pin, lies there), then over
    // the whole piece.
    bounds.sort_by(|x, y| x.0.total_cmp(&y.0));
    for (least, i, near, seed) in bounds.iter().take(ALONG_NARROWED) {
        if *least >= upper(&best) - ctx.tolerance.linear {
            break;
        }
        if let Some((a, b, pa, pb)) = pieces.get(*i) {
            let near = (!near.is_empty()).then_some(near.as_slice());
            let length = pa.distance(*pb);
            let window = if length > 0.0 {
                (2.0 * chord / length).clamp(0.0, 0.5)
            } else {
                0.5
            };
            let seed = seed.clamp(0.0, 1.0);
            let at = |f: f64| *a + (*b - *a) * f.clamp(0.0, 1.0);
            keep(
                &mut best,
                narrow(at(seed - window), at(seed + window), near)?,
            );
            keep(&mut best, narrow(*a, *b, near)?);
        }
    }
    let Some((d, p, q)) = best else {
        return Ok(None);
    };
    // A freeform face of the other near where the least may lie.
    freeform |= bounds
        .iter()
        .any(|(least, _, near, _)| *least <= d && near.iter().any(|f| freeform_faces.contains(f)));
    // Tightening an arc's bound (the review of 2026-11-17, round three: a
    // rim to a straight edge read 26.248806 – 26.248809, a rim to a block
    // at 1e6 51999999.999998 – 52000000): a piece whose bound lies under
    // the least found by more than a quarter of the tolerance, and only
    // its sagitta holds it there, is halved and its halves bounded again
    // (a quarter of the sagitta each), until each is within that quarter
    // or what holds it is the other's mesh (a curved face's error, which
    // the faces' own proof below settles), in bounded work. Exact against
    // flat faces bounded by lines and against straight edges: a circle
    // against a plane or a line comes within the tolerance.
    let goal = 0.25 * ctx.tolerance.linear;
    let mut by_pieces = (!abandoned && edge_proven)
        .then(|| bounds.iter().map(|(least, ..)| *least).fold(d, f64::min));
    if let (Curve3::Circle(circle), Some(_)) = (&curve.curve, by_pieces) {
        let r = circle.radius();
        let reach = d + chord;
        // The least, less the faces' errors, of a piece of a line to the
        // other's triangles of `near` and to the other's pieces; no more
        // than `reach` (triangles farther are left out).
        let raw_of = |pa: Vec3, pb: Vec3, near: &[usize], work: &mut u64| -> f64 {
            let piece_box = box_of(&[pa, pb]);
            let mut least = reach;
            if let Some(part) = &part {
                for &f in near {
                    let err = part.err(f);
                    for t in part.triangles.get(&f).map(Vec::as_slice).unwrap_or(&[]) {
                        if boxes_apart(&piece_box, &box_of(t)) > reach + err {
                            continue;
                        }
                        *work = work.saturating_add(1);
                        least = least.min(segment_to_triangle(pa, pb, t).0 - err);
                    }
                }
            }
            for (x, y) in &other_pieces {
                let (u, v) = closest_segments(pa, pb, *x, *y);
                least = least.min(u.distance(v) - other_err);
            }
            least
        };
        let mut kept = d;
        let mut work = 0_u64;
        let mut queue: Vec<(f64, f64, usize)> = Vec::new();
        for (k, (least, i, _, _)) in bounds.iter().enumerate() {
            match pieces.get(*i) {
                Some((ta, tb, ..)) if *least < d - goal => queue.push((*ta, *tb, k)),
                _ => kept = kept.min(*least),
            }
        }
        while let Some((ta, tb, k)) = queue.pop() {
            let near = bounds
                .get(k)
                .map_or(&[][..], |(_, _, near, _)| near.as_slice());
            let mid = 0.5 * (ta + tb);
            for (x, y) in [(ta, mid), (mid, tb)] {
                let sagitta = (r * (1.0 - (0.5 * (y - x).abs()).pcos())).max(0.0);
                let raw = raw_of(at(x), at(y), near, &mut work) - slack;
                let bound = raw - sagitta;
                let settled = bound >= d - goal;
                // Held by the mesh, not the sagitta (the arc lies within
                // its sagitta of the chord, either side); or out of work.
                let held = raw + sagitta < d - goal || sagitta <= 0.0;
                if settled || held || work > ALONG_TIGHTEN || queue.len() >= ALONG_HALVES {
                    kept = kept.min(bound);
                } else {
                    queue.push((x, y, k));
                }
            }
        }
        by_pieces = Some(kept);
    }
    // What the faces the edge bounds prove: the edge lies on each, so the
    // least distance from any of them to the other, proven on the exact
    // faces as `measure.distance` proves its own, is no more than the
    // edge's (tight where the face comes nearest at the edge, as a rim's
    // faces do; the mesh's bound is a chord off).
    let theirs_made;
    let theirs = match other_edge {
        Some(_) => {
            let around = faces_around(&other.body, other.indices.first().copied());
            theirs_made = Part::new(&other.body, Some(&around), &ctx.tolerance).ok();
            theirs_made.as_ref()
        }
        None => part.as_ref(),
    };
    let mut by_faces: Option<f64> = None;
    if let Some(theirs) = theirs {
        for f in faces_around(&edge.body, edge.indices.first().copied()) {
            let Ok(mine) = Part::new(&edge.body, Some(&[f]), &ctx.tolerance) else {
                continue;
            };
            if let Ok(near) = verify::distance_to(
                std::slice::from_ref(&mine),
                std::slice::from_ref(theirs),
                ctx.stop,
            ) && near.lower.is_finite()
            {
                by_faces = Some(by_faces.map_or(near.lower, |b: f64| b.max(near.lower)));
            }
        }
    }
    let at_least = match (by_pieces, by_faces) {
        (Some(x), Some(y)) => Some(x.max(y)),
        (x, y) => x.or(y),
    }
    .map(|l| l.clamp(0.0, d));
    Ok(Some(Along {
        d,
        p,
        q,
        at_least,
        freeform: freeform || !edge_proven || abandoned,
    }))
}

/// The faces of `body` the edge `edge` bounds, each once.
fn faces_around(body: &Body, edge: Option<usize>) -> Vec<usize> {
    let Some(edge) = edge else {
        return Vec::new();
    };
    let mut faces: Vec<usize> = craft3d_topo::edge_faces(body)
        .get(edge)
        .map(|fs| {
            fs.iter()
                .filter_map(|f| usize::try_from(f.0).ok())
                .collect()
        })
        .unwrap_or_default();
    faces.sort_unstable();
    faces.dedup();
    faces
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
        // Searched on samples (a freeform curve or face, or no bound to be
        // had): the answer says so. A proven bound below still bounds it,
        // so an assertion kept from it can fail (the review of 2026-11-17,
        // round three); without one the side below is open.
        if found.at_least.is_none() {
            out.sampled = Sampled::OpenBelow;
        }
        out.detail.insert("sampled".into(), true.into());
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
