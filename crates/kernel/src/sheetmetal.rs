//! Sheet metal: a flat base with flanges bent along straight bend lines,
//! each bend with an inner radius `R` and a K-factor `K`, folded by the
//! one fold mechanism (the linkage of [`crate::origami`]) and given its
//! thickness by the one offset mechanism (the plate of [`crate::plate`]);
//! its flat pattern measured by the one unfold mechanism
//! ([`crate::unfold::develop`] on each bend's neutral cylinder).
//! docs/design/sheetmetal.md has the method, the sources and the limits.
//!
//! **The flat pattern** is built first, in the plane of the base's sketch
//! (its x, y; `z` along its normal, the side a bend `up` goes to). The
//! base is a closed profile (lines, arcs, circles, splines; holes
//! allowed). A flange sits on a straight edge: one of the base's outer
//! edges, or the far edge (the *tip*) of an earlier flange. Along the
//! edge `s → e` (the parent flat on its left, `û` the unit vector to its
//! right, away from the parent) the pattern grows by the bend's strip,
//! `BA = θ (R + K T)` wide (the bend allowance: the length of the neutral
//! fibre over the angle θ; Machinery's Handbook, *Sheet metal* › *Bend
//! allowance*), and then the flange's flat, `L` long (measured from the
//! bend's tangent line) or `L − (R + T) tan(θ/2)` (measured from the
//! virtual sharp of the outside surfaces, the outside mould line).
//!
//! **Corners and reliefs.** Where two flanged edges of the base meet at a
//! corner `p` (a left turn of the outer loop), the two strips meet at `p`
//! only. A relief of size `r` cuts the base's corner: both edges end `r`
//! from `p` (so do the flanges on them), and the corner is replaced by a
//! notch from `a = p + r eₐ` to `b = p + r e_b` (`eₐ`, `e_b` the edges'
//! directions away from `p`): `rect` the parallelogram `a, a + r e_b, b`;
//! `round` the arc about `p` through the base; `tear` the far arc of the
//! circle tangent to both edges at `a` and `b`. A corner where the loop
//! turns right or goes straight on (the strips would overlap in the
//! pattern, or two bends would continue each other) is refused.
//!
//! **Folding.** Every flat (the base, each flange) is a body of a
//! [`Linkage`]; every bend a joint with the radius `R + K T` whose point
//! is where the strip starts on the neutral fibre (`z = T − K T` for a
//! bend up, whose inner skin is the top; `z = K T` for a bend down) and
//! whose angle is given (`+θ` up, `−θ` down): its motion is the turn
//! about the axis `R + K T` from that point after the shift back across
//! the strip by `θ (R + K T)` ([`crate::origami::Joint`]). The base stays
//! in place.
//!
//! **The part.** The folded bottom skin (`z = 0` of every flat, and per
//! bend the cylinder about the bend's axis of radius `T + R` for a bend
//! up, `R` for a bend down, over θ) is one sheet whose faces are tangent
//! along the tangent lines; [`crate::plate::plate`] gives it the
//! thickness `T` on the side its normal points to (into the material),
//! with square ends everywhere. A bend's skins are the cylinders `R` and
//! `R + T` about its axis exactly.

// `!(x > y)` is deliberate: unlike `x <= y` it is also true for NaN.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use crate::KernelError;
use crate::origami::{Angle, Folded, Joint, Linkage, fold_linkage};
use crate::plate::{Plate, Side, plate};
use crate::profile::{P2, Profile, Segment, Shape, cross, distance, dot, sub};
use crate::unfold::develop;
use craft3d_geom::{Circle, Curve3, Cylinder, Line, NurbsCurve, Plane, Surface};
use craft3d_math::{Frame, Tolerance, Transform, Vec3};
use craft3d_topo::{Body, BodyKind, Coedge, EdgeId, FaceId, VertexId};
use std::collections::BTreeMap;
use std::f64::consts::{PI, TAU};

/// The most flanges one part may have.
pub const MAX_FLANGES: usize = 1024;

/// Which way a bend turns the flange: towards the side the base's normal
/// points to, or away from it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
}

/// The straight edge a flange sits on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum On {
    /// The outer edge of the base made by the sketch curve with this id.
    Base(u64),
    /// The tip (the far edge) of the flange at this index.
    Tip(usize),
}

/// Where a flange's length is measured from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LengthFrom {
    /// From the bend's tangent line: the flange's flat is this long.
    Tangent,
    /// From the virtual sharp of the outside surfaces (the outside mould
    /// line): the flat is shorter by the outside setback `(R + T) tan(θ/2)`.
    Mould,
}

/// The shape of a corner relief.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReliefKind {
    Rect,
    Round,
    Tear,
}

impl ReliefKind {
    pub fn name(self) -> &'static str {
        match self {
            ReliefKind::Rect => "rect",
            ReliefKind::Round => "round",
            ReliefKind::Tear => "tear",
        }
    }
}

/// A corner relief: its shape and size (how far along each edge from the
/// corner it reaches).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Relief {
    pub kind: ReliefKind,
    pub size: f64,
}

/// One flange as the caller gives it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FlangeSpec {
    pub on: On,
    pub length: f64,
    /// The bend angle θ in radians, in (0, π]: 0 is flat, π folded back.
    pub angle: f64,
    pub direction: Direction,
    /// The inner radius `R` of the bend.
    pub radius: f64,
    /// The K-factor: the neutral fibre's distance from the inner surface
    /// over the thickness, in [0, 1].
    pub k: f64,
    pub from: LengthFrom,
}

/// A sheet-metal part as the caller gives it.
#[derive(Clone, Debug, PartialEq)]
pub struct Spec {
    /// The base flat, in the axes of `plane`.
    pub base: Profile,
    /// The plane of the base (its normal: where `up` goes).
    pub plane: Frame,
    pub thickness: f64,
    pub flanges: Vec<FlangeSpec>,
    /// The relief at every corner where two flanged edges of the base meet.
    pub relief: Option<Relief>,
}

/// A bend of the part, in the pattern's axes (the plane's x, y).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bend {
    /// The flat it starts from (0 is the base, `i + 1` flange `i`'s) and
    /// the flat it turns (`flange + 1`).
    pub parent: usize,
    pub child: usize,
    /// The bend angle, signed: positive up.
    pub angle: f64,
    pub radius: f64,
    pub k: f64,
    pub thickness: f64,
    /// The bend allowance `|θ| (R + K T)`: the strip's width.
    pub allowance: f64,
    /// The tangent line on the parent's side, from `start` to `end` with
    /// the parent on its left.
    pub start: [f64; 2],
    pub end: [f64; 2],
    /// Unit, square to the tangent line, towards the flange.
    pub across: [f64; 2],
    /// The length of the flange's flat beyond the strip.
    pub flat_length: f64,
}

impl Bend {
    /// The angle's size θ.
    pub fn theta(&self) -> f64 {
        self.angle.abs()
    }

    /// The radius of the neutral fibre, `R + K T`.
    pub fn neutral(&self) -> f64 {
        self.radius + self.k * self.thickness
    }

    /// The outside setback `(R + T) tan(θ/2)`: from a tangent line to the
    /// virtual sharp of the outside surfaces. `None` within the angular
    /// tolerance `angular` of θ = π, where the outside surfaces never meet
    /// (the rule a length to the mould line is refused by).
    pub fn setback(&self, angular: f64) -> Option<f64> {
        let half = 0.5 * self.theta();
        (half < 0.5 * PI - angular).then(|| (self.radius + self.thickness) * half.tan())
    }

