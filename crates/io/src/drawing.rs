//! Flat outlines for DXF and SVG: the curves of a sketch, or the loops of
//! a flat face, as the paths of `craft3d-drawing`.
//!
//! Every curve stays what it is: a line a line, a circular arc an arc
//! with its centre and radius, an ellipse an ellipse, a NURBS edge a
//! spline. Nothing is turned into a polyline.

use crate::IoError;
use craft3d_doc::{EntityId, Geometry, Sketch, clamped_uniform_knots};
use craft3d_drawing::{Drawing, Path, Point, Segment, Spline, SvgOptions, SvgReport};
use craft3d_geom::{Curve3, Surface};
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_topo::{Body, Coedge, FaceId};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrawingFormat {
    Dxf,
    Svg,
}

impl DrawingFormat {
    /// The format for a file extension (without the dot, any case).
    pub fn from_extension(extension: &str) -> Option<Self> {
        match extension.to_ascii_lowercase().as_str() {
            "dxf" => Some(DrawingFormat::Dxf),
            "svg" => Some(DrawingFormat::Svg),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            DrawingFormat::Dxf => "dxf",
            DrawingFormat::Svg => "svg",
        }
    }
}

/// Encodes a drawing as a file. For SVG, `tolerance` is how far the cubic
/// Béziers may lie from a spline SVG cannot hold exactly (a degree above
/// 3, or rational), and the report says how many there were; DXF holds
/// every curve as itself.
pub fn encode_drawing(
    drawing: &Drawing,
    format: DrawingFormat,
    tolerance: f64,
) -> Result<(Vec<u8>, Option<SvgReport>), IoError> {
    let unsupported = |e: craft3d_drawing::DrawingError| IoError::Unsupported {
        body: String::new(),
        what: e.to_string(),
    };
    match format {
        DrawingFormat::Dxf => craft3d_drawing::write_dxf(drawing)
            .map(|text| (text.into_bytes(), None))
            .map_err(unsupported),
        DrawingFormat::Svg => {
            let options = SvgOptions {
                tolerance,
                ..SvgOptions::default()
            };
            craft3d_drawing::write_svg(drawing, &options)
                .map(|(text, report)| (text.into_bytes(), Some(report)))
                .map_err(unsupported)
        }
    }
}

fn unsupported(what: impl Into<String>) -> IoError {
    IoError::Unsupported {
        body: String::new(),
        what: what.into(),
    }
}

// ----------------------------------------------------------------- sketch

/// The curves of a sketch as paths.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct SketchOutline {
    /// The curves that are not construction geometry, joined into chains.
    pub paths: Vec<Path>,
    /// The construction curves, when asked for.
    pub construction: Vec<Path>,
    /// Lines and arcs left out because their two ends are one point.
    pub degenerate: Vec<EntityId>,
}

/// One curve of a sketch between two nodes (a node is one or more points
/// at the same place).
struct Link {
    entity: EntityId,
    from: usize,
    to: usize,
    curve: LinkCurve,
}

enum LinkCurve {
    Line,
    /// Counter-clockwise from `from` to `to`.
    Arc {
        center: Point,
    },
    /// From its first control point to its last.
    Spline(Spline),
}

