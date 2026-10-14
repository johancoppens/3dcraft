//! Loft: the solid whose side passes through a row of closed profiles, in
//! order, and whose ends are the first and the last of them (or that
//! closes on itself into a ring).
//!
//! **Sections.** Each section is one closed loop of sketch curves in its
//! own plane. A loop is walked counter-clockwise as seen looking back
//! along the loft (from the section after it towards the section before
//! it), whichever way its plane faces, and from the start of one of its
//! curves. The curves of the sections are matched in that order: the
//! first of each with the first of the others, and so on. Where a loop
//! starts: at the curve named by [`Section::start`]; without one, the
//! first section starts at its curve with the smallest id, and every
//! later section is turned to lie nearest to the section before it: with
//! as many curves as that one, so that the distances between matched
//! corners (each seen from the centre of its section's area, squared and
//! added up) are the least; otherwise at the corner that lies, seen from
//! the centre, most nearly in the direction in which the section before
//! it starts (of equally good starts, the one whose curve has the smallest
//! id). A whole circle has no corner: it is walked as four quarter arcs
//! from its point in that direction (circles before the first section
//! with corners take it from the section after them; a loft of circles
//! alone starts at angle 0 of the first). Sections with different
//! numbers of curves are matched by length, see [`loft`].
//!
//! **Sides.** Each column of matched curves gives one side face per band.
//! A band runs between two sections that follow each other when the loft
//! is ruled, closed or has only two sections, and through all sections
//! otherwise. The surface is, where it is one, a plane (every control
//! point in one plane: line to line in a plane), a cylinder or a cone
//! (arc to coaxial arc over the same angles); otherwise a B-spline
//! surface that contains every section curve exactly: the curves of a
//! column are brought to one degree and one knot vector
//! (`craft3d_geom::refine`), and each row of their control points is
//! joined by a straight line (ruled), by the spline of degree up to 3
//! that interpolates it at chord-length parameters (smooth), or by the
//! closed cubic spline with continuous curvature (a smooth ring, one
//! Bézier band per pair of sections). The parameters are the same for
//! every column, so neighbouring faces share their edges exactly.
//! Rational curves (arcs) are interpolated as homogeneous points.
//!
//! **What is checked.** Planes that cross within the sections, sections
//! that coincide, a plane that lies along the loft, a side that folds
//! (its normal vanishes) and everything `craft3d_topo::check` knows are
//! refused here. A side that passes through another side (sections
//! matched with too much twist) is not something this module can see: the
//! caller has to run a self-intersection check on the body
//! (`craft3d_tess::body_self_intersections`, as regeneration does) before
//! it uses it.

use crate::extrude::{Origin, Role};
use crate::profile::{Loop2, Profile, Segment, Shape};
use crate::{KernelError, MAX_REPORTED};
use craft3d_geom::refine::{compatible, interpolation, periodic_tangents, piece, reversed};
use craft3d_geom::{
    Circle, Cone, Curve3, Cylinder, Line, NurbsCurve, NurbsSurface, Plane, Surface,
};
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_topo::{Body, BodyKind, Coedge, EdgeId, FaceId, VertexId, check};
use std::f64::consts::{FRAC_PI_2, TAU};

/// Most sections of one loft.
pub const MAX_SECTIONS: usize = 64;

/// Most curves in the loop of one section, after matching.
pub const MAX_SECTION_CURVES: usize = 1024;

/// Points looked at per curve where sections are compared with planes.
const SAMPLES: u32 = 32;

/// One profile of a loft.
#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    /// Where the sketch lies: its x and y axes carry the loop's
    /// coordinates.
    pub plane: Frame,
    /// The closed loop, as [`crate::profile::detect_profiles`] gives the
    /// outer loop of a profile.
    pub outline: Loop2,
    /// The curve the loop starts with; `None` for the rule of the module
    /// documentation.
    pub start: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Options {
    /// Straight lines between sections that follow each other, instead of
    /// a smooth surface through all of them.
    pub ruled: bool,
    /// The last section joins the first: a ring without ends.
    pub closed: bool,
}

/// A lofted body and where each of its faces came from: `faces[i]` belongs
/// to `FaceId(i)`.
///
/// The start cap and the end cap are named after the curves of the first
/// and of the last section (ascending). A side is named after the curves
/// of its column, one per section, from the first section up to the
/// section the face ends at (so for a face through all sections: all of
/// them; for the face of a closed loft from the last section back to the
/// first: all of them and the first again). The parts a curve was cut in
/// (a whole circle, or a curve cut to match more curves of another
/// section) carry the same curve id: their faces share a name.
#[derive(Clone, Debug, PartialEq)]
pub struct Lofted {
    pub body: Body,
    pub faces: Vec<Origin>,
}

fn refuse(sections: &[usize], reason: impl Into<String>) -> KernelError {
    KernelError::Loft {
        sections: sections.to_vec(),
        reason: reason.into(),
    }
}

fn internal() -> KernelError {
    refuse(&[], "the loft lost track of a section; this is a bug")
}

/// An arc as a piece of a circle: the points `center + radius (cos a · x +
/// sin a · y)` for `a` from `from` over `sweep`, with `y = axis × x`.
#[derive(Clone, Copy, Debug)]
struct Arc {
    frame: Frame,
    radius: f64,
    from: f64,
    sweep: f64,
}

impl Arc {
    fn at(&self, angle: f64) -> Vec3 {
        let (s, c) = angle.sin_cos();
        self.frame.point(self.radius * c, self.radius * s, 0.0)
    }

    /// The exact rational quadratic through `start` and `end` (the ends of
    /// the arc, given so that they are the vertices to the bit), in pieces
    /// of at most a quarter turn (*The NURBS Book* §7.5).
    fn curve(&self, start: Vec3, end: Vec3) -> Result<NurbsCurve, KernelError> {
        let spans = ((self.sweep / FRAC_PI_2) - 1e-9).ceil().clamp(1.0, 4.0);
        let step = self.sweep / spans;
        let spans = spans as usize;
        let weight = (step / 2.0).cos();
        let mut control = vec![start];
        let mut weights = vec![1.0];
        let mut knots = vec![0.0; 3];
        for i in 0..spans {
            let begin = self.from + step * i as f64;
            let (s, c) = (begin + step / 2.0).sin_cos();
            let reach = self.radius / weight;
            control.push(self.frame.point(reach * c, reach * s, 0.0));
            weights.push(weight);
            if i + 1 < spans {
                control.push(self.at(begin + step));
                weights.push(1.0);
                let knot = (i + 1) as f64 / spans as f64;
                knots.extend([knot, knot]);
            }
        }
        control.push(end);
        weights.push(1.0);
        knots.extend([1.0; 3]);
        Ok(NurbsCurve::new(2, knots, control, weights)?)
    }