    /// The bend deduction `2 (R + T) tan(θ/2) − BA` (DIN 6935's
    /// Ausgleichswert with the sign turned): how much shorter the flat is
    /// than the two flanges measured to the virtual sharp.
    pub fn deduction(&self, angular: f64) -> Option<f64> {
        self.setback(angular).map(|s| 2.0 * s - self.allowance)
    }

    /// The bend's centre line in the pattern: across the middle of its
    /// strip.
    pub fn centre_line(&self) -> ([f64; 2], [f64; 2]) {
        let h = 0.5 * self.allowance;
        let shift = |p: P2| [p[0] + h * self.across[0], p[1] + h * self.across[1]];
        (shift(self.start), shift(self.end))
    }

    /// The tangent line on the flange's side.
    pub fn far_line(&self) -> ([f64; 2], [f64; 2]) {
        let h = self.allowance;
        let shift = |p: P2| [p[0] + h * self.across[0], p[1] + h * self.across[1]];
        (shift(self.start), shift(self.end))
    }
}

/// A flat of the pattern: its outline in the pattern's axes, and the
/// motion that folds it (in the pattern's axes, `z` along the normal).
#[derive(Clone, Debug, PartialEq)]
pub struct Flat {
    pub outer: Vec<Segment>,
    pub holes: Vec<Vec<Segment>>,
    pub transform: Transform,
}

/// A corner of the base where two flanged edges meet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Corner {
    pub point: [f64; 2],
    /// The bends on the edge that arrives at the corner and on the one
    /// that leaves it.
    pub bends: [usize; 2],
    pub relief: Option<Relief>,
}

/// What a face of the folded sheet is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SheetFace {
    Flat(usize),
    Bend(usize),
}

/// A sheet-metal part: its pattern, its folded bottom skin and the solid.
#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    pub plane: Frame,
    pub thickness: f64,
    pub flats: Vec<Flat>,
    pub bends: Vec<Bend>,
    pub corners: Vec<Corner>,
    /// The folded bottom skin: one face per flat and per bend.
    pub sheet: Body,
    pub sheet_faces: Vec<SheetFace>,
    pub solid: Plate,
}

fn refuse(flange: Option<usize>, reason: impl Into<String>) -> KernelError {
    KernelError::SheetMetal {
        flange,
        reason: reason.into(),
    }
}

fn add(a: P2, b: P2) -> P2 {
    [a[0] + b[0], a[1] + b[1]]
}

fn scale(a: P2, s: f64) -> P2 {
    [a[0] * s, a[1] * s]
}

fn unit(a: P2) -> Option<P2> {
    let l = a[0].hypot(a[1]);
    (l.is_finite() && l > 0.0).then(|| [a[0] / l, a[1] / l])
}

fn lift(p: P2) -> Vec3 {
    Vec3::new(p[0], p[1], 0.0)
}

fn line(start: P2, end: P2) -> Segment {
    Segment {
        source: 0,
        piece: 0,
        reversed: false,
        start,
        end,
        shape: Shape::Line,
    }
}