/// The paths of a sketch: its lines, arcs and circles in the sketch's own
/// axes, at the solved positions `point` and radii `radius` give.
///
/// Curves that meet in a point (the same point entity, or two points
/// within `tolerance`) are joined into one chain as long as exactly two
/// meet there; a chain that comes back to its start is closed. Chains
/// come in the order of their first entity, and run the way that entity
/// was drawn. Construction geometry is left out unless `construction`,
/// and then it comes apart from the rest.
pub fn sketch_outline(
    sketch: &Sketch,
    point: &dyn Fn(EntityId) -> Option<(f64, f64)>,
    radius: &dyn Fn(EntityId) -> Option<f64>,
    construction: bool,
    tolerance: f64,
) -> Result<SketchOutline, IoError> {
    if !(tolerance.is_finite() && tolerance > 0.0) {
        return Err(unsupported("the tolerance is not positive and finite"));
    }
    let position = |id: EntityId| -> Result<Point, IoError> {
        let (x, y) = point(id)
            .or_else(|| match sketch.entity(id).ok()?.geometry {
                Geometry::Point { x, y } => Some((x, y)),
                _ => None,
            })
            .ok_or_else(|| unsupported(format!("the sketch has no point {id}")))?;
        if x.is_finite() && y.is_finite() {
            Ok([x, y])
        } else {
            Err(unsupported(format!("the point {id} is not finite")))
        }
    };
    let mut outline = SketchOutline::default();
    for wanted in [false, true] {
        if wanted && !construction {
            continue;
        }
        let mut nodes = Nodes::new(tolerance);
        let mut links = Vec::new();
        // (index of the first entity, path): to keep the order drawn.
        let mut paths: Vec<(usize, Path)> = Vec::new();
        for (index, entity) in sketch.entities.iter().enumerate() {
            if entity.construction != wanted {
                continue;
            }
            // Every kind of entity is named here: a new one does not
            // compile until it is mapped.
            match &entity.geometry {
                Geometry::Point { .. } | Geometry::Image { .. } => {}
                &Geometry::Line { start, end } => {
                    let (from, to) = (
                        nodes.at(start, position(start)?),
                        nodes.at(end, position(end)?),
                    );
                    links.push((
                        index,
                        Link {
                            entity: entity.id,
                            from,
                            to,
                            curve: LinkCurve::Line,
                        },
                    ));
                }
                &Geometry::Circle {
                    center,
                    radius: stored,
                } => {
                    let r = radius(entity.id).unwrap_or(stored);
                    if !(r.is_finite() && r > 0.0) {
                        return Err(unsupported(format!(
                            "the circle {} has no positive radius",
                            entity.id
                        )));
                    }
                    paths.push((
                        index,
                        Path::Circle {
                            center: position(center)?,
                            radius: r,
                        },
                    ));
                }
                &Geometry::Arc { center, start, end } => {
                    let (from, to) = (
                        nodes.at(start, position(start)?),
                        nodes.at(end, position(end)?),
                    );
                    links.push((
                        index,
                        Link {
                            entity: entity.id,
                            from,
                            to,
                            curve: LinkCurve::Arc {
                                center: position(center)?,
                            },
                        },
                    ));
                }
                Geometry::Spline { degree, control } => {
                    let not_a_curve =
                        || unsupported(format!("the spline {} is not a curve", entity.id));
                    let (Some(&first), Some(&last)) = (control.first(), control.last()) else {
                        return Err(not_a_curve());
                    };
                    let points = control
                        .iter()
                        .map(|&id| position(id))
                        .collect::<Result<Vec<Point>, IoError>>()?;
                    let spline = Spline::new(
                        usize::try_from(*degree).map_err(|_| not_a_curve())?,
                        clamped_uniform_knots(*degree, control.len()),
                        points,
                        None,
                    )
                    .map_err(|_| not_a_curve())?;
                    let (from, to) = (
                        nodes.at(first, position(first)?),
                        nodes.at(last, position(last)?),
                    );
                    links.push((
                        index,
                        Link {
                            entity: entity.id,
                            from,
                            to,
                            curve: LinkCurve::Spline(spline),
                        },
                    ));
                }
            }
        }
        nodes.merge_near();
        let mut kept = Vec::new();
        for (index, mut link) in links {
            link.from = nodes.find(link.from);
            link.to = nodes.find(link.to);
            // A spline may come back to where it started; a line or an
            // arc that does is nothing.
            if link.from == link.to && !matches!(link.curve, LinkCurve::Spline(_)) {
                outline.degenerate.push(link.entity);
            } else {
                kept.push((index, link));
            }
        }
        paths.extend(chains(&nodes, &kept, tolerance)?);
        paths.sort_by_key(|(index, _)| *index);
        let paths = paths.into_iter().map(|(_, path)| path).collect();
        if wanted {
            outline.construction = paths;
        } else {
            outline.paths = paths;
        }
    }
    Ok(outline)
}

/// The end points of a sketch's curves, with those at the same place
/// taken together.
struct Nodes {
    tolerance: f64,
    by_entity: BTreeMap<EntityId, usize>,
    position: Vec<Point>,
    /// Union-find: the node each node was merged into.
    parent: Vec<usize>,
}