    fn part(&self, from: f64, sweep: f64) -> Arc {
        Arc {
            from,
            sweep,
            ..*self
        }
    }
}

/// One curve of a section, in the direction the loop is walked.
#[derive(Clone, Debug)]
struct Piece {
    source: u64,
    start: Vec3,
    end: Vec3,
    /// The curve as a spline, from `start` to `end`.
    curve: NurbsCurve,
    /// The curve the edge is given, and the range of it this piece is.
    edge: Curve3,
    range: (f64, f64),
    arc: Option<Arc>,
    /// The length along the curve.
    length: f64,
}

impl Piece {
    fn line(source: u64, start: Vec3, end: Vec3) -> Result<Piece, KernelError> {
        let length = start.distance(end);
        Ok(Piece {
            source,
            start,
            end,
            curve: NurbsCurve::polynomial(1, vec![0.0, 0.0, 1.0, 1.0], vec![start, end])?,
            edge: Curve3::Line(Line::through(start, end)?),
            range: (0.0, length),
            arc: None,
            length,
        })
    }

    fn arc(source: u64, arc: Arc, start: Vec3, end: Vec3) -> Result<Piece, KernelError> {
        Ok(Piece {
            source,
            start,
            end,
            curve: arc.curve(start, end)?,
            edge: Curve3::Circle(Circle::new(arc.frame, arc.radius)?),
            range: (arc.from, arc.from + arc.sweep),
            arc: Some(arc),
            length: arc.radius * arc.sweep,
        })
    }

    /// A point of the piece at fraction `s` of its parameter.
    fn sample(&self, s: f64) -> Vec3 {
        let (a, b) = self.curve.domain();
        self.curve.eval(a + (b - a) * s)
    }

    /// The length of the spline from its start to parameter `t`, by
    /// Gauss–Legendre quadrature per knot span.
    fn spline_length(curve: &NurbsCurve, t: f64) -> f64 {
        // Five points: exact for a speed that is a polynomial of degree 9.
        const NODES: [(f64, f64); 5] = [
            (0.0, 0.568_888_888_888_888_9),
            (-0.538_469_310_105_683, 0.478_628_670_499_366_5),
            (0.538_469_310_105_683, 0.478_628_670_499_366_5),
            (-0.906_179_845_938_664, 0.236_926_885_056_189_08),
            (0.906_179_845_938_664, 0.236_926_885_056_189_08),
        ];
        let (a, _) = curve.domain();
        let mut breaks: Vec<f64> = vec![a];
        for &k in curve.knots() {
            if k > a && k < t && breaks.last().is_some_and(|last| k > *last) {
                breaks.push(k);
            }
        }
        breaks.push(t);
        let mut total = 0.0;
        for (&lo, &hi) in breaks.iter().zip(breaks.iter().skip(1)) {
            // Each span in four parts: the speed is not a polynomial.
            for part in 0..4 {
                let (p0, p1) = (
                    lo + (hi - lo) * f64::from(part) / 4.0,
                    lo + (hi - lo) * f64::from(part + 1) / 4.0,
                );
                let (mid, half) = ((p0 + p1) / 2.0, (p1 - p0) / 2.0);
                for (x, w) in NODES {
                    total += w * half * curve.derivatives(mid + half * x).1.length();
                }
            }
        }
        total
    }

    /// The parts of this piece between the fractions `cuts` of its length
    /// (ascending, strictly between 0 and 1): one more part than cuts.
    fn split(&self, cuts: &[f64]) -> Result<Vec<Piece>, KernelError> {
        if cuts.is_empty() {
            return Ok(vec![self.clone()]);
        }
        let mut bounds = vec![0.0];
        bounds.extend_from_slice(cuts);
        bounds.push(1.0);
        let last = bounds.len().saturating_sub(2);
        let mut parts = Vec::with_capacity(bounds.len());
        let mut start = self.start;
        // For a spline: the parameter at each bound.
        let (a, b) = self.curve.domain();
        let mut from_t = a;
        for (index, (&lo, &hi)) in bounds.iter().zip(bounds.iter().skip(1)).enumerate() {
            let part = if let Some(arc) = &self.arc {
                let part = arc.part(arc.from + arc.sweep * lo, arc.sweep * (hi - lo));
                let end = if index == last {
                    self.end
                } else {
                    part.at(part.from + part.sweep)
                };
                Piece::arc(self.source, part, start, end)?
            } else if matches!(self.edge, Curve3::Line(_)) {
                let end = if index == last {
                    self.end
                } else {
                    self.start.lerp(self.end, hi)
                };
                let mut part = Piece::line(self.source, start, end)?;
                // The edge stays on the line of the whole curve.
                part.edge = self.edge.clone();
                part.range = (
                    self.range.0 + self.length * lo,
                    self.range.0 + self.length * hi,
                );
                part.length = self.length * (hi - lo);
                part
            } else {
                // The parameter where the length reaches the fraction, by
                // bisection (the length grows with the parameter).
                let to_t = if index == last {
                    b
                } else {
                    let target = self.length * hi;
                    let (mut low, mut high) = (from_t, b);
                    for _ in 0..60 {
                        let mid = (low + high) / 2.0;
                        if Piece::spline_length(&self.curve, mid) < target {
                            low = mid;
                        } else {
                            high = mid;
                        }
                    }
                    (low + high) / 2.0
                };
                if to_t <= from_t {
                    return Err(refuse(
                        &[],
                        format!("curve {} is too short to cut into parts", self.source),
                    ));
                }
                let curve = piece(&self.curve, from_t, to_t)?;
                let end = if index == last {
                    self.end
                } else {
                    *curve.control_points().last().ok_or_else(internal)?
                };
                // The first part starts on the vertex to the bit; so does
                // each next one, on the end of the part before it.
                let curve = pinned(&curve, start, end)?;
                let part = Piece {
                    source: self.source,
                    start,
                    end,
                    curve,
                    edge: self.edge.clone(),
                    range: (from_t, to_t),
                    arc: None,
                    length: self.length * (hi - lo),
                };
                from_t = to_t;
                part
            };
            start = part.end;
            parts.push(part);
        }
        Ok(parts)
    }
}