/// Builds the part `spec` describes (see the module documentation).
///
/// **Errors.** [`KernelError::SheetMetal`] with the flange it is about and
/// why: a size that is not one (a thickness or a length not more than the
/// tolerance, a radius below zero, a K-factor outside [0, 1], an angle
/// outside (0, 180] degrees), a radius within the tolerance of zero (a
/// sharp bend, unsupported), an edge that is not straight or carries two
/// flanges, a flange on the tip of a later one, flanged edges that meet
/// at a reflex corner or in line, a relief that does not fit its edges;
/// and whatever the fold or the plate refuses.
pub fn build(spec: &Spec, tolerance: &Tolerance) -> Result<Part, KernelError> {
    let tol = tolerance.linear;
    if !(tol.is_finite() && tol > 0.0) {
        return Err(KernelError::InvalidSize {
            name: "tolerance",
            value: tol,
        });
    }
    let t = spec.thickness;
    if !(t.is_finite() && t > tol) {
        return Err(KernelError::InvalidSize {
            name: "thickness",
            value: t,
        });
    }
    if spec.flanges.len() > MAX_FLANGES {
        return Err(refuse(
            None,
            format!("a part takes at most {MAX_FLANGES} flanges"),
        ));
    }
    for (i, f) in spec.flanges.iter().enumerate() {
        check_flange(i, f, tol)?;
    }
    if let Some(relief) = spec.relief
        && !(relief.size.is_finite() && relief.size > tol)
    {
        return Err(refuse(
            None,
            format!(
                "a relief's size has to be more than the tolerance, got {}",
                relief.size
            ),
        ));
    }

    // The base's flanged edges and the flanges' parents.
    let outer = &spec.base.outer.segments;
    let n = outer.len();
    let mut on_segment: BTreeMap<usize, usize> = BTreeMap::new();
    let mut on_tip: BTreeMap<usize, usize> = BTreeMap::new();
    for (i, f) in spec.flanges.iter().enumerate() {
        match f.on {
            On::Base(id) => {
                let mut found = outer
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| s.source == id)
                    .map(|(k, _)| k);
                let Some(k) = found.next() else {
                    let in_hole = spec
                        .base
                        .holes
                        .iter()
                        .any(|h| h.segments.iter().any(|s| s.source == id));
                    return Err(refuse(
                        Some(i),
                        if in_hole {
                            format!(
                                "the curve {id} bounds a hole of the base; a flange on a hole's edge is not supported"
                            )
                        } else {
                            format!("the curve {id} is no edge of the base's outline")
                        },
                    ));
                };
                if found.next().is_some() {
                    return Err(refuse(
                        Some(i),
                        format!(
                            "the curve {id} is cut into several edges of the base; a flange needs one straight edge"
                        ),
                    ));
                }
                if !matches!(outer.get(k).map(|s| &s.shape), Some(Shape::Line)) {
                    return Err(refuse(
                        Some(i),
                        format!("the curve {id} is not straight; a flange sits on a straight edge"),
                    ));
                }
                if on_segment.insert(k, i).is_some() {
                    return Err(refuse(
                        Some(i),
                        format!("the edge of curve {id} has a flange already"),
                    ));
                }
            }
            On::Tip(j) => {
                if j >= i {
                    return Err(refuse(
                        Some(i),
                        format!(
                            "a flange sits on the tip of an earlier flange; flange {j} comes later"
                        ),
                    ));
                }
                if on_tip.insert(j, i).is_some() {
                    return Err(refuse(
                        Some(i),
                        format!("the tip of flange {j} has a flange already"),
                    ));
                }
            }
        }
    }

    // Corners between two flanged edges, and the base's outline with its
    // reliefs.
    let mut corners: Vec<Corner> = Vec::new();
    let mut corner_at_end: BTreeMap<usize, usize> = BTreeMap::new();
    for k in 0..n {
        let next = (k + 1) % n;
        let (Some(&fa), Some(&fb)) = (on_segment.get(&k), on_segment.get(&next)) else {
            continue;
        };
        let (Some(a), Some(b)) = (outer.get(k), outer.get(next)) else {
            continue;
        };
        let (Some(da), Some(db)) = (unit(sub(a.end, a.start)), unit(sub(b.end, b.start))) else {
            return Err(refuse(Some(fa), "an edge with a flange has no length"));
        };
        let turn = cross(da, db);
        if !(turn > tolerance.angular) {
            return Err(refuse(
                Some(fb),
                if dot(da, db) > 0.0 && turn.abs() <= tolerance.angular {
                    format!(
                        "the flanges {fa} and {fb} sit on edges in line; make them one flange on one edge"
                    )
                } else {
                    format!(
                        "the flanges {fa} and {fb} meet at a reflex corner of the base, where their bend strips overlap in the flat pattern; not supported (owed: a mitre between the flanges)"
                    )
                },
            ));
        }
        corner_at_end.insert(k, corners.len());
        corners.push(Corner {
            point: a.end,
            bends: [fa, fb],
            relief: spec.relief,
        });
    }
    let r = spec.relief.map_or(0.0, |relief| relief.size);
    let mut base_outer: Vec<Segment> = Vec::with_capacity(n + 2 * corners.len());
    let mut base_edge: BTreeMap<usize, usize> = BTreeMap::new();
    for (k, seg) in outer.iter().enumerate() {
        let before = (k + n - 1) % n;
        let mut s = seg.clone();
        let d = unit(sub(seg.end, seg.start));
        if spec.relief.is_some() {
            let len = distance(seg.start, seg.end);
            let cut_start = corner_at_end.contains_key(&before);
            let cut_end = corner_at_end.contains_key(&k);
            let cuts = f64::from(u8::from(cut_start)) + f64::from(u8::from(cut_end));
            if cuts > 0.0 && !(len - cuts * r > tol) {
                return Err(refuse(
                    on_segment.get(&k).copied(),
                    format!(
                        "a relief of size {r} does not fit the edge of curve {} ({len} long)",
                        seg.source
                    ),
                ));
            }
            if let Some(d) = d {
                if cut_start {
                    s.start = add(seg.start, scale(d, r));
                }
                if cut_end {
                    s.end = sub(seg.end, scale(d, r));
                }
            }
        }
        if on_segment.contains_key(&k) {
            base_edge.insert(k, base_outer.len());
        }
        base_outer.push(s);
        if let (Some(&c), Some(relief)) = (corner_at_end.get(&k), spec.relief) {
            let next = outer.get((k + 1) % n);
            let (Some(next), Some(d)) = (next, d) else {
                continue;
            };
            let Some(dn) = unit(sub(next.end, next.start)) else {
                continue;
            };
            let p = corners.get(c).map_or(seg.end, |c| c.point);
            base_outer.extend(notch(p, scale(d, -1.0), dn, relief)?);
        }
    }
    let base_holes: Vec<Vec<Segment>> =
        spec.base.holes.iter().map(|h| h.segments.clone()).collect();
    if let Some(relief) = spec.relief {
        reliefs_fit(&base_outer, &base_holes, relief, tolerance)?;
    }

    // The bends and the flanges' flats, in order: a parent comes first.
    let mut bends: Vec<Bend> = Vec::with_capacity(spec.flanges.len());
    let mut flats: Vec<Flat> = Vec::with_capacity(1 + spec.flanges.len());
    flats.push(Flat {
        outer: base_outer,
        holes: base_holes,
        transform: Transform::IDENTITY,
    });
    for (i, f) in spec.flanges.iter().enumerate() {
        let (parent, start, end) = match f.on {
            On::Base(_) => {
                let k = on_segment
                    .iter()
                    .find(|(_, fl)| **fl == i)
                    .map(|(k, _)| *k)
                    .ok_or_else(|| refuse(Some(i), "a flange lost its edge"))?;
                let seg = base_edge
                    .get(&k)
                    .and_then(|&b| flats.first().and_then(|f| f.outer.get(b)))
                    .ok_or_else(|| refuse(Some(i), "a flange lost its edge"))?;
                (0, seg.start, seg.end)
            }
            On::Tip(j) => {
                let parent = bends
                    .get(j)
                    .ok_or_else(|| refuse(Some(i), "a flange's parent is missing"))?;
                let (a, b) = tip(parent);
                (j + 1, a, b)
            }
        };
        let d = unit(sub(end, start)).ok_or_else(|| refuse(Some(i), "the edge has no length"))?;
        let across = [d[1], -d[0]];
        let theta = f.angle;
        let neutral = f.radius + f.k * t;
        let allowance = theta * neutral;
        let flat_length = match f.from {
            LengthFrom::Tangent => f.length,
            LengthFrom::Mould => {
                let half = 0.5 * theta;
                if !(half < 0.5 * PI - tolerance.angular) {
                    return Err(refuse(
                        Some(i),
                        "a length to the outside mould line needs an angle below 180 degrees (at 180 the outside surfaces never meet); measure it from the tangent line",
                    ));
                }
                f.length - (f.radius + t) * half.tan()
            }
        };
        if !(flat_length > tol) {
            return Err(refuse(
                Some(i),
                format!(
                    "the flange's flat would be {flat_length} long: its length is shorter than the outside setback (R + T) tan(θ/2)"
                ),
            ));
        }
        let signed = match f.direction {
            Direction::Up => theta,
            Direction::Down => -theta,
        };
        let bend = Bend {
            parent,
            child: i + 1,
            angle: signed,
            radius: f.radius,
            k: f.k,
            thickness: t,
            allowance,
            start,
            end,
            across,
            flat_length,
        };
        let a0 = add(start, scale(across, allowance));
        let a1 = add(end, scale(across, allowance));
        let b0 = add(a0, scale(across, flat_length));
        let b1 = add(a1, scale(across, flat_length));
        flats.push(Flat {
            outer: vec![line(a1, a0), line(a0, b0), line(b0, b1), line(b1, a1)],
            holes: Vec::new(),
            transform: Transform::IDENTITY,
        });
        bends.push(bend);
    }

    pattern_clear(&flats, &bends, tolerance)?;
    let folded = fold(&flats, &bends, tolerance)?;
    for (flat, transform) in flats.iter_mut().zip(&folded.transforms) {
        flat.transform = *transform;
    }
    let (sheet, sheet_faces) = folded_sheet(
        spec,
        &flats,
        &bends,
        &base_edge_list(&base_edge, &on_segment),
        tolerance,
    )?;
    let faces: Vec<FaceId> = (0..sheet.faces.len())
        .filter_map(|f| u32::try_from(f).ok().map(FaceId))
        .collect();
    let solid = plate(&sheet, &faces, t, Side::Out, &BTreeMap::new(), tolerance)?;
    Ok(Part {
        plane: spec.plane,
        thickness: t,
        flats,
        bends,
        corners,
        sheet,
        sheet_faces,
        solid,
    })
}

/// A segment of an outline as the sketch curve the profile detection
/// reads.
fn sketch_curve(seg: &Segment) -> crate::profile::Curve2 {
    use crate::profile::Curve2;
    match &seg.shape {
        Shape::Line => Curve2::Line {
            start: seg.start,
            end: seg.end,
        },
        Shape::Arc { center, .. } => {
            let (start, end) = if seg.reversed {
                (seg.end, seg.start)
            } else {
                (seg.start, seg.end)
            };
            Curve2::Arc {
                center: *center,
                start,
                end,
            }
        }
        Shape::Circle { center, radius } => Curve2::Circle {
            center: *center,
            radius: *radius,
        },
        Shape::Spline(spline) => Curve2::Spline(spline.clone()),
    }
}