impl Nodes {
    fn new(tolerance: f64) -> Self {
        Self {
            tolerance,
            by_entity: BTreeMap::new(),
            position: Vec::new(),
            parent: Vec::new(),
        }
    }

    fn at(&mut self, entity: EntityId, position: Point) -> usize {
        *self.by_entity.entry(entity).or_insert_with(|| {
            self.position.push(position);
            self.parent.push(self.parent.len());
            self.parent.len() - 1
        })
    }

    fn find(&self, node: usize) -> usize {
        let mut at = node;
        // A parent is always an earlier node: this ends.
        for _ in 0..self.parent.len() {
            match self.parent.get(at) {
                Some(&parent) if parent != at => at = parent,
                _ => break,
            }
        }
        at
    }

    fn place(&self, node: usize) -> Point {
        self.position.get(node).copied().unwrap_or([0.0, 0.0])
    }

    /// Takes nodes within the tolerance of each other together, each
    /// under the one made first: a sweep along x.
    fn merge_near(&mut self) {
        let mut order: Vec<usize> = (0..self.position.len()).collect();
        order.sort_by(|a, b| {
            self.place(*a)[0]
                .total_cmp(&self.place(*b)[0])
                .then(a.cmp(b))
        });
        for (i, &a) in order.iter().enumerate() {
            let pa = self.place(a);
            for &b in order.iter().skip(i + 1) {
                let pb = self.place(b);
                if pb[0] - pa[0] > self.tolerance {
                    break;
                }
                if (pb[0] - pa[0]).hypot(pb[1] - pa[1]) <= self.tolerance {
                    let (ra, rb) = (self.find(a), self.find(b));
                    let (low, high) = (ra.min(rb), ra.max(rb));
                    if let Some(parent) = self.parent.get_mut(high) {
                        *parent = low;
                    }
                }
            }
        }
    }
}

/// Joins links into chains. A chain runs through the nodes where exactly
/// two links meet and stops at any other.
fn chains(
    nodes: &Nodes,
    links: &[(usize, Link)],
    tolerance: f64,
) -> Result<Vec<(usize, Path)>, IoError> {
    let mut at_node: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (index, (_, link)) in links.iter().enumerate() {
        at_node.entry(link.from).or_default().push(index);
        // A link from a node to itself counts twice there.
        at_node.entry(link.to).or_default().push(index);
    }
    let degree = |node: usize| at_node.get(&node).map_or(0, Vec::len);
    let mut used = vec![false; links.len()];
    let mut out = Vec::new();
    // Walks from `node` along `first` until a node where the chain cannot
    // go on, or back at the start.
    let mut walk = |first: usize, node: usize, used: &mut Vec<bool>| -> Result<(), IoError> {
        let start = node;
        let (mut link_index, mut node) = (first, node);
        let mut segments = Vec::new();
        let mut order = usize::MAX;
        let mut closed = false;
        for _ in 0..=links.len() {
            let Some((entity_index, link)) = links.get(link_index) else {
                break;
            };
            if let Some(flag) = used.get_mut(link_index) {
                *flag = true;
            }
            order = order.min(*entity_index);
            let forward = link.from == node;
            let next = if forward { link.to } else { link.from };
            let (from, to) = (nodes.place(node), nodes.place(next));
            segments.push(segment(link, from, to, forward, tolerance)?);
            node = next;
            if node == start {
                closed = true;
                break;
            }
            if degree(node) != 2 {
                break;
            }
            let onward = at_node
                .get(&node)
                .into_iter()
                .flatten()
                .copied()
                .find(|other| *other != link_index);
            match onward {
                Some(other) if used.get(other) == Some(&false) => link_index = other,
                _ => break,
            }
        }
        if !segments.is_empty() {
            out.push((
                order,
                Path::Chain {
                    start: nodes.place(start),
                    segments,
                    closed,
                },
            ));
        }
        Ok(())
    };
    // Open chains first: from every node that is an end or a junction.
    for (index, (_, link)) in links.iter().enumerate() {
        for node in [link.from, link.to] {
            if used.get(index) == Some(&false) && degree(node) != 2 {
                walk(index, node, &mut used)?;
            }
        }
    }
    // What is left lies on loops.
    for (index, (_, link)) in links.iter().enumerate() {
        if used.get(index) == Some(&false) {
            walk(index, link.from, &mut used)?;
        }
    }
    Ok(out)
}