/// `curve` with its first and last control point replaced by `start` and
/// `end`, which they are up to rounding.
pub(crate) fn pinned(
    curve: &NurbsCurve,
    start: Vec3,
    end: Vec3,
) -> Result<NurbsCurve, KernelError> {
    let mut control = curve.control_points().to_vec();
    let mut weights = curve.weights().to_vec();
    if let (Some(first), Some(w)) = (control.first_mut(), weights.first_mut()) {
        *first = start;
        *w = 1.0;
    }
    if let (Some(last), Some(w)) = (control.last_mut(), weights.last_mut()) {
        *last = end;
        *w = 1.0;
    }
    Ok(NurbsCurve::new(
        curve.degree(),
        curve.knots().to_vec(),
        control,
        weights,
    )?)
}

/// A section before its whole circle, if it is one, is cut up.
enum Outline {
    /// A whole circle, walked counter-clockwise about `axis`.
    Whole {
        source: u64,
        center: Vec3,
        axis: Vec3,
        radius: f64,
    },
    Pieces(Vec<Piece>),
}

/// The segment walked the other way.
fn turned(segment: &Segment) -> Segment {
    Segment {
        source: segment.source,
        piece: segment.piece,
        reversed: !segment.reversed,
        start: segment.end,
        end: segment.start,
        shape: segment.shape.clone(),
    }
}

/// The curves of section `index` in the order and direction they are
/// walked: counter-clockwise about `forward` (the plane's normal, or its
/// opposite), from the curve the section starts with.
fn outline(index: usize, section: &Section, flip: bool) -> Result<Outline, KernelError> {
    let plane = &section.plane;
    let segments: Vec<Segment> = if flip {
        section.outline.segments.iter().rev().map(turned).collect()
    } else {
        section.outline.segments.clone()
    };
    let at = |p: [f64; 2]| {
        let point = plane.point(p[0], p[1], 0.0);
        if point.is_finite() {
            Ok(point)
        } else {
            Err(refuse(&[index], "a point of the section is not finite"))
        }
    };
    let number = index + 1;
    if let [only] = segments.as_slice() {
        let Shape::Circle { center, radius } = only.shape else {
            return Err(KernelError::InvalidProfile {
                reason: "a loop of one segment that is not a circle is not closed",
            });
        };
        if section.start.is_some_and(|start| start != only.source) {
            return Err(refuse(
                &[index],
                format!(
                    "start: section {number} has no curve {} in its profile",
                    section.start.unwrap_or(0)
                ),
            ));
        }
        if !(radius.is_finite() && radius > 0.0) {
            return Err(KernelError::InvalidSize {
                name: "radius",
                value: radius,
            });
        }
        let axis = if only.reversed { -plane.z() } else { plane.z() };
        return Ok(Outline::Whole {
            source: only.source,
            center: at(center)?,
            axis,
            radius,
        });
    }
    if segments.len() > MAX_SECTION_CURVES {
        return Err(KernelError::TooManyCurves {
            count: segments.len(),
            limit: MAX_SECTION_CURVES,
        });
    }
    let mut pieces = Vec::with_capacity(segments.len());
    for segment in &segments {
        let (start, end) = (at(segment.start)?, at(segment.end)?);
        let made = match &segment.shape {
            Shape::Line => Piece::line(segment.source, start, end)?,
            &Shape::Arc { center, radius } => {
                let (_, sweep) = segment.angles().ok_or(KernelError::InvalidProfile {
                    reason: "an arc without angles",
                })?;
                if !(sweep > 0.0 && sweep < TAU && radius.is_finite() && radius > 0.0) {
                    return Err(KernelError::InvalidProfile {
                        reason: "an arc whose start and end coincide; a full turn is a circle",
                    });
                }
                let center = at(center)?;
                let axis = if segment.reversed {
                    -plane.z()
                } else {
                    plane.z()
                };
                let frame = Frame::new(center, axis, start - center).ok_or_else(|| {
                    refuse(&[index], "an arc of the section has no radius to speak of")
                })?;
                Piece::arc(
                    segment.source,
                    Arc {
                        frame,
                        radius,
                        from: 0.0,
                        sweep,
                    },
                    start,
                    end,
                )?
            }
            Shape::Circle { .. } => {
                return Err(KernelError::InvalidProfile {
                    reason: "a circle is a loop on its own",
                });
            }
            Shape::Spline(spline) => {
                if let Some(reason) = spline.fault() {
                    return Err(KernelError::DegenerateCurve {
                        id: segment.source,
                        reason,
                    });
                }
                let control = spline
                    .control
                    .iter()
                    .map(|p| at(*p))
                    .collect::<Result<Vec<_>, _>>()?;
                let own = NurbsCurve::polynomial(spline.degree, spline.knots.clone(), control)?;
                let curve = if segment.reversed {
                    // The edge keeps the spline's own curve; walked
                    // backwards it is the reversed one.
                    reversed(&own)?
                } else {
                    own
                };
                let curve = pinned(&curve, start, end)?;
                let length = Piece::spline_length(&curve, curve.domain().1);
                Piece {
                    source: segment.source,
                    start,
                    end,
                    range: curve.domain(),
                    edge: Curve3::Nurbs(curve.clone()),
                    curve,
                    arc: None,
                    length,
                }
            }
        };
        pieces.push(made);
    }
    if pieces.is_empty() {
        return Err(KernelError::InvalidProfile {
            reason: "a loop without segments",
        });
    }
    if let Some(start) = section.start
        && !pieces.iter().any(|p| p.source == start)
    {
        return Err(refuse(
            &[index],
            format!("start: section {number} has no curve {start} in its profile"),
        ));
    }
    Ok(Outline::Pieces(pieces))
}

/// The curve of `own` to start with so that its corners lie nearest to
/// those of `before`, a section with as many curves, when both are seen
/// from their centres: the turn of the loop for which the squares of the
/// distances between matched corners add up to the least. Of turns that
/// are equally good, the one that starts with the smallest curve id.
fn nearest_turn(
    own: &[Piece],
    middle: Vec3,
    before: &[Piece],
    before_middle: Vec3,
) -> Option<usize> {
    let count = own.len();
    let mut best: Option<(usize, f64, u64)> = None;
    for turn in 0..count {
        let sum: f64 = before
            .iter()
            .zip(own.iter().cycle().skip(turn))
            .map(|(b, o)| ((o.start - middle) - (b.start - before_middle)).length_squared())
            .sum();
        let id = own.get(turn)?.source;
        let better = match best {
            None => true,
            Some((_, least, smallest)) => {
                let same = (sum - least).abs() <= 1e-9 * (sum + least);
                (sum < least && !same) || (same && id < smallest)
            }
        };
        if better && sum.is_finite() {
            best = Some((turn, sum, id));
        }
    }
    best.map(|(turn, _, _)| turn)
}