/// The flat pattern does not overlap itself: its outline (every flat's
/// and strip's edges that no strip continues, and the holes), run through
/// the profile detection of the sketch, is one region with all the holes
/// and the area the pieces add up to. Two pieces that cross, touch or lie
/// along each other anywhere but where the pattern joins them (a bend's
/// tangent lines, the corner point two strips share) make two curves of
/// the outline cross or touch away from a shared end, or more than two
/// ends meet at a point, which the detection names; a piece that lies
/// wholly inside another leaves the area short. Either fails the part,
/// naming the base or the flanges whose pieces meet; anything else the
/// detection refuses fails it too (fail closed).
fn pattern_clear(flats: &[Flat], bends: &[Bend], tolerance: &Tolerance) -> Result<(), KernelError> {
    use crate::profile::{SketchCurve, detect_profiles};
    let chains = tagged_outline(flats, bends, tolerance.linear);
    let tagged: Vec<&(Segment, usize)> = chains.iter().flatten().collect();
    let curves: Vec<SketchCurve> = tagged
        .iter()
        .enumerate()
        .map(|(i, (seg, _))| SketchCurve {
            id: i as u64 + 1,
            curve: sketch_curve(seg),
        })
        .collect();
    let owner = |id: u64| {
        usize::try_from(id)
            .ok()
            .and_then(|i| i.checked_sub(1))
            .and_then(|i| tagged.get(i))
            .map(|(_, flat)| *flat)
    };
    let name = |flat: usize| match flat.checked_sub(1) {
        None => "the base".to_string(),
        Some(f) => format!("flange {f}"),
    };
    let meeting = |ids: &[u64]| {
        let mut owners: Vec<usize> = ids.iter().filter_map(|&id| owner(id)).collect();
        owners.sort_unstable();
        owners.dedup();
        let flange = owners.last().and_then(|f| f.checked_sub(1));
        let names: Vec<String> = owners.iter().map(|&f| name(f)).collect();
        let who = match names.as_slice() {
            [one] => format!("{one} runs into itself"),
            [a, b] => format!("{a} and {b} cross or touch"),
            _ => format!("{} cross or touch", names.join(", ")),
        };
        refuse(
            flange,
            format!(
                "the flat pattern overlaps itself: {who} in the flat, so it cannot be cut from one sheet (shorten or move the flanges, or bend them from other edges)"
            ),
        )
    };
    let profiles = match detect_profiles(&curves, tolerance) {
        Ok(profiles) => profiles,
        Err(KernelError::CrossingCurves { curves }) => return Err(meeting(&curves)),
        Err(KernelError::BranchingProfile { curves }) => return Err(meeting(&curves)),
        Err(other) => {
            return Err(refuse(
                None,
                format!(
                    "the flat pattern's outline could not be checked for overlapping itself: {other}"
                ),
            ));
        }
    };
    let holes: usize = flats.iter().map(|f| f.holes.len()).sum();
    let area = pattern_area(flats, bends);
    match profiles.as_slice() {
        [one]
            if one.holes.len() == holes
                && (one.area() - area).abs() <= tolerance.linear * area.abs().sqrt() * 8.0 =>
        {
            Ok(())
        }
        _ => Err(refuse(
            None,
            format!(
                "the flat pattern overlaps itself: its outline bounds {} region(s) where its pieces make one of area {area}, so a piece lies on another in the flat",
                profiles.len()
            ),
        )),
    }
}

/// The base's outline with its reliefs is still one region with the same
/// holes: the profile detection of the sketch, run on its curves, finds one
/// profile whose outer loop has the outline's area (a notch that crosses
/// another edge or a hole, or a tear relief whose circle grows past the
/// base at a corner near 180 degrees, does not).
fn reliefs_fit(
    outer: &[Segment],
    holes: &[Vec<Segment>],
    relief: Relief,
    tolerance: &Tolerance,
) -> Result<(), KernelError> {
    use crate::profile::{SketchCurve, detect_profiles};
    let curves: Vec<SketchCurve> = outer
        .iter()
        .chain(holes.iter().flatten())
        .enumerate()
        .map(|(i, seg)| SketchCurve {
            id: i as u64 + 1,
            curve: sketch_curve(seg),
        })
        .collect();
    let refused = || {
        refuse(
            None,
            format!(
                "the {} reliefs of size {} cut across the base's outline or a hole of it (a tear relief's circle, of radius size × tan(corner/2), grows without bound as a corner nears 180 degrees): make them smaller",
                relief.kind.name(),
                relief.size
            ),
        )
    };
    let profiles = detect_profiles(&curves, tolerance).map_err(|_| refused())?;
    let area = crate::profile::Loop2 {
        segments: outer.to_vec(),
    }
    .signed_area();
    match profiles.as_slice() {
        [one]
            if one.holes.len() == holes.len()
                && (one.outer.signed_area() - area).abs()
                    <= tolerance.linear * area.abs().sqrt() * 8.0 =>
        {
            Ok(())
        }
        _ => Err(refused()),
    }
}

/// Per flange on the base, the index of its edge in the base's new outer
/// loop.
fn base_edge_list(
    base_edge: &BTreeMap<usize, usize>,
    on_segment: &BTreeMap<usize, usize>,
) -> BTreeMap<usize, usize> {
    on_segment
        .iter()
        .filter_map(|(k, &flange)| base_edge.get(k).map(|&b| (flange, b)))
        .collect()
}

fn check_flange(i: usize, f: &FlangeSpec, tol: f64) -> Result<(), KernelError> {
    if !(f.length.is_finite() && f.length > tol) {
        return Err(refuse(
            Some(i),
            format!(
                "a flange's length has to be more than the tolerance, got {}",
                f.length
            ),
        ));
    }
    if !(f.angle.is_finite() && f.angle > 0.0 && f.angle <= PI) {
        return Err(refuse(
            Some(i),
            format!(
                "a bend's angle is more than 0 and at most 180 degrees, got {}",
                f.angle.to_degrees()
            ),
        ));
    }
    if !(f.radius.is_finite() && f.radius >= 0.0) {
        return Err(refuse(
            Some(i),
            format!(
                "a bend's inner radius cannot be below zero, got {}",
                f.radius
            ),
        ));
    }
    if !(f.radius > tol) {
        return Err(refuse(
            Some(i),
            "a sharp bend (an inner radius within the tolerance of zero) is not supported: its inner skin would be an edge (owed); give the radius the tooling makes",
        ));
    }
    if !(f.k.is_finite() && (0.0..=1.0).contains(&f.k)) {
        return Err(refuse(
            Some(i),
            format!(
                "a K-factor is the neutral fibre's place across the thickness, from 0 (the inner surface) to 1 (the outer), got {}",
                f.k
            ),
        ));
    }
    Ok(())
}

/// The tip of a flange's flat: from `b0` to `b1`, the flange on its left.
fn tip(bend: &Bend) -> (P2, P2) {
    let h = bend.allowance + bend.flat_length;
    (
        add(bend.start, scale(bend.across, h)),
        add(bend.end, scale(bend.across, h)),
    )
}

/// The notch of a relief at the corner `p` of the base, from
/// `a = p + r ea` to `b = p + r eb` through the base (see the module
/// documentation). `ea` and `eb` are unit, along the two edges away from
/// `p`; the base lies between them, counter-clockwise from `eb` to `ea`.
fn notch(p: P2, ea: P2, eb: P2, relief: Relief) -> Result<Vec<Segment>, KernelError> {
    let r = relief.size;
    let a = add(p, scale(ea, r));
    let b = add(p, scale(eb, r));
    Ok(match relief.kind {
        ReliefKind::Rect => {
            let q = add(a, scale(eb, r));
            vec![line(a, q), line(q, b)]
        }
        ReliefKind::Round => vec![Segment {
            source: 0,
            piece: 0,
            reversed: true,
            start: a,
            end: b,
            shape: Shape::Arc {
                center: p,
                radius: r,
            },
        }],
        ReliefKind::Tear => {
            // The corner's angle α between the edges; the circle tangent
            // to both at distance r from p has its centre on the bisector,
            // r / cos(α/2) from p, and the radius r tan(α/2).
            let alpha = dot(ea, eb).clamp(-1.0, 1.0).acos();
            let half = 0.5 * alpha;
            let bisector = unit(add(ea, eb)).ok_or_else(|| {
                refuse(
                    None,
                    "a relief at a corner whose edges run back on each other",
                )
            })?;
            let (c, rho) = (half.cos(), half.tan());
            if !(c > 1e-9 && rho.is_finite() && rho > 0.0) {
                return Err(refuse(None, "a tear relief at a corner of 180 degrees"));
            }
            vec![Segment {
                source: 0,
                piece: 0,
                reversed: true,
                start: a,
                end: b,
                shape: Shape::Arc {
                    center: add(p, scale(bisector, r / c)),
                    radius: r * rho,
                },
            }]
        }
    })
}