/// A link as a segment from `from` to `to`, walked the way it was drawn
/// (`forward`) or against it.
fn segment(
    link: &Link,
    from: Point,
    to: Point,
    forward: bool,
    tolerance: f64,
) -> Result<Segment, IoError> {
    match &link.curve {
        LinkCurve::Line => Ok(Segment::Line { to }),
        LinkCurve::Arc { center } => {
            let distance = |p: Point| (p[0] - center[0]).hypot(p[1] - center[1]);
            let (a, b) = (distance(from), distance(to));
            // The two ends of an arc are at one distance from its centre
            // once the sketch is solved; joining points within the
            // tolerance moves them by that much at most.
            if (a - b).abs() > 4.0 * tolerance + 1e-9 * a.max(b) {
                return Err(unsupported(format!(
                    "the ends of the arc {} are at {a} and {b} from its centre: solve the sketch first (sketch.solve)",
                    link.entity
                )));
            }
            Ok(Segment::Arc {
                to,
                center: *center,
                radius: (a + b) / 2.0,
                ccw: forward,
            })
        }
        LinkCurve::Spline(spline) => {
            let spline = if forward {
                spline.clone()
            } else {
                spline.reversed()
            };
            Ok(Segment::Spline(spline.with_ends(from, to)))
        }
    }
}

// ------------------------------------------------------------------- face

/// The loops of flat faces as paths, in the axes of the first face seen
/// from outside the body.
#[derive(Clone, Debug, PartialEq)]
pub struct FaceOutline {
    /// The plane the paths are drawn in: x to the right, y up, z towards
    /// the viewer, out of the body.
    pub frame: Frame,
    /// One closed path per loop: outer boundaries counter-clockwise,
    /// holes clockwise.
    pub paths: Vec<Path>,
}

/// The outline of the faces `faces` of `body`: every loop a closed path.
///
/// The faces must be flat and lie in one plane, facing the same way
/// (the pieces a boolean cut one face into do). The axes are those of the
/// first face's plane, seen from outside the body, so the outline is what
/// one sees looking at the face and not its mirror image.
pub fn face_outline(
    body: &Body,
    faces: &[FaceId],
    tolerance: Tolerance,
) -> Result<FaceOutline, IoError> {
    let mut frame: Option<Frame> = None;
    let mut paths = Vec::new();
    for id in faces {
        let face = body
            .face(*id)
            .ok_or_else(|| unsupported(format!("the body has no face {}", id.0)))?;
        let Surface::Plane(plane) = &face.surface else {
            return Err(unsupported(format!(
                "the face is a {} face: only a flat face has an outline to export",
                face.surface.kind()
            )));
        };
        let own = if face.reversed {
            plane.frame().reversed_at(plane.frame().origin())
        } else {
            Some(*plane.frame())
        }
        .ok_or_else(|| unsupported("the face has a degenerate plane"))?;
        let frame = *frame.get_or_insert(own);
        // Another face has to lie in the plane of the first.
        let off = frame.local(own.origin()).2.abs();
        let tilt = frame.z().cross(own.z()).length();
        if !(frame.z().dot(own.z()) > 0.0
            && tilt <= tolerance.angular.max(1e-9)
            && off <= 10.0 * tolerance.linear)
        {
            return Err(unsupported(
                "the faces of that name do not lie in one plane; export them one by one",
            ));
        }
        for loop_id in &face.loops {
            let coedges = &body
                .loop_(*loop_id)
                .ok_or_else(|| unsupported("the face has a loop that does not exist"))?
                .coedges;
            paths.push(loop_path(body, coedges, &frame, tolerance)?);
        }
    }
    let frame = frame.ok_or_else(|| unsupported("there is no face to export"))?;
    if paths.is_empty() {
        return Err(unsupported("the face has no boundary to export"));
    }
    Ok(FaceOutline { frame, paths })
}

fn flat(frame: &Frame, p: Vec3) -> Point {
    let (x, y, _) = frame.local(p);
    [x, y]
}