/// The curve of `own` whose start lies, seen from `middle`, most nearly
/// in the direction `towards`; of those equally near it, the one with the
/// smallest id.
fn in_direction(own: &[Piece], middle: Vec3, towards: Vec3) -> Option<usize> {
    let mut best: Option<(usize, f64, u64)> = None;
    for (i, p) in own.iter().enumerate() {
        let Some(d) = (p.start - middle).normalized() else {
            continue;
        };
        let along = d.dot(towards);
        let better = match best {
            None => true,
            Some((_, most, id)) => {
                along > most + 1e-9 || ((along - most).abs() <= 1e-9 && p.source < id)
            }
        };
        if better {
            best = Some((i, along, p.source));
        }
    }
    best.map(|(i, _, _)| i)
}

/// A whole circle as `parts` equal arcs from the point at direction
/// `towards` from its centre.
fn circle_pieces(
    index: usize,
    source: u64,
    center: Vec3,
    axis: Vec3,
    radius: f64,
    towards: Vec3,
    parts: u32,
) -> Result<Vec<Piece>, KernelError> {
    let frame = Frame::new(center, axis, towards).ok_or_else(|| {
        refuse(
            &[index],
            "a circle of the section has no direction to start in",
        )
    })?;
    let whole = Arc {
        frame,
        radius,
        from: 0.0,
        sweep: TAU,
    };
    let step = TAU / f64::from(parts);
    let first = whole.at(0.0);
    let mut pieces = Vec::with_capacity(parts as usize);
    let mut start = first;
    for part in 0..parts {
        let arc = whole.part(step * f64::from(part), step);
        let end = if part + 1 == parts {
            first
        } else {
            arc.at(arc.from + arc.sweep)
        };
        pieces.push(Piece::arc(source, arc, start, end)?);
        start = end;
    }
    Ok(pieces)
}

/// Cuts the sections so that all have the same number of curves: every
/// section is cut where any other section has a corner, at the same
/// fraction of the length of its loop from its start. Sections that have
/// equally many curves already are left as they are.
fn matched(sections: Vec<Vec<Piece>>, tol: f64) -> Result<Vec<Vec<Piece>>, KernelError> {
    let count = sections.first().map_or(0, Vec::len);
    if sections.iter().all(|s| s.len() == count) {
        return Ok(sections);
    }
    // Where each section has its corners, as fractions of its length.
    let mut corners: Vec<Vec<f64>> = Vec::with_capacity(sections.len());
    for pieces in &sections {
        let total: f64 = pieces.iter().map(|p| p.length).sum();
        if !(total.is_finite() && total > 0.0) {
            return Err(internal());
        }
        let mut sum = 0.0;
        let mut own = Vec::with_capacity(pieces.len());
        for piece in pieces {
            own.push(sum / total);
            sum += piece.length;
        }
        corners.push(own);
    }
    // All of them, ascending; corners of different sections that fall
    // within a thousandth of the loop of each other are one corner, so a
    // sliver of a face is not made for a difference nobody drew.
    const SAME_CORNER: f64 = 1e-3;
    let mut all: Vec<f64> = corners.iter().flatten().copied().collect();
    all.sort_by(f64::total_cmp);
    let mut cuts: Vec<f64> = Vec::new();
    for c in all {
        if cuts.last().is_none_or(|last| c - last > SAME_CORNER) && 1.0 - c > SAME_CORNER {
            cuts.push(c);
        }
    }
    if cuts.len() > MAX_SECTION_CURVES {
        return Err(KernelError::TooManyCurves {
            count: cuts.len(),
            limit: MAX_SECTION_CURVES,
        });
    }
    let mut out = Vec::with_capacity(sections.len());
    for (index, (pieces, own)) in sections.into_iter().zip(&corners).enumerate() {
        let total: f64 = pieces.iter().map(|p| p.length).sum();
        let mut cut_section = Vec::with_capacity(cuts.len());
        for (j, piece) in pieces.iter().enumerate() {
            let begin = own.get(j).copied().unwrap_or(0.0);
            let end = own.get(j + 1).copied().unwrap_or(1.0);
            // The cuts strictly inside this piece, as fractions of it; a
            // cut at a corner of this section (within the same margin) is
            // that corner.
            let inside: Vec<f64> = cuts
                .iter()
                .filter(|c| **c - begin > SAME_CORNER && end - **c > SAME_CORNER)
                .map(|c| (c - begin) / (end - begin))
                .collect();
            if inside
                .iter()
                .zip(inside.iter().skip(1))
                .any(|(a, b)| (b - a) * piece.length <= 4.0 * tol)
                || (!inside.is_empty() && piece.length / total <= SAME_CORNER)
            {
                return Err(refuse(
                    &[index],
                    format!(
                        "section {} would be cut into parts shorter than the tolerance to match the other sections; give the sections the same number of curves",
                        index + 1
                    ),
                ));
            }
            cut_section.extend(piece.split(&inside)?);
        }
        out.push(cut_section);
    }
    let count = out.first().map_or(0, Vec::len);
    if out.iter().any(|s| s.len() != count) {
        let counts: Vec<String> = out.iter().map(|s| s.len().to_string()).collect();
        let all: Vec<usize> = (0..out.len()).collect();
        return Err(refuse(
            &all,
            format!(
                "the sections cannot be matched: cut where the others have their corners they have {} curves; give the sections the same number of curves, with corners at matching places",
                counts.join(", ")
            ),
        ));
    }
    Ok(out)
}

/// A stretch of the loft between two sections with edges, and how the
/// rows of the sections' control points become the rows of its surfaces.
pub(crate) struct Band {
    pub(crate) from: usize,
    pub(crate) to: usize,
    pub(crate) degree: usize,
    pub(crate) knots: Vec<f64>,
    /// Row `r` of the band is the sum over the sections `k` of
    /// `mix[r][k]` times row `k`.
    pub(crate) mix: Vec<Vec<f64>>,
}

impl Band {
    pub(crate) fn straight(from: usize, to: usize, sections: usize) -> Band {
        let unit = |k: usize| (0..sections).map(|i| f64::from(u8::from(i == k))).collect();
        Band {
            from,
            to,
            degree: 1,
            knots: vec![0.0, 0.0, 1.0, 1.0],
            mix: vec![unit(from), unit(to)],
        }
    }