/// The flats folded by the bends: the base stays.
fn fold(flats: &[Flat], bends: &[Bend], tolerance: &Tolerance) -> Result<Folded, KernelError> {
    let mut joints = Vec::with_capacity(bends.len());
    let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    for flat in flats {
        for s in &flat.outer {
            for p in [s.start, s.end] {
                lo = [lo[0].min(p[0]), lo[1].min(p[1])];
                hi = [hi[0].max(p[0]), hi[1].max(p[1])];
            }
        }
    }
    let size = (hi[0] - lo[0]).hypot(hi[1] - lo[1]);
    for (i, b) in bends.iter().enumerate() {
        let d = unit(sub(b.end, b.start)).ok_or_else(|| refuse(Some(i), "a bend has no length"))?;
        let direction = lift(d);
        let t = b.thickness;
        // The strip starts on the neutral fibre: K T from the inner skin.
        let z = if b.angle > 0.0 { t - b.k * t } else { b.k * t };
        joints.push(Joint {
            left: b.parent,
            right: b.child,
            point: Vec3::new(b.start[0], b.start[1], z),
            direction,
            side: Vec3::Z.cross(direction),
            reach: distance(b.start, b.end),
            id: i as u64,
            radius: b.neutral(),
            across: lift(b.across),
        });
    }
    let link = Linkage {
        bodies: flats.len(),
        joints,
        size: if size.is_finite() && size > 0.0 {
            size
        } else {
            1.0
        },
    };
    let angles: Vec<Angle> = bends.iter().map(|b| Angle::Given(b.angle)).collect();
    fold_linkage(&link, &angles, 0, tolerance.linear, tolerance.angular)
        .map_err(|e| refuse(None, format!("the bends do not fold: {e}")))
}

/// The world transform of a flat: its fold, then the plane.
fn world(plane: &Frame, flat: &Flat) -> Transform {
    flat.transform.then(&Transform::from_frame(plane))
}

/// A loop's vertices and its edges (with whether the loop runs against
/// each).
type Ring = (Vec<VertexId>, Vec<(EdgeId, bool)>);

struct Builder {
    body: Body,
}

impl Builder {
    fn vertex(&mut self, p: Vec3) -> VertexId {
        self.body.add_vertex(p)
    }

    fn point(&self, v: VertexId) -> Option<Vec3> {
        usize::try_from(v.0)
            .ok()
            .and_then(|i| self.body.vertices.get(i))
            .map(|v| v.point)
    }

    fn line(&mut self, a: VertexId, b: VertexId) -> Result<EdgeId, KernelError> {
        let (Some(pa), Some(pb)) = (self.point(a), self.point(b)) else {
            return Err(refuse(None, "a vertex is missing"));
        };
        let l = Line::through(pa, pb)?;
        Ok(self
            .body
            .add_edge(Curve3::Line(l), (0.0, pa.distance(pb)), a, b))
    }

    /// The edge of a pattern segment carried by `w`, between the vertices
    /// where the loop enters and leaves it; and whether the loop runs
    /// against the edge.
    fn segment(
        &mut self,
        seg: &Segment,
        w: &Transform,
        ends: (VertexId, VertexId),
    ) -> Result<(EdgeId, bool), KernelError> {
        let frame_at = |c: P2| -> Result<Frame, KernelError> {
            Frame::new(w.point(lift(c)), w.vector(Vec3::Z), w.vector(Vec3::X))
                .ok_or_else(|| refuse(None, "a flat's plane is not finite"))
        };
        match &seg.shape {
            Shape::Line => Ok((self.line(ends.0, ends.1)?, false)),
            &Shape::Arc { center, radius } => {
                let (begin, sweep) = seg
                    .angles()
                    .ok_or_else(|| refuse(None, "an arc without angles"))?;
                if !(sweep > 0.0 && sweep < TAU) {
                    return Err(refuse(None, "an arc whose ends coincide"));
                }
                let circle = Curve3::Circle(Circle::new(frame_at(center)?, radius)?);
                let own = if seg.reversed { (ends.1, ends.0) } else { ends };
                Ok((
                    self.body
                        .add_edge(circle, (begin, begin + sweep), own.0, own.1),
                    seg.reversed,
                ))
            }
            &Shape::Circle { center, radius } => {
                let circle = Curve3::Circle(Circle::new(frame_at(center)?, radius)?);
                Ok((
                    self.body.add_edge(circle, (0.0, TAU), ends.0, ends.1),
                    seg.reversed,
                ))
            }
            Shape::Spline(spline) => {
                let control = spline.control.iter().map(|p| w.point(lift(*p))).collect();
                let curve = NurbsCurve::polynomial(spline.degree, spline.knots.clone(), control)?;
                let range = curve.domain();
                let own = if seg.reversed { (ends.1, ends.0) } else { ends };
                Ok((
                    self.body
                        .add_edge(Curve3::Nurbs(curve), range, own.0, own.1),
                    seg.reversed,
                ))
            }
        }
    }

    /// The edges of a closed loop of pattern segments carried by `w`: a
    /// vertex where each segment starts and an edge per segment, with
    /// whether the loop runs against it.
    fn ring(&mut self, segs: &[Segment], w: &Transform) -> Result<Ring, KernelError> {
        let vertices: Vec<VertexId> = segs
            .iter()
            .map(|s| self.vertex(w.point(lift(s.start))))
            .collect();
        let mut edges = Vec::with_capacity(segs.len());
        for (k, s) in segs.iter().enumerate() {
            let (Some(&a), Some(&b)) = (vertices.get(k), vertices.get((k + 1) % segs.len())) else {
                return Err(refuse(None, "a loop lost a vertex"));
            };
            edges.push(self.segment(s, w, (a, b))?);
        }
        Ok((vertices, edges))
    }
}

fn coedges(edges: &[(EdgeId, bool)]) -> Vec<Coedge> {
    edges
        .iter()
        .map(|&(edge, reversed)| Coedge { edge, reversed })
        .collect()
}

