//! Parts cut from sheet stock (docs/design/cutting.md): a body recognised
//! as a plate ([`plate_of`]), its outline and holes in its own 2D frame
//! ([`Shape`], [`own_frame`]), the bounding rectangle it is cut from
//! ([`bounding_rect`]), whether two parts are the same part up to a rigid
//! motion ([`same_part`]) and the outline offset by a laser's kerf
//! ([`kerf`]).
//!
//! The 2D work is on the paths of `craft3d-drawing`, so every curve stays
//! what it is (a line a line, an arc an arc). Nesting (placing parts to
//! waste the least sheet) is not here: it is work for a separate program
//! on top of the platform.

mod kerf;
mod plate;
mod rect;
mod same;

pub use kerf::kerf;
pub use plate::{NotPlate, Plate, plate_of};
pub use rect::{Rect, bounding_rect, own_frame};
pub use same::{Prepared, prepare, same_part};

use craft3d_drawing::{Color, Drawing, Layer, Path, Point, Segment, Spline, Unit};
use craft3d_math::Portable;
use std::f64::consts::TAU;

/// What a mark on a part is: a cut that is not a loop (a formed louvre's
/// lance), or a line drawn for the shop (a bend line up or down, where a
/// forming tool lands).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum MarkKind {
    Cut,
    BendUp,
    BendDown,
    Form,
}

impl MarkKind {
    /// The kind seen from the other side of the sheet: a bend up is a bend
    /// down there.
    pub fn turned_over(self) -> Self {
        match self {
            MarkKind::BendUp => MarkKind::BendDown,
            MarkKind::BendDown => MarkKind::BendUp,
            other => other,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Mark {
    pub kind: MarkKind,
    pub path: Path,
}

/// A part as it is cut: its outline (the outer loop), its holes (the
/// inner loops) and its marks, in one 2D frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Shape {
    pub outline: Path,
    pub holes: Vec<Path>,
    pub marks: Vec<Mark>,
}

impl Shape {
    /// The shape of closed loops of one region: the loop that encloses
    /// the largest area is the outline, the others are its holes.
    pub fn from_loops(mut loops: Vec<Path>, marks: Vec<Mark>) -> Result<Shape, String> {
        if loops.iter().any(|p| !p.is_closed()) {
            return Err("a loop of the outline does not close".into());
        }
        let mut best: Option<(usize, f64)> = None;
        for (i, path) in loops.iter().enumerate() {
            let area = path.area().abs();
            if !area.is_finite() {
                return Err("a loop of the outline has no finite area".into());
            }
            if best.is_none_or(|(_, b)| area > b) {
                best = Some((i, area));
            }
        }
        let Some((at, _)) = best else {
            return Err("the outline has no loop".into());
        };
        if at >= loops.len() {
            return Err("the outline has no loop".into());
        }
        let outline = loops.remove(at);
        Ok(Shape {
            outline,
            holes: loops,
            marks,
        })
    }

    /// The area of the material: inside the outline, outside the holes.
    pub fn area(&self) -> f64 {
        self.outline.area().abs() - self.holes.iter().map(|h| h.area().abs()).sum::<f64>()
    }

    /// The length the cutter travels: the outline, the holes and the cut
    /// marks (lances).
    pub fn cut_length(&self) -> f64 {
        let cuts = self
            .marks
            .iter()
            .filter(|m| m.kind == MarkKind::Cut)
            .map(|m| m.path.clone());
        let paths: Vec<Path> = std::iter::once(self.outline.clone())
            .chain(self.holes.iter().cloned())
            .chain(cuts)
            .collect();
        one_layer(paths).length()
    }

    /// `[min x, min y, max x, max y]` of the outline (the holes and marks
    /// lie inside it); a spline by its furthest points.
    pub fn bbox(&self) -> Option<[f64; 4]> {
        rect::extent(&self.outline, [1.0, 0.0]).ok()
    }

    /// The shape moved by `motion`.
    pub fn moved(&self, motion: &Motion) -> Result<Shape, String> {
        Ok(Shape {
            outline: moved_path(&self.outline, motion)?,
            holes: self
                .holes
                .iter()
                .map(|h| moved_path(h, motion))
                .collect::<Result<_, _>>()?,
            marks: self
                .marks
                .iter()
                .map(|m| {
                    Ok(Mark {
                        kind: if motion.mirror {
                            m.kind.turned_over()
                        } else {
                            m.kind
                        },
                        path: moved_path(&m.path, motion)?,
                    })
                })
                .collect::<Result<_, String>>()?,
        })
    }
}

/// A drawing of one layer of `paths`, for its measurements.
fn one_layer(paths: Vec<Path>) -> Drawing {
    Drawing {
        unit: Unit::Millimeter,
        layers: vec![Layer {
            name: "outline".into(),
            color: Color::Black,
            dash: None,
            dot: false,
            paths,
        }],
    }
}

/// A rigid motion of the plane: the point mirrored in the x axis when
/// `mirror`, then turned by the angle whose cosine and sine are `cos` and
/// `sin`, then moved by `shift`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Motion {
    pub cos: f64,
    pub sin: f64,
    pub mirror: bool,
    pub shift: Point,
}

impl Motion {
    pub const IDENTITY: Motion = Motion {
        cos: 1.0,
        sin: 0.0,
        mirror: false,
        shift: [0.0, 0.0],
    };