    /// The rows of the band from one point per section.
    pub(crate) fn points(&self, per_section: &[Vec3]) -> Vec<Vec3> {
        self.mix
            .iter()
            .map(|row| {
                row.iter()
                    .zip(per_section)
                    .filter(|(w, _)| **w != 0.0)
                    .fold(Vec3::ZERO, |sum, (w, p)| sum + *p * *w)
            })
            .collect()
    }

    /// The same for homogeneous points: (position times weight, weight).
    fn weighted(&self, per_section: &[(Vec3, f64)]) -> Vec<(Vec3, f64)> {
        self.mix
            .iter()
            .map(|row| {
                row.iter()
                    .zip(per_section)
                    .filter(|(w, _)| **w != 0.0)
                    .fold((Vec3::ZERO, 0.0), |sum, (w, (p, weight))| {
                        (sum.0 + *p * (*weight * *w), sum.1 + weight * w)
                    })
            })
            .collect()
    }
}

/// The bands of a loft through sections whose distances (in the
/// parameter of the loft) are `steps`: `steps[k]` from section `k` to the
/// next, and for a closed loft the last one back to the first.
fn bands(sections: usize, steps: &[f64], options: &Options) -> Result<Vec<Band>, KernelError> {
    let unit =
        |k: usize| -> Vec<f64> { (0..sections).map(|i| f64::from(u8::from(i == k))).collect() };
    if options.closed {
        if options.ruled {
            return Ok((0..sections)
                .map(|k| Band::straight(k, (k + 1) % sections, sections))
                .collect());
        }
        let tangents = periodic_tangents(steps)?;
        let mut out = Vec::with_capacity(sections);
        for k in 0..sections {
            let next = (k + 1) % sections;
            let (Some(&h), Some(here), Some(there)) =
                (steps.get(k), tangents.get(k), tangents.get(next))
            else {
                return Err(internal());
            };
            // The cubic from section k to the next as a Bézier curve:
            // P_k, P_k + h/3 T_k, P_next − h/3 T_next, P_next.
            let mut leave = unit(k);
            for (slot, t) in leave.iter_mut().zip(here) {
                *slot += h / 3.0 * t;
            }
            let mut arrive = unit(next);
            for (slot, t) in arrive.iter_mut().zip(there) {
                *slot -= h / 3.0 * t;
            }
            out.push(Band {
                from: k,
                to: next,
                degree: 3,
                knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
                mix: vec![unit(k), leave, arrive, unit(next)],
            });
        }
        return Ok(out);
    }
    if options.ruled || sections == 2 {
        return Ok((0..sections - 1)
            .map(|k| Band::straight(k, k + 1, sections))
            .collect());
    }
    let total: f64 = steps.iter().sum();
    let mut params = Vec::with_capacity(sections);
    let mut sum = 0.0;
    params.push(0.0);
    for (k, step) in steps.iter().enumerate() {
        sum += step;
        params.push(if k + 2 == sections { 1.0 } else { sum / total });
    }
    let degree = (sections - 1).min(3);
    let (knots, mut mix) = interpolation(&params, degree)?;
    // The spline starts on the first point and ends on the last: say so
    // to the bit, so that the faces end on the section curves exactly.
    if let Some(first) = mix.first_mut() {
        *first = unit(0);
    }
    if let Some(last) = mix.last_mut() {
        *last = unit(sections - 1);
    }
    Ok(vec![Band {
        from: 0,
        to: sections - 1,
        degree,
        knots,
        mix,
    }])
}

/// The cylinder or cone through two arcs about one axis that cover the
/// same angles, if they are that, and whether the face looks away from
/// the axis.
fn round(a: &Arc, b: &Arc, forward: Vec3, tolerance: &Tolerance) -> Option<(Surface, bool)> {
    let parallel = |u: Vec3, v: Vec3| u.cross(v).length() <= tolerance.angular && u.dot(v) > 0.0;
    if !parallel(a.frame.z(), b.frame.z())
        || !parallel(a.frame.x(), b.frame.x())
        || (a.from - b.from).abs() > tolerance.angular
        || (a.sweep - b.sweep).abs() > tolerance.angular
    {
        return None;
    }
    let offset = b.frame.origin() - a.frame.origin();
    let height = offset.dot(a.frame.z());
    if (offset - a.frame.z() * height).length() > tolerance.linear
        || height.abs() <= tolerance.linear
    {
        return None;
    }
    let outward = a.frame.z().dot(forward) > 0.0;
    let surface = if (a.radius - b.radius).abs() <= tolerance.linear {
        Surface::Cylinder(Cylinder::new(a.frame, a.radius).ok()?)
    } else {
        let half_angle = ((b.radius - a.radius) / height).atan();
        Surface::Cone(Cone::new(a.frame, a.radius, half_angle).ok()?)
    };
    Some((surface, outward))
}

/// The straight line through `points`, when they lie on one in order.
pub(crate) fn straight(points: &[Vec3], slack: f64) -> Option<(Curve3, (f64, f64))> {
    let (first, last) = (*points.first()?, *points.last()?);
    let length = first.distance(last);
    let along = (last - first).normalized()?;
    let mut reached = 0.0;
    for p in points.iter().skip(1) {
        let d = *p - first;
        let on = d.dot(along);
        if (d - along * on).length() > slack || on <= reached {
            return None;
        }
        reached = on;
    }
    Some((
        Curve3::Line(Line::through(first, last).ok()?),
        (0.0, length),
    ))
}

/// Everything the loft is built from, per section.
struct Walk {
    pieces: Vec<Piece>,
    /// The plane's normal, turned to point along the loft.
    forward: Vec3,
}

struct Builder {
    body: Body,
    faces: Vec<Origin>,
}

impl Builder {
    fn face(
        &mut self,
        surface: Surface,
        reversed: bool,
        loops: Vec<Vec<Coedge>>,
        role: Role,
        sources: Vec<u64>,
    ) -> FaceId {
        let loops = loops.into_iter().map(|l| self.body.add_loop(l)).collect();
        // Parts of one cut curve share a side's name in a loft: no piece
        // number of its own.
        self.faces.push(Origin {
            role,
            sources,
            piece: 0,
        });
        self.body.add_face(surface, reversed, loops)
    }
}