/// The folded bottom skin: a planar face per flat, a cylindrical one per
/// bend, sharing the tangent lines' edges.
fn folded_sheet(
    spec: &Spec,
    flats: &[Flat],
    bends: &[Bend],
    base_edge: &BTreeMap<usize, usize>,
    tolerance: &Tolerance,
) -> Result<(Body, Vec<SheetFace>), KernelError> {
    let mut b = Builder {
        body: Body::new(BodyKind::Sheet),
    };
    let plane = &spec.plane;
    let worlds: Vec<Transform> = flats.iter().map(|f| world(plane, f)).collect();
    let w_of = |i: usize| -> Result<Transform, KernelError> {
        worlds
            .get(i)
            .copied()
            .ok_or_else(|| refuse(None, "a flat is missing"))
    };
    // Per flat: its outer loop's vertices and edges.
    let mut rings: Vec<Ring> = Vec::with_capacity(flats.len());
    let mut hole_rings: Vec<Vec<(EdgeId, bool)>> = Vec::new();
    let base = flats.first().ok_or_else(|| refuse(None, "no base"))?;
    let w0 = w_of(0)?;
    rings.push(b.ring(&base.outer, &w0)?);
    for hole in &base.holes {
        hole_rings.push(b.ring(hole, &w0)?.1);
    }
    // The bends' arcs, and the flanges' flats in order. Flange i's flat
    // shares its tangent edge with bend i's face; its tip may be the
    // parent edge of a later bend.
    let mut bend_loops: Vec<Vec<(EdgeId, bool)>> = Vec::with_capacity(bends.len());
    for (i, bend) in bends.iter().enumerate() {
        // The parent edge: `s → e` in the parent's loop.
        let (ps, pe, parent_edge) = if bend.parent == 0 {
            let k = *base_edge
                .get(&i)
                .ok_or_else(|| refuse(Some(i), "a flange lost its edge"))?;
            let (vs, es) = rings.first().ok_or_else(|| refuse(None, "no base"))?;
            let n = vs.len();
            (
                *vs.get(k).ok_or_else(|| refuse(Some(i), "no vertex"))?,
                *vs.get((k + 1) % n)
                    .ok_or_else(|| refuse(Some(i), "no vertex"))?,
                *es.get(k).ok_or_else(|| refuse(Some(i), "no edge"))?,
            )
        } else {
            // A tip: segment 2 of the parent flange's loop, b0 → b1.
            let (vs, es) = rings
                .get(bend.parent)
                .ok_or_else(|| refuse(Some(i), "a flange's parent is missing"))?;
            (
                *vs.get(2).ok_or_else(|| refuse(Some(i), "no vertex"))?,
                *vs.get(3).ok_or_else(|| refuse(Some(i), "no vertex"))?,
                *es.get(2).ok_or_else(|| refuse(Some(i), "no edge"))?,
            )
        };
        let flat = flats
            .get(bend.child)
            .ok_or_else(|| refuse(Some(i), "a flange's flat is missing"))?;
        let wc = w_of(bend.child)?;
        let wp = w_of(bend.parent)?;
        let ring = b.ring(&flat.outer, &wc)?;
        let (Some(&a1), Some(&a0)) = (ring.0.first(), ring.0.get(1)) else {
            return Err(refuse(Some(i), "a flange's flat has no corners"));
        };
        let tangent = *ring
            .1
            .first()
            .ok_or_else(|| refuse(Some(i), "a flange's flat has no edges"))?;
        // The arcs at both ends, about the axis as the parent has it.
        let t = bend.thickness;
        let zc = if bend.angle > 0.0 {
            t + bend.radius
        } else {
            -bend.radius
        };
        let d = lift(unit(sub(bend.end, bend.start)).ok_or_else(|| refuse(Some(i), "no length"))?);
        let centre = Vec3::new(bend.start[0], bend.start[1], zc);
        let axis_world = wp.vector(d);
        let z = axis_world * (-bend.angle.signum());
        let c0 = wp.point(centre);
        let s0 = wp.point(lift(bend.start));
        let frame = Frame::new(c0, z, s0 - c0)
            .ok_or_else(|| refuse(Some(i), "a bend's axis is not finite"))?;
        let rb = zc.abs();
        let theta = bend.theta();
        let arc = |b: &mut Builder,
                   from: VertexId,
                   to: VertexId,
                   shift: f64|
         -> Result<EdgeId, KernelError> {
            let f = frame
                .at(frame.origin() + axis_world * shift)
                .ok_or_else(|| refuse(Some(i), "a bend's axis is not finite"))?;
            let circle = Curve3::Circle(Circle::new(f, rb)?);
            Ok(b.body.add_edge(circle, (0.0, theta), from, to))
        };
        let arc_s = arc(&mut b, ps, a0, 0.0)?;
        let arc_e = arc(&mut b, pe, a1, distance(bend.start, bend.end))?;
        bend_loops.push(vec![
            (arc_s, false),
            (tangent.0, !tangent.1),
            (arc_e, true),
            (parent_edge.0, !parent_edge.1),
        ]);
        rings.push(ring);
    }

    // Faces: the flats, then the bends.
    let mut faces: Vec<FaceId> = Vec::with_capacity(flats.len() + bends.len());
    let mut roles = Vec::with_capacity(flats.len() + bends.len());
    for (i, ((_, edges), w)) in rings.iter().zip(&worlds).enumerate() {
        let mut loops = vec![b.body.add_loop(coedges(edges))];
        if i == 0 {
            for hole in &hole_rings {
                loops.push(b.body.add_loop(coedges(hole)));
            }
        }
        let frame = w
            .frame(&Frame::WORLD)
            .ok_or_else(|| refuse(None, "a flat's plane is not finite"))?;
        faces.push(
            b.body
                .add_face(Surface::Plane(Plane::new(frame)), false, loops),
        );
        roles.push(SheetFace::Flat(i));
    }
    for (i, (bend, edges)) in bends.iter().zip(&bend_loops).enumerate() {
        let wp = w_of(bend.parent)?;
        let t = bend.thickness;
        let zc = if bend.angle > 0.0 {
            t + bend.radius
        } else {
            -bend.radius
        };
        let d = lift(unit(sub(bend.end, bend.start)).ok_or_else(|| refuse(Some(i), "no length"))?);
        let axis_world = wp.vector(d);
        let c0 = wp.point(Vec3::new(bend.start[0], bend.start[1], zc));
        let s0 = wp.point(lift(bend.start));
        let frame = Frame::new(c0, axis_world * (-bend.angle.signum()), s0 - c0)
            .ok_or_else(|| refuse(Some(i), "a bend's axis is not finite"))?;
        let surface = Surface::Cylinder(Cylinder::new(frame, zc.abs())?);
        let lp = b.body.add_loop(coedges(edges));
        // The bottom skin faces into the material: towards the axis for a
        // bend up (the bottom is the outside), away from it for a bend down.
        faces.push(b.body.add_face(surface, bend.angle > 0.0, vec![lp]));
        roles.push(SheetFace::Bend(i));
    }
    b.body.add_shell(faces);
    let mut defects = craft3d_topo::check_remembered(&b.body, tolerance);
    if !defects.is_empty() {
        defects.truncate(crate::MAX_REPORTED);
        return Err(KernelError::InvalidBody {
            operation: "sheet metal",
            defects,
        });
    }
    Ok((b.body, roles))
}

/// The bend's neutral band, `R + K T` about its axis over its angle and
/// as long as the bend, as a sheet of one face: what [`develop`] lays
/// flat to measure the strip's width.
pub fn neutral_band(bend: &Bend) -> Result<Body, KernelError> {
    let r = bend.neutral();
    let theta = bend.theta();
    let w = distance(bend.start, bend.end);
    let mut body = Body::new(BodyKind::Sheet);
    let frame = Frame::WORLD;
    let at = |u: f64, v: f64| Vec3::new(r * u.cos(), r * u.sin(), v);
    let v00 = body.add_vertex(at(0.0, 0.0));
    let v10 = body.add_vertex(at(theta, 0.0));
    let v11 = body.add_vertex(at(theta, w));
    let v01 = body.add_vertex(at(0.0, w));
    let bottom = body.add_edge(
        Curve3::Circle(Circle::new(frame, r)?),
        (0.0, theta),
        v00,
        v10,
    );
    let top_frame = frame
        .at(Vec3::new(0.0, 0.0, w))
        .ok_or_else(|| refuse(None, "a band's frame is not finite"))?;
    let top = body.add_edge(
        Curve3::Circle(Circle::new(top_frame, r)?),
        (0.0, theta),
        v01,
        v11,
    );
    let side = |body: &mut Body,
                a: VertexId,
                b: VertexId,
                pa: Vec3,
                pb: Vec3|
     -> Result<EdgeId, KernelError> {
        Ok(body.add_edge(
            Curve3::Line(Line::through(pa, pb)?),
            (0.0, pa.distance(pb)),
            a,
            b,
        ))
    };
    let far = side(&mut body, v10, v11, at(theta, 0.0), at(theta, w))?;
    let near = side(&mut body, v00, v01, at(0.0, 0.0), at(0.0, w))?;
    let lp = body.add_loop(vec![
        Coedge {
            edge: bottom,
            reversed: false,
        },
        Coedge {
            edge: far,
            reversed: false,
        },
        Coedge {
            edge: top,
            reversed: true,
        },
        Coedge {
            edge: near,
            reversed: true,
        },
    ]);
    let face = body.add_face(Surface::Cylinder(Cylinder::new(frame, r)?), false, vec![lp]);
    body.add_shell(vec![face]);
    Ok(body)
}