    /// The turn that takes the unit direction `d` to +x, then `shift`.
    pub fn x_along(d: Point, shift: Point) -> Motion {
        Motion {
            cos: d[0],
            sin: -d[1],
            mirror: false,
            shift,
        }
    }

    pub fn vector(&self, v: Point) -> Point {
        let y = if self.mirror { -v[1] } else { v[1] };
        [
            self.cos * v[0] - self.sin * y,
            self.sin * v[0] + self.cos * y,
        ]
    }

    pub fn point(&self, p: Point) -> Point {
        let v = self.vector(p);
        [v[0] + self.shift[0], v[1] + self.shift[1]]
    }

    /// This motion followed by moving by `d`.
    pub fn then_shift(&self, d: Point) -> Motion {
        Motion {
            shift: [self.shift[0] + d[0], self.shift[1] + d[1]],
            ..*self
        }
    }
}

fn moved_spline(spline: &Spline, motion: &Motion) -> Result<Spline, String> {
    let control: Vec<Point> = spline
        .control_points()
        .iter()
        .map(|p| motion.point(*p))
        .collect();
    Spline::new(
        spline.degree(),
        spline.knots().to_vec(),
        control,
        spline.weights().map(<[f64]>::to_vec),
    )
    .map_err(|e| format!("a spline of the outline cannot be moved: {e}"))
}

fn moved_segment(segment: &Segment, motion: &Motion) -> Result<Segment, String> {
    let turn = |ccw: bool| ccw != motion.mirror;
    Ok(match segment {
        Segment::Line { to } => Segment::Line {
            to: motion.point(*to),
        },
        Segment::Arc {
            to,
            center,
            radius,
            ccw,
        } => Segment::Arc {
            to: motion.point(*to),
            center: motion.point(*center),
            radius: *radius,
            ccw: turn(*ccw),
        },
        Segment::EllipticArc {
            to,
            center,
            major,
            ratio,
            ccw,
        } => Segment::EllipticArc {
            to: motion.point(*to),
            center: motion.point(*center),
            major: motion.vector(*major),
            ratio: *ratio,
            ccw: turn(*ccw),
        },
        Segment::Spline(spline) => Segment::Spline(moved_spline(spline, motion)?),
    })
}

/// The path moved by `motion`. A mirror turns the way a chain runs; the
/// curves stay the curves they are.
pub fn moved_path(path: &Path, motion: &Motion) -> Result<Path, String> {
    Ok(match path {
        Path::Chain {
            start,
            segments,
            closed,
        } => Path::Chain {
            start: motion.point(*start),
            segments: segments
                .iter()
                .map(|s| moved_segment(s, motion))
                .collect::<Result<_, _>>()?,
            closed: *closed,
        },
        Path::Circle { center, radius } => Path::Circle {
            center: motion.point(*center),
            radius: *radius,
        },
        Path::Ellipse {
            center,
            major,
            ratio,
        } => Path::Ellipse {
            center: motion.point(*center),
            major: motion.vector(*major),
            ratio: *ratio,
        },
    })
}

fn sub(a: Point, b: Point) -> Point {
    [a[0] - b[0], a[1] - b[1]]
}

fn cross(a: Point, b: Point) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}