/// Lofts through `sections` in order.
///
/// Every section is one closed planar loop. They are matched as the
/// module documentation says; sections with different numbers of curves
/// are first cut so that the numbers agree: each where the others have
/// their corners, at the same fraction of the length of its loop measured
/// from its start (a square and a circle: the circle in four quarters; a
/// rectangle and a circle: arcs as long, in proportion, as the sides).
/// Corners of different sections within a thousandth of the loop of each
/// other count as one.
///
/// The body is a solid of one shell that has passed `craft3d_topo::check`
/// with `tolerance`. It can still pass through itself when the sections
/// are matched with a twist: see the module documentation. Entities are
/// made in a fixed order: per section with edges its vertices and edges,
/// then per band the edges between the sections and the side faces, then
/// the start cap and the end cap.
pub fn loft(
    sections: &[Section],
    options: &Options,
    tolerance: &Tolerance,
) -> Result<Lofted, KernelError> {
    let tol = tolerance.linear;
    if !(tol.is_finite() && tol > 0.0) {
        return Err(KernelError::InvalidSize {
            name: "tolerance",
            value: tol,
        });
    }
    let count = sections.len();
    let least = if options.closed { 3 } else { 2 };
    if count < least || count > MAX_SECTIONS {
        return Err(refuse(
            &[],
            if options.closed {
                format!(
                    "a closed loft needs at least 3 sections and at most {MAX_SECTIONS}, got {count}"
                )
            } else {
                format!("a loft needs at least 2 sections and at most {MAX_SECTIONS}, got {count}")
            },
        ));
    }

    // Where each section is, and which way the loft passes through it.
    let mut centers = Vec::with_capacity(count);
    for (index, section) in sections.iter().enumerate() {
        if section.outline.segments.is_empty() {
            return Err(KernelError::InvalidProfile {
                reason: "a loop without segments",
            });
        }
        let region = Profile {
            outer: section.outline.clone(),
            holes: Vec::new(),
        };
        let center = region
            .centroid()
            .map(|c| section.plane.point(c[0], c[1], 0.0))
            .filter(|c| c.is_finite())
            .ok_or_else(|| refuse(&[index], format!("section {} encloses no area", index + 1)))?;
        centers.push(center);
    }
    let center = |k: usize| centers.get(k).copied().ok_or_else(internal);
    let mut walks = Vec::with_capacity(count);
    let mut outlines = Vec::with_capacity(count);
    for (index, section) in sections.iter().enumerate() {
        let (before, after) = if options.closed {
            ((index + count - 1) % count, (index + 1) % count)
        } else {
            (index.saturating_sub(1), (index + 1).min(count - 1))
        };
        let travel = center(after)? - center(before)?;
        let normal = section.plane.z();
        let lean = travel.normalized().map(|t| t.dot(normal));
        let Some(lean) = lean.filter(|l| l.abs() > 1e-6) else {
            return Err(refuse(
                &[index],
                format!(
                    "the plane of section {} lies along the loft (the sections next to it are not on either side of it), or the sections next to it are in one place",
                    index + 1
                ),
            ));
        };
        let forward = if lean > 0.0 { normal } else { -normal };
        // Counter-clockwise about `forward`.
        let counter_clockwise = section.outline.signed_area() > 0.0;
        let flip = counter_clockwise != (lean > 0.0);
        outlines.push(outline(index, section, flip)?);
        walks.push(forward);
    }

    // Where each section starts. The first section with corners starts
    // at the curve it names, or at its curve with the smallest id; every
    // other section starts, unless it names a curve, in the direction
    // (from its centre) in which the section next to it starts. A whole
    // circle is cut into quarters from there.
    let settled = outlines
        .iter()
        .position(|o| matches!(o, Outline::Pieces(_)))
        .unwrap_or(0);
    let mut pieces: Vec<Option<Vec<Piece>>> = (0..count).map(|_| None).collect();
    let order = (settled..count).chain((0..settled).rev());
    for index in order {
        let neighbour = if index > settled {
            index - 1
        } else {
            index + 1
        };
        let section = sections.get(index).ok_or_else(internal)?;
        let forward = *walks.get(index).ok_or_else(internal)?;
        // The direction the neighbour starts in, laid in this plane.
        let towards = match (
            pieces
                .get(neighbour)
                .and_then(Option::as_ref)
                .and_then(|p| p.first()),
            centers.get(neighbour),
        ) {
            (Some(first), Some(&middle)) if index != settled => {
                let d = first.start - middle;
                (d - forward * d.dot(forward)).normalized()
            }
            _ => None,
        };
        let made = match outlines.get(index).ok_or_else(internal)? {
            Outline::Pieces(own) => {
                let mut own = own.clone();
                let smallest = || {
                    own.iter()
                        .enumerate()
                        .min_by_key(|(_, p)| p.source)
                        .map_or(0, |(i, _)| i)
                };
                let first = match (section.start, towards) {
                    (Some(start), _) => own
                        .iter()
                        .position(|p| p.source == start)
                        .ok_or_else(internal)?,
                    (None, None) => smallest(),
                    (None, Some(towards)) => {
                        let before = pieces.get(neighbour).and_then(Option::as_ref);
                        match before.filter(|b| b.len() == own.len()) {
                            Some(before) => {
                                nearest_turn(&own, center(index)?, before, center(neighbour)?)
                            }
                            None => in_direction(&own, center(index)?, towards),
                        }
                        .unwrap_or_else(smallest)
                    }
                };
                own.rotate_left(first);
                own
            }
            &Outline::Whole {
                source,
                center,
                axis,
                radius,
            } => {
                let towards = towards.unwrap_or(section.plane.x());
                circle_pieces(index, source, center, axis, radius, towards, 4)?
            }
        };
        if let Some(slot) = pieces.get_mut(index) {
            *slot = Some(made);
        }
    }
    let pieces: Vec<Vec<Piece>> = pieces
        .into_iter()
        .collect::<Option<Vec<_>>>()
        .ok_or_else(internal)?;

    // A loft of whole circles about one axis, in straight bands, is a row
    // of cylinders and cones without a seam.
    let tube = tube(&outlines, options, tolerance);
    let pieces = if tube {
        let mut whole = Vec::with_capacity(count);
        for (index, outline) in outlines.iter().enumerate() {
            let &Outline::Whole {
                source,
                center,
                axis,
                radius,
            } = outline
            else {
                return Err(internal());
            };
            let plane = &sections.get(index).ok_or_else(internal)?.plane;
            whole.push(circle_pieces(
                index,
                source,
                center,
                axis,
                radius,
                plane.x(),
                1,
            )?);
        }
        whole
    } else {
        matched(pieces, tol)?
    };
    let width = pieces.first().map_or(0, Vec::len);
    let sections_walked: Vec<Walk> = pieces
        .into_iter()
        .zip(walks)
        .map(|(pieces, forward)| Walk { pieces, forward })
        .collect();
    let walk = |k: usize| sections_walked.get(k).ok_or_else(internal);

    // No section may reach the plane of the one next to it.
    let pairs: Vec<(usize, usize)> = (0..count - 1)
        .map(|k| (k, k + 1))
        .chain(options.closed.then_some((count - 1, 0)))
        .collect();
    for &(a, b) in &pairs {
        let (first, second) = (walk(a)?, walk(b)?);
        let (plane_a, plane_b) = (
            sections.get(a).ok_or_else(internal)?.plane.origin(),
            sections.get(b).ok_or_else(internal)?.plane.origin(),
        );
        let samples = |w: &Walk| -> Vec<Vec3> {
            w.pieces
                .iter()
                .flat_map(|p| (0..SAMPLES).map(|i| p.sample(f64::from(i) / f64::from(SAMPLES))))
                .collect()
        };
        let ahead = samples(second)
            .iter()
            .all(|p| (*p - plane_a).dot(first.forward) > tol);
        let behind = samples(first)
            .iter()
            .all(|p| (*p - plane_b).dot(second.forward) < -tol);
        if !(ahead && behind) {
            return Err(refuse(
                &[a, b],
                format!(
                    "sections {} and {} coincide or their planes cross within the profiles; each profile has to lie wholly on one side of the plane of the next",
                    a + 1,
                    b + 1
                ),
            ));
        }
    }

    // The curves of each column on one degree and one knot vector.
    let mut columns: Vec<Vec<NurbsCurve>> = Vec::with_capacity(width);
    for j in 0..width {
        let curves: Vec<NurbsCurve> = sections_walked
            .iter()
            .filter_map(|w| w.pieces.get(j).map(|p| p.curve.clone()))
            .collect();
        if curves.len() != count {
            return Err(internal());
        }
        let made = compatible(&curves)?;
        // The ends are the vertices, to the bit.
        let mut pinned_curves = Vec::with_capacity(count);
        for (curve, w) in made.iter().zip(&sections_walked) {
            let p = w.pieces.get(j).ok_or_else(internal)?;
            pinned_curves.push(pinned(curve, p.start, p.end)?);
        }
        columns.push(pinned_curves);
    }

    // How far the sections are apart: the mean distance between matching
    // control points, the same for every column.
    let mut steps = Vec::with_capacity(pairs.len());
    for &(a, b) in &pairs {
        let (mut sum, mut terms) = (0.0, 0_usize);
        for column in &columns {
            let (Some(first), Some(second)) = (column.get(a), column.get(b)) else {
                return Err(internal());
            };
            for (p, q) in first.control_points().iter().zip(second.control_points()) {
                sum += p.distance(*q);
                terms += 1;
            }
        }
        let step = sum / terms.max(1) as f64;
        if !(step.is_finite() && step > tol) {
            return Err(refuse(
                &[a, b],
                format!("sections {} and {} are in one place", a + 1, b + 1),
            ));
        }
        steps.push(step);
    }
    let bands = bands(count, &steps, options)?;

    let mut b = Builder {
        body: Body::new(BodyKind::Solid),
        faces: Vec::new(),
    };
    // Vertices and edges of the sections that bound a band.
    let mut rings: Vec<Option<(Vec<VertexId>, Vec<EdgeId>)>> = (0..count).map(|_| None).collect();
    for index in 0..count {
        if !bands
            .iter()
            .any(|band| band.from == index || band.to == index)
        {
            continue;
        }
        let w = walk(index)?;
        let vertices: Vec<VertexId> = w
            .pieces
            .iter()
            .map(|p| b.body.add_vertex(p.start))
            .collect();
        let after = vertices.iter().cycle().skip(1);
        let edges: Vec<EdgeId> = w
            .pieces
            .iter()
            .zip(vertices.iter().zip(after))
            .map(|(p, (&here, &there))| b.body.add_edge(p.edge.clone(), p.range, here, there))
            .collect();
        if let Some(slot) = rings.get_mut(index) {
            *slot = Some((vertices, edges));
        }
    }
    let ring = |k: usize| rings.get(k).and_then(Option::as_ref).ok_or_else(internal);
    let coedge = |edge: EdgeId, reversed: bool| Coedge { edge, reversed };

    let mut shell = Vec::new();
    for band in &bands {
        let (low, high) = (ring(band.from)?, ring(band.to)?);
        // The edges between the sections, one per vertex.
        let mut rails = Vec::with_capacity(width);
        if !tube {
            for j in 0..width {
                let per_section: Vec<Vec3> = sections_walked
                    .iter()
                    .filter_map(|w| w.pieces.get(j).map(|p| p.start))
                    .collect();
                let mut control = band.points(&per_section);
                let (Some(&from), Some(&to)) = (low.0.get(j), high.0.get(j)) else {
                    return Err(internal());
                };
                // The ends are the vertices themselves.
                if let (Some(first), Some(start)) =
                    (control.first_mut(), per_section.get(band.from))
                {
                    *first = *start;
                }
                if let (Some(last), Some(end)) = (control.last_mut(), per_section.get(band.to)) {
                    *last = *end;
                }
                let (curve, range) = match straight(&control, tol / 8.0) {
                    Some(line) => line,
                    None if band.degree == 1 => {
                        return Err(refuse(
                            &[band.from, band.to],
                            format!(
                                "sections {} and {} touch: a corner of one lies on the matching corner of the other",
                                band.from + 1,
                                band.to + 1
                            ),
                        ));
                    }
                    None => {
                        let curve =
                            NurbsCurve::polynomial(band.degree, band.knots.clone(), control)?;
                        let range = curve.domain();
                        (Curve3::Nurbs(curve), range)
                    }
                };
                rails.push(b.body.add_edge(curve, range, from, to));
            }
        }
        for j in 0..width {
            let (Some(&bottom), Some(&top)) = (low.1.get(j), high.1.get(j)) else {
                return Err(internal());
            };
            let (first, second) = (
                walk(band.from)?.pieces.get(j).ok_or_else(internal)?,
                walk(band.to)?.pieces.get(j).ok_or_else(internal)?,
            );
            let forward = walk(band.from)?.forward;
            // Seen from outside: along the bottom, up, back along the
            // top, down.
            let loops = if tube {
                vec![vec![coedge(bottom, false)], vec![coedge(top, true)]]
            } else {
                let (Some(&here), Some(&there)) = (rails.get(j), rails.get((j + 1) % width)) else {
                    return Err(internal());
                };
                vec![vec![
                    coedge(bottom, false),
                    coedge(there, false),
                    coedge(top, true),
                    coedge(here, true),
                ]]
            };
            let analytic = match (&first.arc, &second.arc, band.degree) {
                (Some(a), Some(c), 1) => round(a, c, forward, tolerance),
                _ => None,
            };
            let (surface, reversed) = match analytic {
                Some((surface, outward)) => (surface, !outward),
                None if tube => return Err(internal()),
                None => {
                    let column = columns.get(j).ok_or_else(internal)?;
                    let surface = skin(column, band)?;
                    let sections_named = [band.from, band.to];
                    (flat_or(surface, tol, &sections_named)?, false)
                }
            };
            // Named after the column's curves up to the section the face
            // ends at.
            let reach = if band.to > band.from {
                band.to
            } else {
                count - 1
            };
            let mut sources: Vec<u64> = sections_walked
                .iter()
                .take(reach + 1)
                .filter_map(|w| w.pieces.get(j).map(|p| p.source))
                .collect();
            if band.to < band.from {
                sources.push(second.source);
            }
            shell.push(b.face(surface, reversed, loops, Role::Side, sources));
        }
    }

    if !options.closed {
        for (index, role) in [(0, Role::StartCap), (count - 1, Role::EndCap)] {
            let (w, (_, edges)) = (walk(index)?, ring(index)?);
            let plane = &sections.get(index).ok_or_else(internal)?.plane;
            // The start cap looks back along the loft, the end cap along it.
            let outward = if index == 0 { -w.forward } else { w.forward };
            let frame = if outward.dot(plane.z()) > 0.0 {
                *plane
            } else {
                plane
                    .reversed_at(plane.origin())
                    .ok_or_else(|| refuse(&[index], "the plane of the section is not finite"))?
            };
            let around: Vec<Coedge> = if index == 0 {
                edges.iter().rev().map(|e| coedge(*e, true)).collect()
            } else {
                edges.iter().map(|e| coedge(*e, false)).collect()
            };
            let mut sources: Vec<u64> = w.pieces.iter().map(|p| p.source).collect();
            sources.sort_unstable();
            sources.dedup();
            shell.push(b.face(
                Surface::Plane(Plane::new(frame)),
                false,
                vec![around],
                role,
                sources,
            ));
        }
    }
    b.body.add_shell(shell);

    let mut defects = check(&b.body, tolerance);
    if !defects.is_empty() {
        defects.truncate(MAX_REPORTED);
        return Err(KernelError::InvalidBody {
            operation: "loft",
            defects,
        });
    }
    Ok(Lofted {
        body: b.body,
        faces: b.faces,
    })
}

