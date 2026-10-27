//! Developable sheets and unfolds in regeneration
//! (docs/design/developable.md): the rails of a developable from a
//! sketch's open chain or from edges of earlier bodies, handed to
//! `craft3d_kernel::developable::connect`; the faces of an unfold handed
//! to `craft3d_kernel::unfold::develop` and the pattern placed in the
//! document.

use crate::datum::named_body_of;
use crate::extrude::drawn;
use crate::{FeatureResult, Names};
use craft3d_doc::{
    Developable, Document, FeatureId, NameRole, PersistentName, Rail as DocRail, Unfold,
};
use craft3d_expr::Evaluation;
use craft3d_geom::{Circle, Curve3, Line, NurbsCurve};
use craft3d_kernel::developable::{Piece, Rail, RailPiece, connect};
use craft3d_kernel::profile::{Shape, open_chain};
use craft3d_kernel::unfold::{Development, develop};
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_topo::{Body, FaceId};
use std::sync::Arc;

fn tolerance_of(doc: &Document) -> Result<Tolerance, String> {
    Tolerance::new(doc.tolerance.linear, doc.tolerance.angular)
        .ok_or("the document's tolerance is not positive and finite".to_string())
}

/// The rail of a sketch: its one open chain, lifted onto its plane.
fn sketch_rail(
    doc: &Document,
    earlier: &[FeatureResult],
    sketch: FeatureId,
    role: &str,
) -> Result<Rail, String> {
    let made = drawn(doc, earlier, sketch).map_err(|e| format!("{role}: {e}"))?;
    let name = made.name;
    let chain = open_chain(&made.curves, &made.tolerance).map_err(|e| {
        format!("{role}: the curves of sketch `{name}` are not one open chain: {e}")
    })?;
    let plane = made.plane;
    let lift = |p: [f64; 2]| plane.point(p[0], p[1], 0.0);
    let mut pieces = Vec::with_capacity(chain.segments.len());
    for segment in &chain.segments {
        let (a, b) = (lift(segment.start), lift(segment.end));
        let piece = match &segment.shape {
            Shape::Line => {
                let line = Line::through(a, b).map_err(|e| format!("{role}: {e}"))?;
                RailPiece {
                    curve: Curve3::Line(line),
                    range: (0.0, a.distance(b)),
                    reversed: false,
                    source: segment.source,
                }
            }
            Shape::Arc { center, radius } | Shape::Circle { center, radius } => {
                let frame = Frame::new(lift(*center), plane.z(), plane.x())
                    .ok_or_else(|| format!("{role}: the plane of sketch `{name}` has no axes"))?;
                let circle = Circle::new(frame, *radius).map_err(|e| format!("{role}: {e}"))?;
                let (begin, sweep) = segment.angles().unwrap_or((0.0, std::f64::consts::TAU));
                RailPiece {
                    curve: Curve3::Circle(circle),
                    range: (begin, begin + sweep),
                    reversed: segment.reversed,
                    source: segment.source,
                }
            }
            Shape::Spline(spline) => {
                let control: Vec<Vec3> = spline.control.iter().map(|p| lift(*p)).collect();
                let curve = NurbsCurve::polynomial(spline.degree, spline.knots.clone(), control)
                    .map_err(|e| format!("{role}: {e}"))?;
                let range = curve.domain();
                RailPiece {
                    curve: Curve3::Nurbs(curve),
                    range,
                    reversed: segment.reversed,
                    source: segment.source,
                }
            }
        };
        pieces.push(piece);
    }
    Ok(Rail { pieces })
}

/// The rail of edges of earlier bodies, each run the way that meets the
/// next.
fn edge_rail(
    doc: &Document,
    earlier: &[FeatureResult],
    edges: &[PersistentName],
    role: &str,
    tol: f64,
) -> Result<Rail, String> {
    if edges.is_empty() {
        return Err(format!("{role}: no edges"));
    }
    let mut raw = Vec::with_capacity(edges.len());
    for (k, name) in edges.iter().enumerate() {
        let (body, names, made_by) = named_body_of(doc, earlier, name, None, role)?;
        let index = names
            .edge_named(name)
            .ok_or_else(|| format!("{role}: `{made_by}` has no edge {name}"))?;
        let edge = body
            .edges
            .get(index)
            .ok_or_else(|| format!("{role}: `{made_by}` has no edge {name}"))?;
        let (a, b) = edge.range;
        raw.push((
            RailPiece {
                curve: edge.curve.clone(),
                range: (a, b),
                reversed: false,
                source: k as u64,
            },
            edge.curve.eval(a),
            edge.curve.eval(b),
        ));
    }
    // Orient: the first so that its end meets the second, every later
    // one so that its start meets the end before it.
    let mut pieces: Vec<RailPiece> = Vec::with_capacity(raw.len());
    let mut end: Option<Vec3> = None;
    let count = raw.len();
    for k in 0..count {
        let Some((piece, start, finish)) = raw.get(k).cloned() else {
            continue;
        };
        let reversed = match end {
            None => match raw.get(1) {
                Some((_, s1, f1)) => {
                    let forward = finish.distance(*s1).min(finish.distance(*f1));
                    let backward = start.distance(*s1).min(start.distance(*f1));
                    backward < forward
                }
                None => false,
            },
            Some(e) => {
                if start.distance(e) <= tol {
                    false
                } else if finish.distance(e) <= tol {
                    true
                } else {
                    return Err(format!(
                        "{role}: edge {} does not meet edge {}: the edges of a rail run end to end",
                        edges
                            .get(k - 1)
                            .map(ToString::to_string)
                            .unwrap_or_default(),
                        edges.get(k).map(ToString::to_string).unwrap_or_default()
                    ));
                }
            }
        };
        end = Some(if reversed { start } else { finish });
        pieces.push(RailPiece { reversed, ..piece });
    }
    Ok(Rail { pieces })
}