/// A bend's allowance as the development of its neutral band measures
/// it: the flat length of the band's arcs (both of them; the larger).
pub fn developed_allowance(bend: &Bend, tolerance: &Tolerance) -> Result<f64, KernelError> {
    let band = neutral_band(bend)?;
    let dev = develop(&band, &[FaceId(0)], tolerance)?;
    let mut widths = Vec::new();
    for curve in dev.outline() {
        let Some(edge) = curve.edge else { continue };
        let is_arc = band
            .edges
            .get(edge.0 as usize)
            .is_some_and(|e| matches!(e.curve, Curve3::Circle(_)));
        if is_arc {
            widths.push(curve.length_flat);
        }
    }
    widths
        .into_iter()
        .reduce(f64::max)
        .ok_or_else(|| refuse(None, "the band's development has no arcs"))
}

/// The inner radius of a bend as the solid has it: the smaller radius of
/// the two skins the plate made of the bend's face.
pub fn measured_radius(part: &Part, bend: usize) -> Option<f64> {
    let face = part
        .sheet_faces
        .iter()
        .position(|f| *f == SheetFace::Bend(bend))
        .and_then(|f| u32::try_from(f).ok())
        .map(FaceId)?;
    let mut radii = Vec::new();
    for (i, made) in part.solid.faces.iter().enumerate() {
        let of = match made {
            crate::plate::Made::Front(f) | crate::plate::Made::Back(f) => *f,
            crate::plate::Made::Rim(_) => continue,
        };
        if of != face {
            continue;
        }
        if let Some(Surface::Cylinder(c)) = part.solid.body.faces.get(i).map(|f| &f.surface) {
            radii.push(c.radius());
        }
    }
    radii.into_iter().reduce(f64::min)
}

/// The angle between the planes of a bend's two flats, as the fold put
/// them: θ back.
pub fn measured_angle(part: &Part, bend: usize) -> Option<f64> {
    let b = part.bends.get(bend)?;
    let np = part.flats.get(b.parent)?.transform.vector(Vec3::Z);
    let nc = part.flats.get(b.child)?.transform.vector(Vec3::Z);
    Some(np.dot(nc).clamp(-1.0, 1.0).acos())
}

/// How a hole of the base lies to a bend of the base: the least distance
/// in the pattern from the hole's boundary to the bend's tangent line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HoleToBend {
    /// The hole (its index among the base's holes) and the bend.
    pub hole: usize,
    pub bend: usize,
    /// The least distance: in closed form for lines, arcs and circles; for
    /// a spline the least distance from the chords of its pieces once they
    /// are flat within an eighth of the tolerance, so within an eighth of
    /// the tolerance of the true one either way (not a distance at a place
    /// on the curve) ...
    pub distance: f64,
    /// ... what the true one cannot be below: equal to `distance` in
    /// closed form, an eighth of the tolerance less for a spline, 0 when
    /// the search ran out of work ...
    pub lower: f64,
    /// ... and what it cannot be above: equal to `distance` in closed
    /// form, an eighth of the tolerance more for a spline, infinite when
    /// the search ran out of work (a piece left unrefined proves nothing).
    pub upper: f64,
    /// True when `distance` is in closed form (no spline in the hole).
    pub exact: bool,
}

/// For every hole of the base, the distance to the nearest bend on the
/// base (holes beside no bend on the base are not listed).
pub fn holes_to_bends(part: &Part, tolerance: &Tolerance) -> Vec<HoleToBend> {
    let Some(base) = part.flats.first() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (h, hole) in base.holes.iter().enumerate() {
        let mut best: Option<HoleToBend> = None;
        for (i, bend) in part.bends.iter().enumerate() {
            if bend.parent != 0 {
                continue;
            }
            let mut found = HoleToBend {
                hole: h,
                bend: i,
                distance: f64::INFINITY,
                lower: f64::INFINITY,
                upper: f64::INFINITY,
                exact: true,
            };
            for seg in hole {
                let (d, lower, upper, exact) =
                    segment_to_line(seg, bend.start, bend.end, tolerance.linear);
                found.distance = found.distance.min(d);
                found.lower = found.lower.min(lower);
                found.upper = found.upper.min(upper);
                found.exact &= exact;
            }
            if best.is_none_or(|b| found.lower < b.lower) {
                best = Some(found);
            }
        }
        out.extend(best);
    }
    out
}