/// True when the loft is a row of cylinders and cones: whole circles
/// about one axis, joined by straight lines.
fn tube(outlines: &[Outline], options: &Options, tolerance: &Tolerance) -> bool {
    if options.closed || !(options.ruled || outlines.len() == 2) {
        return false;
    }
    let mut circles = Vec::with_capacity(outlines.len());
    for outline in outlines {
        match outline {
            Outline::Whole { center, axis, .. } => circles.push((*center, *axis)),
            Outline::Pieces(_) => return false,
        }
    }
    circles
        .iter()
        .zip(circles.iter().skip(1))
        .all(|((c0, a0), (c1, a1))| {
            let offset = *c1 - *c0;
            a0.cross(*a1).length() <= tolerance.angular
                && a0.dot(*a1) > 0.0
                && (offset - *a0 * offset.dot(*a0)).length() <= tolerance.linear
        })
}

/// The B-spline surface through the curves of one column over one band:
/// u along the curves, v along the loft.
pub(crate) fn skin(column: &[NurbsCurve], band: &Band) -> Result<NurbsSurface, KernelError> {
    let first = column.first().ok_or_else(internal)?;
    let count_u = first.control_points().len();
    let count_v = band.mix.len();
    let mut control = Vec::with_capacity(count_u * count_v);
    let mut weights = Vec::with_capacity(count_u * count_v);
    for i in 0..count_u {
        let per_section: Vec<(Vec3, f64)> = column
            .iter()
            .filter_map(|c| Some((*c.control_points().get(i)?, *c.weights().get(i)?)))
            .collect();
        if per_section.len() != column.len() {
            return Err(internal());
        }
        for (p, w) in band.weighted(&per_section) {
            // Weights that are 1 but for rounding are 1.
            let w = if (w - 1.0).abs() <= 64.0 * f64::EPSILON {
                1.0
            } else {
                w
            };
            if !(w.is_finite() && w > 0.0) {
                return Err(KernelError::Geometry(craft3d_geom::GeomError::Unsupported(
                    "a smooth loft through these arcs: the weights of the surface between the sections are not positive; make it ruled, or draw the sections with splines",
                )));
            }
            control.push(if w == 1.0 { p } else { p * (1.0 / w) });
            weights.push(w);
        }
    }
    Ok(NurbsSurface::new(
        first.degree(),
        band.degree,
        first.knots().to_vec(),
        band.knots.clone(),
        count_u,
        count_v,
        control,
        weights,
    )?)
}