/// A direction in the plane's axes.
fn flat_vector(frame: &Frame, v: Vec3) -> Point {
    [v.dot(frame.x()), v.dot(frame.y())]
}

fn loop_path(
    body: &Body,
    coedges: &[Coedge],
    frame: &Frame,
    tolerance: Tolerance,
) -> Result<Path, IoError> {
    let broken = || unsupported("the face has an edge or a vertex that does not exist");
    let vertex = |id| -> Result<Point, IoError> {
        Ok(flat(frame, body.vertex(id).ok_or_else(broken)?.point))
    };
    // Whether a curve's own plane is this one, and whether it turns the
    // same way: +1, -1, or an error for a curve that leaves the plane.
    let turn = |axis: Vec3, kind: &str| -> Result<f64, IoError> {
        let along = axis.dot(frame.z());
        if frame.z().cross(axis).length() > tolerance.angular.max(1e-9) || along == 0.0 {
            return Err(unsupported(format!(
                "a {kind} edge of the face does not lie in the face's plane"
            )));
        }
        Ok(along.signum())
    };
    let mut start = None;
    let mut segments = Vec::with_capacity(coedges.len());
    for coedge in coedges {
        let edge = body.edge(coedge.edge).ok_or_else(broken)?;
        let (first, last) = body.coedge_ends(*coedge).ok_or_else(broken)?;
        let (from, to) = (vertex(first)?, vertex(last)?);
        start.get_or_insert(from);
        let whole = edge.start == edge.end;
        match &edge.curve {
            Curve3::Line(_) => segments.push(Segment::Line { to }),
            Curve3::Circle(circle) => {
                let ccw = (turn(circle.frame().z(), "circle")? > 0.0) != coedge.reversed;
                let center = flat(frame, circle.frame().origin());
                if whole {
                    if coedges.len() != 1 {
                        return Err(unsupported(
                            "a whole circle among other edges of one loop cannot be exported",
                        ));
                    }
                    return Ok(Path::Circle {
                        center,
                        radius: circle.radius(),
                    });
                }
                segments.push(Segment::Arc {
                    to,
                    center,
                    radius: circle.radius(),
                    ccw,
                });
            }
            Curve3::Ellipse(ellipse) => {
                let ccw = (turn(ellipse.frame().z(), "ellipse")? > 0.0) != coedge.reversed;
                let center = flat(frame, ellipse.frame().origin());
                let (a, b) = ellipse.semi_axes();
                let x = flat_vector(frame, ellipse.frame().x());
                let y = flat_vector(frame, ellipse.frame().y());
                // The longer semi-axis leads.
                let (major, ratio) = if a >= b {
                    ([x[0] * a, x[1] * a], b / a)
                } else {
                    ([y[0] * b, y[1] * b], a / b)
                };
                if whole {
                    if coedges.len() != 1 {
                        return Err(unsupported(
                            "a whole ellipse among other edges of one loop cannot be exported",
                        ));
                    }
                    return Ok(Path::Ellipse {
                        center,
                        major,
                        ratio,
                    });
                }
                segments.push(Segment::EllipticArc {
                    to,
                    center,
                    major,
                    ratio,
                    ccw,
                });
            }
            Curve3::Nurbs(curve) => {
                let control: Vec<Point> = curve
                    .control_points()
                    .iter()
                    .map(|p| flat(frame, *p))
                    .collect();
                let invalid = |e: craft3d_drawing::DrawingError| {
                    unsupported(format!("a spline edge of the face: {e}"))
                };
                let spline = Spline::new(
                    curve.degree(),
                    curve.knots().to_vec(),
                    control,
                    Some(curve.weights().to_vec()),
                )
                .map_err(invalid)?;
                // The edge may be a piece of its curve.
                let spline = if edge.range == spline.domain() {
                    spline
                } else {
                    spline
                        .between(edge.range.0, edge.range.1)
                        .map_err(invalid)?
                };
                let spline = if coedge.reversed {
                    spline.reversed()
                } else {
                    spline
                };
                segments.push(Segment::Spline(spline.with_ends(from, to)));
            }
        }
    }
    let start = start.ok_or_else(|| unsupported("the face has an empty loop"))?;
    Ok(Path::Chain {
        start,
        segments,
        closed: true,
    })
}