fn point_to_segment(p: P2, a: P2, b: P2) -> (f64, P2) {
    let d = sub(b, a);
    let l2 = dot(d, d);
    let t = if l2 > 0.0 {
        (dot(sub(p, a), d) / l2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let q = add(a, scale(d, t));
    (distance(p, q), q)
}

/// The least distance from a segment of a loop to the straight segment
/// `a`–`b` they do not cross: the distance found, what the true one cannot
/// be below and above, and whether it is in closed form (lines, arcs,
/// circles) or searched (splines, to an eighth of `tol`; without bounds,
/// 0 and infinity, when the search ran out of work).
fn segment_to_line(seg: &Segment, a: P2, b: P2, tol: f64) -> (f64, f64, f64, bool) {
    let closed = |d: f64| (d, d, d, true);
    match &seg.shape {
        Shape::Line => closed(
            point_to_segment(seg.start, a, b)
                .0
                .min(point_to_segment(seg.end, a, b).0)
                .min(seg.distance_to(a, tol))
                .min(seg.distance_to(b, tol)),
        ),
        Shape::Circle { center, radius } => {
            closed((point_to_segment(*center, a, b).0 - radius).max(0.0))
        }
        Shape::Arc { center, radius } => {
            let mut d = point_to_segment(seg.start, a, b)
                .0
                .min(point_to_segment(seg.end, a, b).0)
                .min(seg.distance_to(a, tol))
                .min(seg.distance_to(b, tol));
            // The arc's point nearest the line, where the normal from the
            // segment's nearest point to the centre meets it.
            let (dc, q) = point_to_segment(*center, a, b);
            if let Some(dir) = unit(sub(q, *center)) {
                let on = add(*center, scale(dir, *radius));
                if seg.distance_to(on, tol) <= tol {
                    d = d.min((dc - radius).abs());
                }
            }
            closed(d)
        }
        Shape::Spline(_) => match seg.spline_to_line(a, b, tol) {
            Some((d, true)) => (d, (d - tol / 8.0).max(0.0), d + tol / 8.0, false),
            Some((d, false)) => (d, 0.0, f64::INFINITY, false),
            None => (f64::INFINITY, 0.0, f64::INFINITY, false),
        },
    }
}

/// The flange length of a flange's flat: from its bend's tangent line on
/// the flange's side to the farthest point of its outline.
pub fn flange_length(part: &Part, bend: usize) -> Option<f64> {
    let b = part.bends.get(bend)?;
    let flat = part.flats.get(b.child)?;
    let (far, _) = b.far_line();
    flat.outer
        .iter()
        .flat_map(|s| [s.start, s.end])
        .map(|p| dot(sub(p, far), b.across))
        .reduce(f64::max)
}

/// The area of the flat pattern: the flats' (holes taken out) and the
/// bends' strips.
pub fn flat_area(part: &Part) -> f64 {
    pattern_area(&part.flats, &part.bends)
}

fn pattern_area(flats: &[Flat], bends: &[Bend]) -> f64 {
    let ring = |segs: &[Segment]| {
        crate::profile::Loop2 {
            segments: segs.to_vec(),
        }
        .signed_area()
    };
    let flats: f64 = flats
        .iter()
        .map(|f| ring(&f.outer) + f.holes.iter().map(|h| ring(h)).sum::<f64>())
        .sum();
    let strips: f64 = bends
        .iter()
        .map(|b| b.allowance * distance(b.start, b.end))
        .sum();
    flats + strips
}

/// The outline of the flat pattern, in the pattern's axes: closed chains
/// of segments, the outer one counter-clockwise and the holes clockwise
/// (each segment's `start` is where the one before it ends). The flats'
/// edges that a bend's strip continues are left out; the strips' ends
/// are lines across them.
pub fn outline(part: &Part, tolerance: &Tolerance) -> Vec<Vec<Segment>> {
    tagged_outline(&part.flats, &part.bends, tolerance.linear)
        .into_iter()
        .map(|chain| chain.into_iter().map(|(s, _)| s).collect())
        .collect()
}

/// [`outline`] with, per segment, the flat it belongs to (0 the base,
/// `i + 1` flange `i`, whose bend's strip is counted with it).
fn tagged_outline(flats: &[Flat], bends: &[Bend], tol: f64) -> Vec<Vec<(Segment, usize)>> {
    // The edges a strip continues: per flat, the indices of its outer
    // segments.
    let mut inner: BTreeMap<(usize, usize), ()> = BTreeMap::new();
    for b in bends {
        // The child's tangent edge is its segment 0.
        inner.insert((b.child, 0), ());
        if let Some(flat) = flats.get(b.parent)
            && let Some(k) = flat.outer.iter().position(|s| {
                matches!(s.shape, Shape::Line)
                    && distance(s.start, b.start) <= tol
                    && distance(s.end, b.end) <= tol
            })
        {
            inner.insert((b.parent, k), ());
        }
    }
    let mut pieces: Vec<(Segment, usize)> = Vec::new();
    let mut loops: Vec<Vec<(Segment, usize)>> = Vec::new();
    for (f, flat) in flats.iter().enumerate() {
        for (k, s) in flat.outer.iter().enumerate() {
            if !inner.contains_key(&(f, k)) {
                pieces.push((s.clone(), f));
            }
        }
        for hole in &flat.holes {
            loops.push(hole.iter().map(|s| (s.clone(), f)).collect());
        }
    }
    for b in bends {
        let (a0, a1) = b.far_line();
        pieces.push((line(b.start, a0), b.child));
        pieces.push((line(a1, b.end), b.child));
    }
    // Chained head to tail.
    let mut outer: Vec<Vec<(Segment, usize)>> = Vec::new();
    let mut used = vec![false; pieces.len()];
    for first in 0..pieces.len() {
        if used.get(first).copied().unwrap_or(true) {
            continue;
        }
        let Some(seg) = pieces.get(first).cloned() else {
            continue;
        };
        if let Some(u) = used.get_mut(first) {
            *u = true;
        }
        let start = seg.0.start;
        let mut chain = vec![seg];
        let mut guard = 0usize;
        while guard <= pieces.len() {
            guard += 1;
            let Some(end) = chain.last().map(|(s, _)| s.end) else {
                break;
            };
            if chain.len() > 1 && distance(end, start) <= tol {
                break;
            }
            let next = (0..pieces.len()).find(|&j| {
                !used.get(j).copied().unwrap_or(true)
                    && pieces
                        .get(j)
                        .is_some_and(|(s, _)| distance(s.start, end) <= tol)
            });
            let Some(j) = next else { break };
            if let Some(u) = used.get_mut(j) {
                *u = true;
            }
            if let Some(s) = pieces.get(j) {
                chain.push(s.clone());
            }
        }
        outer.push(chain);
    }
    outer.extend(loops);
    outer
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::{Curve2, SketchCurve, Spline2, detect_profiles};
    use crate::spline2::SEGMENT_WORK_LIMIT;

    /// A 60 × 40 base with a hole bounded by a cubic spline and a line,
    /// and one flange on its bottom edge, at the scale `k`.
    fn spline_hole_part(k: f64, tol: &Tolerance) -> Part {
        let s = |p: [f64; 2]| [p[0] * k, p[1] * k];
        let c = [[0.0, 0.0], [60.0, 0.0], [60.0, 40.0], [0.0, 40.0]];
        let line = |id: u64, a: [f64; 2], b: [f64; 2]| SketchCurve {
            id,
            curve: Curve2::Line {
                start: s(a),
                end: s(b),
            },
        };
        let mut curves: Vec<SketchCurve> = (0..4)
            .map(|i| line(i as u64 + 1, c[i], c[(i + 1) % 4]))
            .collect();
        curves.push(line(9, [40.0, 15.0], [20.0, 15.0]));
        curves.push(SketchCurve {
            id: 10,
            curve: Curve2::Spline(Spline2 {
                degree: 3,
                knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
                control: [[20.0, 15.0], [24.0, 2.0], [33.0, 4.0], [40.0, 15.0]]
                    .iter()
                    .map(|p| s(*p))
                    .collect(),
            }),
        });
        let base = detect_profiles(&curves, tol).unwrap().remove(0);
        build(
            &Spec {
                base,
                plane: Frame::WORLD,
                thickness: k,
                flanges: vec![FlangeSpec {
                    on: On::Base(1),
                    length: 10.0 * k,
                    angle: 0.5 * PI,
                    direction: Direction::Up,
                    radius: k,
                    k: 0.4,
                    from: LengthFrom::Tangent,
                }],
                relief: None,
            },
            tol,
        )
        .unwrap()
    }

    /// A spline hole's distance to the bend is bounded both ways while the
    /// search is proven (an eighth of the tolerance), and not bounded
    /// above at all when the search runs out of work: the spline's upper
    /// bound is infinite then, so the hole's is only what its line proves
    /// (15 from the bend line, in closed form), never the unproven search's
    /// estimate plus an eighth.
    #[test]
    fn a_spline_search_that_runs_out_of_work_bounds_nothing_above() {
        for k in [1e-3, 1.0, 1e6] {
            let tol = Tolerance::new(1e-6 * k, 1e-9).unwrap();
            let part = spline_hole_part(k, &tol);
            let found = holes_to_bends(&part, &tol);
            assert_eq!(found.len(), 1);
            let h = found[0];
            assert!(!h.exact);
            let eighth = tol.linear / 8.0;
            assert!(
                (h.upper - (h.distance + eighth)).abs() <= 1e-12 * k,
                "{h:?}"
            );
            assert!((h.distance - eighth - h.lower).abs() <= 1e-12 * k, "{h:?}");

            let before = SEGMENT_WORK_LIMIT.with(|c| c.replace(2));
            let starved = holes_to_bends(&part, &tol);
            let bend = part.bends[0];
            let spline = part.flats[0].holes[0]
                .iter()
                .find(|s| matches!(s.shape, Shape::Spline(_)))
                .unwrap()
                .clone();
            let (d, lower, upper, exact) =
                segment_to_line(&spline, bend.start, bend.end, tol.linear);
            SEGMENT_WORK_LIMIT.with(|c| c.set(before));
            assert!(!exact && d.is_finite());
            assert_eq!((lower, upper), (0.0, f64::INFINITY));
            assert_eq!(starved.len(), 1);
            let h = starved[0];
            assert!(!h.exact);
            assert_eq!(h.lower, 0.0, "{h:?}");
            assert!(h.distance + eighth < 15.0 * k, "{h:?}");
            assert!((h.upper - 15.0 * k).abs() <= 1e-12 * k, "{h:?}");
        }
    }
}