/// The surface itself, or the plane it lies in when it lies in one; an
/// error when its normal vanishes somewhere (the side folds).
fn flat_or(surface: NurbsSurface, tol: f64, sections: &[usize]) -> Result<Surface, KernelError> {
    const GRID: u32 = 8;
    let mut middle = None;
    for i in 0..=GRID {
        for j in 0..=GRID {
            let (u, v) = (
                f64::from(i) / f64::from(GRID),
                f64::from(j) / f64::from(GRID),
            );
            let (point, du, dv) = surface.derivatives(u, v);
            let normal = du.cross(dv);
            let (speed_u, speed_v) = (du.length(), dv.length());
            let size = normal.length();
            if !size.is_finite() || size <= 1e-9 * speed_u * speed_v {
                let named: Vec<String> = sections.iter().map(|s| (s + 1).to_string()).collect();
                return Err(refuse(
                    sections,
                    format!(
                        "the side between sections {} folds onto itself (a curve of one runs along the loft, or matched curves run in opposite directions); change the start of a section",
                        named.join(" and ")
                    ),
                ));
            }
            if i == GRID / 2 && j == GRID / 2 {
                middle = Some((point, normal, du));
            }
        }
    }
    let Some((origin, normal, along)) = middle else {
        return Err(internal());
    };
    let Some(unit) = normal.normalized() else {
        return Err(internal());
    };
    let flat = surface
        .control_points()
        .iter()
        .all(|p| (*p - origin).dot(unit).abs() <= tol / 8.0);
    if flat && let Some(frame) = Frame::new(origin, unit, along) {
        return Ok(Surface::Plane(Plane::new(frame)));
    }
    Ok(Surface::Nurbs(surface))
}