fn rail_of(
    doc: &Document,
    earlier: &[FeatureResult],
    rail: &DocRail,
    role: &str,
    tol: f64,
) -> Result<Rail, String> {
    match rail {
        DocRail::Sketch(id) => sketch_rail(doc, earlier, *id, role),
        DocRail::Edges(edges) => edge_rail(doc, earlier, edges, role, tol),
    }
}

/// The number a face of a developable is named by.
fn piece_number(piece: Piece) -> u64 {
    match piece {
        Piece::Rulings => 0,
        Piece::StartFan => 1,
        Piece::EndFan => 2,
        Piece::Inner(n) => 3 + u64::from(n),
    }
}

pub(crate) fn build_developable(
    doc: &Document,
    id: FeatureId,
    feature: &Developable,
    earlier: &[FeatureResult],
) -> Result<(Body, Names, Developed), String> {
    let tolerance = tolerance_of(doc)?;
    let a = rail_of(doc, earlier, &feature.a, "a", tolerance.linear)?;
    let b = rail_of(doc, earlier, &feature.b, "b", tolerance.linear)?;
    let made = connect(&a, &b, &tolerance).map_err(|e| e.to_string())?;
    let faces: Vec<PersistentName> = made
        .faces
        .iter()
        .map(|p| PersistentName::new(id, NameRole::Face, vec![piece_number(*p)]))
        .collect();
    let names = Names::by_faces(id, &made.body, faces, tolerance.linear)?;
    let report = Developed::Sheet {
        reversed_b: made.reversed_b,
        rulings: made.rulings.len(),
        deviation: made.deviation,
        curvature: made.curvature,
    };
    Ok((made.body, names, report))
}

/// What a developable or an unfold made beside its body, kept with the
/// feature's result (and so with the regeneration's cache).
#[derive(Clone, Debug, PartialEq)]
pub enum Developed {
    /// A developable: whether `b` was run the other way, how many rulings
    /// it has, how far the faces were found from the true developable and
    /// the largest |K| found on them.
    Sheet {
        reversed_b: bool,
        rulings: usize,
        deviation: f64,
        curvature: f64,
    },
    /// An unfold: its development, with the map between pattern and part.
    Pattern(Arc<Development>),
    /// A sheet-metal part: its pattern, bends and folded skin as built
    /// (what the bend table, the measures and the export read).
    SheetMetal(Arc<crate::SheetMetalPart>),
    /// A fold folded flat: the order of its layers and their display
    /// offsets (`fold::FlatFold`).
    FlatFold(Arc<crate::FlatFold>),
}

/// The faces of an unfold in its source's body, in the order given.
fn unfold_faces<'a>(
    doc: &Document,
    earlier: &'a [FeatureResult],
    feature: &Unfold,
) -> Result<(&'a Body, Vec<FaceId>), String> {
    let first = feature
        .faces
        .first()
        .ok_or_else(|| "faces: an unfold needs at least one face".to_string())?;
    let (body, names, made_by) = named_body_of(doc, earlier, first, None, "faces")?;
    let mut ids = Vec::with_capacity(feature.faces.len());
    for name in &feature.faces {
        if name.feature != first.feature {
            return Err(format!(
                "faces: {name} is not a face of `{made_by}`; the faces of an unfold are faces of one body"
            ));
        }
        let index = (0..body.faces.len())
            .find(|&i| names.face_answers_to(i, name))
            .ok_or_else(|| format!("faces: `{made_by}` has no face {name}"))?;
        ids.push(FaceId(
            u32::try_from(index).map_err(|_| "faces: too many faces".to_string())?,
        ));
    }
    Ok((body, ids))
}

/// The development of an unfold's faces, in the pattern's own axes.
fn development(
    doc: &Document,
    feature: &Unfold,
    earlier: &[FeatureResult],
) -> Result<Development, String> {
    let tolerance = tolerance_of(doc)?;
    let (body, ids) = unfold_faces(doc, earlier, feature)?;
    develop(body, &ids, &tolerance).map_err(|e| format!("faces: {e}"))
}

/// The frame an unfold's pattern lies in, with the document's variables.
pub fn placement(doc: &Document, feature: &Unfold) -> Result<Frame, String> {
    placed_frame(doc, &crate::feature_variables(doc), feature)
}

fn placed_frame(doc: &Document, variables: &Evaluation, feature: &Unfold) -> Result<Frame, String> {
    let at = crate::derive::lengths(doc, variables, "at", &feature.at)?;
    let turn = crate::angle(doc, variables, "angle", &feature.angle)?;
    if !turn.is_finite() {
        return Err("angle is not finite".to_string());
    }
    let (s, c) = turn.sin_cos();
    Frame::new(at, Vec3::Z, Vec3::new(c, s, 0.0))
        .ok_or_else(|| "the placement has no axes".to_string())
}

pub(crate) fn build_unfold(
    doc: &Document,
    variables: &Evaluation,
    id: FeatureId,
    feature: &Unfold,
    earlier: &[FeatureResult],
) -> Result<(Body, Names, Arc<Development>), String> {
    let tolerance = tolerance_of(doc)?;
    let frame = placed_frame(doc, variables, feature)?;
    let dev = Arc::new(development(doc, feature, earlier)?);
    let made = dev
        .flat_body(&frame, &tolerance)
        .map_err(|e| format!("faces: {e}"))?;
    let faces = vec![PersistentName::new(id, NameRole::Face, vec![0])];
    let names = Names::by_faces(id, &made.body, faces, tolerance.linear)?;
    Ok((made.body, names, dev))
}