fn dot(a: Point, b: Point) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}

fn norm(a: Point) -> f64 {
    a[0].phypot(a[1])
}

/// The angle run from `a` to `b` turning one way: in `(0, 2π]` (a whole
/// turn when they are the same).
fn wrap(a: f64) -> f64 {
    let w = a.rem_euclid(TAU);
    if w > 0.0 { w } else { TAU }
}

/// The signed sweep of an arc from `from` to `to` about `center`.
fn arc_sweep(from: Point, to: Point, center: Point, ccw: bool) -> (f64, f64) {
    let a = sub(from, center);
    let b = sub(to, center);
    let start = a[1].patan2(a[0]);
    let end = b[1].patan2(b[0]);
    let sweep = if ccw {
        wrap(end - start)
    } else {
        -wrap(start - end)
    };
    (start, sweep)
}

/// The ellipse's own parameter of the point `p` on it.
fn ellipse_param(p: Point, center: Point, major: Point, ratio: f64) -> f64 {
    let d = sub(p, center);
    let minor = [-major[1] * ratio, major[0] * ratio];
    let x = dot(d, major) / dot(major, major);
    let y = dot(d, minor) / dot(minor, minor);
    y.patan2(x)
}

fn ellipse_point(center: Point, major: Point, ratio: f64, t: f64) -> Point {
    let minor = [-major[1] * ratio, major[0] * ratio];
    let (s, c) = (t.psin(), t.pcos());
    [
        center[0] + major[0] * c + minor[0] * s,
        center[1] + major[1] * c + minor[1] * s,
    ]
}

/// Points along `path`: every segment's ends, and `n` steps along each
/// curved piece, in the curve's own parameter (so that two congruent
/// paths give congruent points).
fn samples(path: &Path, n: usize, out: &mut Vec<Point>) {
    let n = n.max(1);
    let steps = |sweep: f64| (0..=n).map(move |i| sweep * i as f64 / n as f64);
    match path {
        Path::Circle { center, radius } => {
            for t in steps(TAU).take(n) {
                out.push([center[0] + radius * t.pcos(), center[1] + radius * t.psin()]);
            }
        }
        Path::Ellipse {
            center,
            major,
            ratio,
        } => {
            for t in steps(TAU).take(n) {
                out.push(ellipse_point(*center, *major, *ratio, t));
            }
        }
        Path::Chain {
            start, segments, ..
        } => {
            let mut at = *start;
            out.push(at);
            for segment in segments {
                match segment {
                    Segment::Line { to } => out.push(*to),
                    Segment::Arc {
                        to,
                        center,
                        radius,
                        ccw,
                    } => {
                        let (a0, sweep) = arc_sweep(at, *to, *center, *ccw);
                        for t in steps(sweep).skip(1).take(n.saturating_sub(1)) {
                            let a = a0 + t;
                            out.push([
                                center[0] + radius * a.pcos(),
                                center[1] + radius * a.psin(),
                            ]);
                        }
                        out.push(*to);
                    }
                    Segment::EllipticArc {
                        to,
                        center,
                        major,
                        ratio,
                        ccw,
                    } => {
                        let t0 = ellipse_param(at, *center, *major, *ratio);
                        let t1 = ellipse_param(*to, *center, *major, *ratio);
                        let sweep = if *ccw { wrap(t1 - t0) } else { -wrap(t0 - t1) };
                        for t in steps(sweep).skip(1).take(n.saturating_sub(1)) {
                            out.push(ellipse_point(*center, *major, *ratio, t0 + t));
                        }
                        out.push(*to);
                    }
                    Segment::Spline(spline) => {
                        let (lo, hi) = spline.domain();
                        for t in steps(hi - lo).skip(1).take(n.saturating_sub(1)) {
                            if let Ok(p) = spline.eval(lo + t) {
                                out.push(p);
                            }
                        }
                        out.push(spline.end());
                    }
                }
                at = segment.end();
            }
        }
    }
}

#[cfg(test)]
mod tests;
