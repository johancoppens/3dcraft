//! The section's handle in the view (the developer's design of
//! 2026-11-17 and his second look the same day, `docs/guide/window.md` ›
//! *Looking around*): while the section is on, the plane's outline (thin,
//! dashed, faint until the pointer comes near), one arrow at a corner of
//! it along its normal, pointing into the half that is cut away (light
//! with a dark edge, so it reads on any background; the accent under the
//! pointer), and beside it one group on a dark plate: the offset as a
//! number with the axis and the document's unit, the buttons X, Y, Z (and
//! ∥) and Flip side. Gone when the section is off; the arrow and the
//! group keep clear of the plates, the cube and the triad.
//!
//! - A drag on the arrow moves the plane: the pointer's ray is taken to
//!   the nearest point of the line through the arrow ([`nearest_on_line`]),
//!   and the offset follows by as much. It snaps lightly
//!   ([`snap_offset`]): to the places the model gives (a flat face
//!   parallel to the plane, the axis of a hole across it:
//!   `craft3d_engine::SectionPlane::snaps`, found once per scene and
//!   plane), else to a round value of the model's size, else to about a
//!   point's worth; a small tick says it snapped. **Alt** held: none.
//! - A double click on the arrow, or the Flip button, keeps the other half.
//! - A click on the number types a value, as on the Offset slider: Enter
//!   sets it, Escape leaves it.
//! - X, Y or Z turns the plane across that axis through the middle of the
//!   model; ∥ lays it on the selected flat face.
//!
//! The handle is a view of `state.section` and changes nothing else: the
//! slider, the display bar and `ui.set` drive the same state, and an
//! agent reads it with `ui.get`. It is hit before the model's faces, and
//! the plates over the viewport before it. Seen end on (the normal
//! pointing at the viewer) the arrow is a ring, and the number and the
//! buttons still work. On a window at least as wide as the narrow layout
//! the display bar shows Section alone while the section is on; the
//! narrow layout (a phone) leaves the handle out and keeps X, Y, Z, the
//! Offset slider and Flip side on the bar: there a finger's drag in the
//! middle of the model is a turn of the view. Once a finger has touched
//! the screen, the arrow and the buttons are a finger's size
//! (`Tokens::touch`).

use super::Shell;
use super::controls::{SECTION_PLANES, choose_axis, chosen_axis};
use crate::camera::Projector;
use crate::kit::Kit;
use crate::kit::slider::{Typing, nice_step, shown, snapped};
use crate::state::CameraState;
use crate::theme::{Tokens, color, color_alpha};
use craft3d_engine::{Scene, SectionPlane, SectionSnap, SnapKind};
use craft3d_math::{Bbox3, Portable, Vec3};
use egui::{Pos2, Rect, Vec2, pos2, vec2};

/// The name of the handle's number in [`Shell::typing`].
pub const NUMBER_FIELD: &str = "section-number";
/// Most places `ui.get` lists under `section.snaps`: those nearest the
/// plane where it is ([`nearest_snaps`]).
pub const REPORTED_SNAPS: usize = 64;

/// The [`REPORTED_SNAPS`] places of `snaps` (sorted by offset) nearest
/// `offset`, in their order.
pub fn nearest_snaps(snaps: &[SectionSnap], offset: f64) -> &[SectionSnap] {
    if snaps.len() <= REPORTED_SNAPS {
        return snaps;
    }
    let at = snaps.partition_point(|snap| snap.offset < offset);
    let first = at
        .saturating_sub(REPORTED_SNAPS / 2)
        .min(snaps.len() - REPORTED_SNAPS);
    snaps.get(first..first + REPORTED_SNAPS).unwrap_or(snaps)
}
/// How far the outline reaches past the model's box, as a share of the
/// box's larger side in the plane.
const OUTLINE_MARGIN: f64 = 0.04;
/// Shorter than this share of its length, the arrow is seen end on.
const END_ON: f64 = 0.3;
/// How far the number keeps from the arrow's tip, the dots from the
/// number and from each other, and the handle from the viewport's edge,
/// in points.
const GAP: f32 = 8.0;
const EDGE: f32 = 6.0;
/// How the number is padded on its plate, in points.
const PAD: Vec2 = vec2(7.0, 3.0);

/// A part of the handle a pointer can be on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HandlePart {
    Arrow,
    Number,
    /// The button of X, Y or Z (0, 1, 2), or [`FACE_DOT`]: the plane
    /// parallel to the selected flat face.
    Dot(usize),
    /// The button that keeps the other half (Flip side).
    Flip,
}

/// The fourth dot's place among the dots, and the bar's fourth cell's
/// among X, Y, Z: the plane parallel to a face.
pub const FACE_DOT: usize = 3;

/// The face the fourth dot would lay the plane on: the one face selected,
/// by its body's feature (`F3`) and its persistent name, and why it
/// cannot when it is not flat.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FaceChoice {
    pub feature: String,
    pub face: String,
    pub refused: Option<String>,
}

/// The section across the face of `choice`, through it: the face's plane
/// with its outward normal, offset 0, nothing kept flipped (what lies
/// outside the face is cut away, so nothing is yet: the arrow, dragged
/// in, cuts into the body). A face that is not flat leaves the section
/// as it is.
pub fn choose_face(section: &mut crate::state::SectionState, choice: &FaceChoice) {
    if choice.refused.is_some() {
        return;
    }
    if section.plane.as_deref() == Some(choice.face.as_str())
        && section.feature.as_deref() == Some(choice.feature.as_str())
    {
        return;
    }
    section.plane = Some(choice.face.clone());
    section.feature = Some(choice.feature.clone());
    section.point = None;
    section.normal = None;
    section.offset = 0.0;
    section.flip = false;
}

/// What a drag snapped to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Snapped {
    /// A place the model gives.
    Place(SnapKind),
    /// A round value.
    Round,
}

/// Where the plane and its arrow are on the screen ([`plane_frame`]).
#[derive(Clone, Debug, PartialEq)]
pub struct PlaneFrame {
    /// The sides of the plane's outline, as far as they are in front of
    /// the eye, in the window's points.
    pub outline: Vec<[Pos2; 2]>,
    /// The arrow's foot: a corner of the plane's outline, the same one
    /// however the view turns (the one furthest along x, then against y
    /// and z), moved along the plane into the viewport where it is out of
    /// it; in space and on the screen. The line the arrow is dragged
    /// along goes through it.
    pub middle: Vec3,
    pub at: Pos2,
    /// The middle of the cut on the screen: what the group keeps away
    /// from.
    pub centre: Pos2,
    /// The unit normal into the half that is cut away, and how long the
    /// arrow is along it, in model units.
    pub normal: Vec3,
    pub length: f64,
    /// The arrow on the screen, from the middle to its tip; `None` when
    /// it is seen end on.
    pub arrow: Option<[Pos2; 2]>,
    /// The normal points toward the viewer.
    pub toward: bool,
    /// Model units per point along the arrow on the screen.
    pub per_point: f64,
}

/// Where the handle was drawn, in the window's points.
#[derive(Clone, Debug, PartialEq)]
pub struct HandlePlace {
    pub frame: PlaneFrame,
    /// The group's plate, and in it the number.
    pub group: Rect,
    pub number: Rect,
    /// The middles of the buttons X, Y and Z (and the face's, while there
    /// is one: [`FACE_DOT`]), and of Flip side; half their side.
    pub dots: Vec<Pos2>,
    pub flip: Pos2,
    pub dot_radius: f32,
    /// How near the arrow a press takes it.
    pub hit: f32,
}

impl HandlePlace {
    /// The part of the handle at `p`: the number, a button, then the
    /// arrow.
    pub fn part_at(&self, p: Pos2) -> Option<HandlePart> {
        if self.number.expand(1.0).contains(p) {
            return Some(HandlePart::Number);
        }
        let cell =
            |c: Pos2| Rect::from_center_size(c, egui::Vec2::splat(self.dot_radius * 2.0 + 2.0));
        if let Some(k) = self.dots.iter().position(|&d| cell(d).contains(p)) {
            return Some(HandlePart::Dot(k));
        }
        if cell(self.flip).contains(p) {
            return Some(HandlePart::Flip);
        }
        if self.group.contains(p) {
            return None;
        }
        let [a, b] = self.frame.arrow?;
        (distance_to_segment(p, a, b) <= self.hit).then_some(HandlePart::Arrow)
    }

    /// Whether `p` is within `reach` of the handle: of the arrow (or the
    /// ring) or the group.
    pub fn near(&self, p: Pos2, reach: f32) -> bool {
        let [a, b] = self.frame.arrow.unwrap_or([self.frame.at; 2]);
        distance_to_segment(p, a, b) <= reach || self.group.expand(reach).contains(p)
    }
}

/// How far `p` is from the segment from `a` to `b`.
pub fn distance_to_segment(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let length = ab.length_sq();
    let t = if length > 0.0 {
        ((p - a).dot(ab) / length).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let d = p.distance(a + ab * t);
    if d.is_nan() { f32::INFINITY } else { d }
}

/// Two unit vectors square to `normal` and to each other: the sides of
/// the outline. The world axis least along the normal fixes them, so an
/// outline across X, Y or Z has sides along the other two.
fn in_plane(normal: Vec3) -> Option<(Vec3, Vec3)> {
    let across = [Vec3::X, Vec3::Y, Vec3::Z]
        .into_iter()
        .min_by(|a, b| normal.dot(*a).abs().total_cmp(&normal.dot(*b).abs()))?;
    let u = normal.cross(across).normalized()?;
    Some((u, normal.cross(u)))
}

/// Where the plane through `origin` with unit normal `normal` passes
/// through the box from `lo` to `hi`: the corners of that polygon in the
/// plane's coordinates along `u` and `v` from `origin`, in order round
/// it; `None` when it passes by (or touches it in less than a polygon).
/// Across X, Y or Z it is the box's cross-section; at a slant it is
/// where the plane cuts the corner it passes through.
fn box_cut(
    lo: Vec3,
    hi: Vec3,
    origin: Vec3,
    normal: Vec3,
    u: Vec3,
    v: Vec3,
) -> Option<Vec<(f64, f64)>> {
    let size = lo.distance(hi);
    let eps = (size * 1e-12).max(f64::MIN_POSITIVE);
    let corner = |k: usize| {
        Vec3::new(
            if k & 1 == 0 { lo.x } else { hi.x },
            if k & 2 == 0 { lo.y } else { hi.y },
            if k & 4 == 0 { lo.z } else { hi.z },
        )
    };
    let mut points: Vec<(f64, f64)> = Vec::new();
    let mut add = |p: Vec3| {
        let d = p - origin;
        let q = (d.dot(u), d.dot(v));
        if q.0.is_finite()
            && q.1.is_finite()
            && !points
                .iter()
                .any(|r| (r.0 - q.0).abs() <= eps && (r.1 - q.1).abs() <= eps)
        {
            points.push(q);
        }
    };
    // The twelve edges: corners that differ in one bit.
    for a in 0..8_usize {
        for bit in [1, 2, 4] {
            if a & bit != 0 {
                continue;
            }
            let (p, q) = (corner(a), corner(a | bit));
            let (sp, sq) = ((p - origin).dot(normal), (q - origin).dot(normal));
            if sp.abs() <= eps {
                add(p);
            }
            if sq.abs() <= eps {
                add(q);
            }
            if (sp < -eps && sq > eps) || (sp > eps && sq < -eps) {
                add(p + (q - p) * (sp / (sp - sq)));
            }
        }
    }
    if points.len() < 3 {
        return None;
    }
    let n = points.len() as f64;
    let (cs, ct) = points
        .iter()
        .fold((0.0, 0.0), |(a, b), (s, t)| (a + s / n, b + t / n));
    points.sort_by(|a, b| {
        (a.1 - ct)
            .patan2(a.0 - cs)
            .total_cmp(&(b.1 - ct).patan2(b.0 - cs))
    });
    Some(points)
}

/// How far inside the viewport's edge the arrow's foot is kept, in
/// points: room for the arrow and its group.
const INSIDE: f32 = 56.0;

/// `p`, a point of the plane through `origin` with unit normal `normal`,
/// when it shows inside the viewport less [`INSIDE`], [`INSIDE`] clear of
/// each of `keep_out` (the controls over the view); else the point
/// between it and `centre` (the cut's middle, on the plane too) furthest
/// toward it that shows there; where the middle does not show either, the
/// point of the plane that shows at the nearest place inside it clear of
/// `keep_out` (on the way to the viewport's middle). `None`
/// where none of these is (the plane edge on and out of view).
fn into_view(
    projector: &Projector,
    viewport: Rect,
    keep_out: &[Rect],
    p: Vec3,
    centre: Vec3,
    origin: Vec3,
    normal: Vec3,
) -> Option<Vec3> {
    let room = viewport.shrink(
        INSIDE
            .min(viewport.width() * 0.25)
            .min(viewport.height() * 0.25),
    );
    let shows = |q: Vec3| {
        projector.project(q).is_some_and(|s| {
            let at = pos2(s.x as f32 + viewport.left(), s.y as f32 + viewport.top());
            room.contains(at) && keep_out.iter().all(|r| !r.expand(INSIDE).contains(at))
        })
    };
    if shows(p) {
        return Some(p);
    }
    // Toward the cut's middle, as far out as still shows: on the plane,
    // also where it is seen edge on.
    if shows(centre) {
        let (mut inside, mut outside) = (0.0_f64, 1.0_f64);
        for _ in 0..30 {
            let half = (inside + outside) * 0.5;
            if shows(centre.lerp(p, half)) {
                inside = half;
            } else {
                outside = half;
            }
        }
        return Some(centre.lerp(p, inside));
    }
    let seen = projector.project(p);
    let (x, y) = match seen {
        Some(s) => (s.x as f32 + viewport.left(), s.y as f32 + viewport.top()),
        None => (viewport.center().x, viewport.center().y),
    };
    let inside = pos2(
        x.clamp(room.left(), room.right()),
        y.clamp(room.top(), room.bottom()),
    );
    // Clear of the controls: toward the viewport's middle as far as it
    // takes.
    let inside = (0..=16)
        .map(|k| inside.lerp(room.center(), k as f32 / 16.0))
        .find(|at| keep_out.iter().all(|r| !r.expand(INSIDE).contains(*at)))
        .unwrap_or(inside);
    let (start, way) = projector.ray(
        f64::from(inside.x - viewport.left()),
        f64::from(inside.y - viewport.top()),
    );
    let across = way.dot(normal);
    if across.abs() <= 1e-9 * way.length() {
        return None;
    }
    let t = (origin - start).dot(normal) / across;
    let q = start + way * t;
    (t > 0.0 && q.is_finite()).then_some(q)
}

/// Where `plane` and its arrow are in the `viewport` (in the window's
/// points) seen by `camera`: the outline of where it passes through the
/// model's box `bounds` (the box laid onto it where it passes by), a
/// little larger, and the arrow at a corner of it, clear of `keep_out`
/// (the controls over the view) where it can be,
/// [`Tokens::section_arrow_length`] long seen side on. `None` for an
/// empty box, a camera that looks nowhere, a middle behind the eye, or a
/// number that is not one.
pub fn plane_frame(
    camera: &CameraState,
    viewport: Rect,
    keep_out: &[Rect],
    plane: &SectionPlane,
    bounds: &Bbox3,
    tokens: &Tokens,
) -> Option<PlaneFrame> {
    let (lo, hi) = bounds.corners()?;
    let projector = Projector::new(
        &camera.to_render(),
        f64::from(viewport.width()),
        f64::from(viewport.height()),
    )?;
    let (u, v) = in_plane(plane.along)?;
    let origin = plane.origin;
    let (mut s0, mut s1, mut t0, mut t1) = (
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
    );
    for x in [lo.x, hi.x] {
        for y in [lo.y, hi.y] {
            for z in [lo.z, hi.z] {
                let d = Vec3::new(x, y, z) - origin;
                let (s, t) = (d.dot(u), d.dot(v));
                (s0, s1, t0, t1) = (s0.min(s), s1.max(s), t0.min(t), t1.max(t));
            }
        }
    }
    let margin = (s1 - s0).max(t1 - t0) * OUTLINE_MARGIN;
    let corner = |s: f64, t: f64| origin + u * s + v * t;
    // Where the plane passes through the box, a little larger; the box
    // laid onto the plane where it passes by it.
    let (corners, middle) = match box_cut(lo, hi, origin, plane.along, u, v) {
        Some(polygon) => {
            let n = polygon.len() as f64;
            let (cs, ct) = polygon
                .iter()
                .fold((0.0, 0.0), |(a, b), (s, t)| (a + s / n, b + t / n));
            let grown: Vec<Vec3> = polygon
                .iter()
                .map(|&(s, t)| {
                    let (ds, dt) = (s - cs, t - ct);
                    let reach = ds.phypot(dt);
                    let k = if reach > 0.0 {
                        1.0 + margin / reach
                    } else {
                        1.0
                    };
                    corner(cs + ds * k, ct + dt * k)
                })
                .collect();
            (grown, corner(cs, ct))
        }
        None => {
            let (s0, s1, t0, t1) = (s0 - margin, s1 + margin, t0 - margin, t1 + margin);
            (
                vec![
                    corner(s0, t0),
                    corner(s1, t0),
                    corner(s1, t1),
                    corner(s0, t1),
                ],
                corner((s0 + s1) * 0.5, (t0 + t1) * 0.5),
            )
        }
    };
    if !(middle.is_finite() && corners.iter().all(|c| c.is_finite())) {
        return None;
    }
    let to_window = |x: f64, y: f64| {
        let p = pos2(viewport.left() + x as f32, viewport.top() + y as f32);
        (p.x.is_finite() && p.y.is_finite()).then_some(p)
    };
    let sides = corners.len();
    let outline: Vec<[Pos2; 2]> = (0..sides)
        .filter_map(|k| {
            let (a, b) = projector.segment(*corners.get(k)?, *corners.get((k + 1) % sides)?)?;
            Some([to_window(a.x, a.y)?, to_window(b.x, b.y)?])
        })
        .collect();
    let centre = projector.project(middle).and_then(|p| to_window(p.x, p.y));
    // The arrow's foot: the corner furthest along x - y - z, a rule of
    // the model's, so it stays where it is while the view turns.
    let corner = corners
        .iter()
        .copied()
        .max_by(|a, b| (a.x - a.y - a.z).total_cmp(&(b.x - b.y - b.z)))?;
    let foot = into_view(
        &projector,
        viewport,
        keep_out,
        corner,
        middle,
        origin,
        plane.along,
    )
    .unwrap_or(corner);
    let seen = projector.project(foot)?;
    let at = to_window(seen.x, seen.y)?;
    let length = f64::from(tokens.section_arrow_length) * projector.footprint(seen.depth);
    let normal = plane.normal;
    if !(length.is_finite() && length > 0.0) {
        return None;
    }
    let tip = projector
        .project(foot + normal * length)
        .and_then(|p| to_window(p.x, p.y));
    let shown = tip.map_or(0.0, |tip| f64::from(tip.distance(at)));
    let side_on = shown >= END_ON * f64::from(tokens.section_arrow_length);
    Some(PlaneFrame {
        outline,
        middle: foot,
        at,
        centre: centre.unwrap_or(at),
        normal,
        length,
        arrow: tip.filter(|_| side_on).map(|tip| [at, tip]),
        toward: normal.dot(camera.toward_eye()) > 0.0,
        per_point: if side_on { length / shown } else { 0.0 },
    })
}

/// Where the group (`size`: the number and the buttons on one plate)
/// goes beside the arrow's foot in the `viewport`: away from the cut's
/// middle, so it never covers it, on the first side that keeps clear of
/// the arrow (outward from the middle, then square to the arrow, then
/// back along it) and of `keep_out` (the controls over the view); within
/// the viewport.
pub fn group_place(frame: &PlaneFrame, viewport: Rect, keep_out: &[Rect], size: Vec2) -> Rect {
    let room = viewport.shrink(EDGE);
    let at = frame.at;
    let out = (at - frame.centre).normalized();
    let out = if out.x.is_finite() && out.length() > 0.5 {
        out
    } else {
        vec2(1.0, 0.5).normalized()
    };
    let arrow = frame.arrow.map(|[a, b]| (b - a).normalized());
    let mut ways = vec![out];
    if let Some(way) = arrow {
        let across = vec2(-way.y, way.x);
        let across = if across.dot(out) >= 0.0 {
            across
        } else {
            -across
        };
        ways.extend([across, -across, -way]);
    }
    let placed = |way: Vec2| {
        let reach = way.x.abs() * size.x * 0.5 + way.y.abs() * size.y * 0.5;
        let mut place = Rect::from_center_size(at + way * (GAP + reach), size);
        let shift = vec2(
            (room.left() - place.left()).max(0.0) - (place.right() - room.right()).max(0.0),
            (room.top() - place.top()).max(0.0) - (place.bottom() - room.bottom()).max(0.0),
        );
        if shift.x.is_finite() && shift.y.is_finite() {
            place = place.translate(shift);
        }
        place
    };
    let clear = |place: &Rect| {
        frame.arrow.is_none_or(|[a, b]| {
            let grown = place.expand(GAP * 0.5);
            // Clear of the arrow: none of points along it inside.
            (0..=8).all(|k| !grown.contains(a.lerp(b, k as f32 / 8.0)))
        }) && !place.expand(GAP * 0.5).contains(at)
            && keep_out.iter().all(|r| !r.intersects(*place))
    };
    ways.iter()
        .map(|&way| placed(way))
        .find(clear)
        .unwrap_or_else(|| placed(out))
}

/// How the outline is drawn: as one closed path when its sides join,
/// as one segment between its farthest points when it is seen edge on
/// (its sides lie over each other), else side by side.
pub fn outline_paths(outline: &[[Pos2; 2]]) -> Vec<Vec<Pos2>> {
    let joined = !outline.is_empty()
        && outline.iter().enumerate().all(|(k, [_, end])| {
            outline
                .get((k + 1) % outline.len())
                .is_some_and(|[next, _]| next.distance(*end) < 0.01)
        });
    if !joined {
        return outline.iter().map(|side| side.to_vec()).collect();
    }
    let points: Vec<Pos2> = outline.iter().map(|[a, _]| *a).collect();
    let n = points.len();
    let (mut area, mut perimeter) = (0.0_f32, 0.0_f32);
    for k in 0..n {
        let (Some(a), Some(b)) = (points.get(k), points.get((k + 1) % n)) else {
            continue;
        };
        area += a.x * b.y - b.x * a.y;
        perimeter += a.distance(*b);
    }
    // Thinner than a point and a half: edge on.
    if perimeter > 0.0 && area.abs() / perimeter < 1.5 {
        let mut far = (0.0_f32, None);
        for a in &points {
            for b in &points {
                let d = a.distance(*b);
                if d > far.0 {
                    far = (d, Some([*a, *b]));
                }
            }
        }
        return far.1.map(|[a, b]| vec![vec![a, b]]).unwrap_or_default();
    }
    let mut closed = points;
    if let Some(&first) = closed.first() {
        closed.push(first);
    }
    vec![closed]
}

/// The parameter `t` of the point of the line `origin + direction · t`
/// nearest to the ray `start + way · s` (`s` positive: in front of where
/// the ray starts, the eye or the eye plane): where a pointer's ray meets
/// the line of the arrow. `None` where the ray runs (nearly) along the
/// line, where the nearest place on the ray is behind its start (in a
/// perspective view, a pointer past the point the line runs to on the
/// screen: the line's nearest point is then behind the eye, and the
/// plane would jump to the other end), and for numbers that are not.
pub fn nearest_on_line(origin: Vec3, direction: Vec3, start: Vec3, way: Vec3) -> Option<f64> {
    let w = origin - start;
    let (a, b, c) = (direction.dot(direction), direction.dot(way), way.dot(way));
    let (d, e) = (direction.dot(w), way.dot(w));
    let denominator = a * c - b * b;
    // Less than about a tenth of a degree apart: no answer to trust.
    if !(denominator > 1e-6 * a * c && denominator.is_finite()) {
        return None;
    }
    let s = (a * e - b * d) / denominator;
    if s.is_nan() || s <= 0.0 {
        return None;
    }
    let t = (b * e - c * d) / denominator;
    t.is_finite().then_some(t)
}

/// The smallest of 1, 2 and 5 times a power of ten at least `rough`.
fn round_step(rough: f64) -> Option<f64> {
    if !(rough > 0.0 && rough.is_finite()) {
        return None;
    }
    let power = 10f64.ppowf(rough.plog10().floor());
    [1.0, 2.0, 5.0, 10.0]
        .into_iter()
        .map(|m| m * power)
        .find(|&s| s >= rough * (1.0 - 1e-9))
        .filter(|s| s.is_finite() && *s > 0.0)
}

/// The offset a drag to `raw` gives, held to `range`: the nearest of the
/// `places` (sorted by offset) within `reach` model units, else a round
/// value of the range's size ([`nice_step`]) within `reach`, else `raw` on
/// a grid of about a point's worth (`per_point` model units), so the
/// number stays clean; with `free` (Alt) `raw` as it is. Also what it
/// snapped to.
pub fn snap_offset(
    raw: f64,
    places: &[SectionSnap],
    reach: f64,
    per_point: f64,
    range: [f64; 2],
    free: bool,
) -> (f64, Option<Snapped>) {
    let [lo, hi] = range;
    let ranged = lo.is_finite() && hi.is_finite() && lo < hi;
    let raw = if ranged { raw.clamp(lo, hi) } else { raw };
    if free || !raw.is_finite() {
        return (raw, None);
    }
    let reach = if reach.is_finite() {
        reach.max(0.0)
    } else {
        0.0
    };
    let k = places.partition_point(|s| s.offset < raw);
    let nearest = [k.checked_sub(1), Some(k)]
        .into_iter()
        .flatten()
        .filter_map(|k| places.get(k))
        .filter(|s| (s.offset - raw).abs() <= reach && (!ranged || (lo..=hi).contains(&s.offset)))
        .min_by(|a, b| (a.offset - raw).abs().total_cmp(&(b.offset - raw).abs()));
    if let Some(place) = nearest {
        return (place.offset, Some(Snapped::Place(place.kind)));
    }
    if ranged {
        let round = snapped(raw, nice_step(range), range);
        if (round - raw).abs() <= reach {
            return (round, Some(Snapped::Round));
        }
    }
    let whole = [f64::NEG_INFINITY, f64::INFINITY];
    match round_step(per_point) {
        Some(grid) => (snapped(raw, grid, if ranged { range } else { whole }), None),
        None => (raw, None),
    }
}

/// A drag of the arrow under way.
#[derive(Clone, Debug, PartialEq)]
struct Drag {
    /// The line of the arrow when the drag began: its middle and the unit
    /// normal.
    middle: Vec3,
    normal: Vec3,
    /// Where the pointer's ray met it then, and the offset then.
    from: f64,
    offset: f64,
    /// +1 when the normal is the plane's own (`along`), −1 flipped.
    sign: f64,
    /// The section as the drag left it last (as it was when it began,
    /// until it moves it), and the document it began in: a section that
    /// is no longer that (an agent's `ui.set`, another tab shown) ends
    /// the drag.
    expected: crate::state::SectionState,
    document: Option<u64>,
    /// Ended so: the pointer's drag is held until it lets go, and moves
    /// nothing (neither the plane nor the view).
    spent: bool,
}

/// What the shell keeps of the handle between frames.
#[derive(Clone, Debug, Default)]
pub(crate) struct HandleView {
    /// Where it was drawn last: what a press is tested against.
    place: Option<HandlePlace>,
    drag: Option<Drag>,
    /// What the drag under way snapped to last.
    snapped: Option<Snapped>,
    /// The typing into the number began this frame.
    fresh: bool,
    /// The face the fourth dot lays the plane on, and the selection and
    /// scene it was found for (`Shell::sync_face_choice`).
    pub(crate) face: Option<FaceChoice>,
    face_for: Option<(u64, u64, (u64, Option<u64>))>,
}

impl Shell {
    /// Where the section's handle was drawn in the last frame: `None`
    /// while it is not shown (the section off, the narrow layout, a
    /// document without bodies).
    pub fn section_handle(&self) -> Option<&HandlePlace> {
        self.section_handle.place.as_ref()
    }

    /// The face the plane would be laid on (the display bar's fourth
    /// cell, the handle's fourth dot): the one face selected, with why
    /// it cannot be when it is not flat.
    pub fn section_face_choice(&self) -> Option<&FaceChoice> {
        self.section_handle.face.as_ref()
    }

    /// Whether the section's plane is that of a face (laid on it by the
    /// fourth dot, or `ui.set` with a face's name).
    pub fn section_on_face(&self) -> bool {
        self.section_cache.as_ref().is_some_and(|cache| cache.face)
    }

    /// Whether the fourth dot and the bar's fourth cell are shown: one
    /// face selected, or the plane a face's already.
    pub(super) fn face_option(&self) -> bool {
        self.section_handle.face.is_some() || self.section_on_face()
    }

    /// Finds again, when the selection or the scene changed, the face the
    /// fourth dot would lay the plane on: the one face selected, flat or
    /// with why not (the engine's own test, `Session::section_plane`).
    pub(crate) fn sync_face_choice(
        &mut self,
        session: &mut craft3d_engine::Session,
        scene: &Scene,
    ) {
        let (tolerance, angle) = scene.sampling();
        let key = (
            session.selection_serial(),
            scene.revision,
            (tolerance.to_bits(), angle.map(f64::to_bits)),
        );
        if self.section_handle.face_for == Some(key) {
            return;
        }
        self.section_handle.face_for = Some(key);
        let one = match self.selection().items.as_slice() {
            [one] if one.kind == craft3d_engine::SelectionKind::Face => one.clone(),
            _ => {
                self.section_handle.face = None;
                return;
            }
        };
        let spec = craft3d_engine::SectionSpec {
            plane: Some(one.name.clone()),
            feature: Some(one.feature.clone()),
            ..craft3d_engine::SectionSpec::default()
        };
        let refused = session.section_plane(scene, &spec).err().map(|why| {
            if why.contains("not flat") {
                "Not a flat face: the plane lies along a flat face".to_string()
            } else {
                why
            }
        });
        self.section_handle.face = Some(FaceChoice {
            feature: one.feature,
            face: one.name,
            refused,
        });
    }

    /// What the drag of the section's arrow under way snapped to last.
    pub fn section_snapped(&self) -> Option<Snapped> {
        self.section_handle
            .drag
            .as_ref()
            .filter(|drag| !drag.spent)
            .and(self.section_handle.snapped)
    }

    /// Whether the handle is shown: the section on and resolved, the
    /// display bar with its Section control, not the narrow layout.
    fn handle_shown(&self) -> bool {
        self.state.section.on
            && self.section_cache.is_some()
            && !self.narrow
            && self
                .controls_layout
                .is_some_and(|layout| layout.section_toggle.is_some())
    }

    /// A drag on the section's arrow, before anything else a drag in the
    /// viewport does: true while it moves the plane.
    pub(super) fn handle_drag(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        rect: Rect,
    ) -> bool {
        if response.drag_started() {
            self.section_handle.drag = None;
            let press = ui.input(|i| i.pointer.press_origin());
            let on_arrow = self
                .section_handle
                .place
                .as_ref()
                .zip(press)
                .filter(|(place, at)| place.part_at(*at) == Some(HandlePart::Arrow))
                .map(|(place, at)| (place.frame.clone(), at));
            if let Some((frame, at)) = on_arrow
                && let Some(plane) = self.section_cache.as_ref().map(|c| c.plane)
                && let Some(from) = self.ray_on_line(rect, at, frame.middle, frame.normal)
            {
                self.section_handle.drag = Some(Drag {
                    middle: frame.middle,
                    normal: frame.normal,
                    from,
                    offset: self.state.section.offset,
                    sign: if plane.normal.dot(plane.along) < 0.0 {
                        -1.0
                    } else {
                        1.0
                    },
                    expected: self.state.section.clone(),
                    document: self.shown_status.as_ref().map(|s| s.id),
                    spent: false,
                });
                self.section_handle.snapped = None;
            }
        }
        let Some(mut drag) = self.section_handle.drag.clone() else {
            return false;
        };
        if !response.dragged() {
            self.section_handle.drag = None;
            self.section_handle.snapped = None;
            return false;
        }
        // Another writer changed the section (an agent, another tab shown,
        // the plane lost): the drag is over; held until let go, moving
        // nothing.
        let document = self.shown_status.as_ref().map(|s| s.id);
        if drag.spent
            || !self.handle_shown()
            || self.state.section != drag.expected
            || document != drag.document
        {
            drag.spent = true;
            self.section_handle.drag = Some(drag);
            self.section_handle.snapped = None;
            return true;
        }
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        let Some(at) = response.interact_pointer_pos() else {
            return true;
        };
        let Some(t) = self.ray_on_line(rect, at, drag.middle, drag.normal) else {
            return true;
        };
        let raw = drag.offset + (t - drag.from) * drag.sign;
        let (range, places) = match &self.section_cache {
            Some(cache) => (
                cache.range.unwrap_or([f64::NAN; 2]),
                std::sync::Arc::clone(&cache.snaps),
            ),
            None => return true,
        };
        let per_point = self
            .section_handle
            .place
            .as_ref()
            .map_or(0.0, |p| p.frame.per_point);
        let reach = f64::from(self.tokens.section_snap_reach) * per_point;
        let free = ui.input(|i| i.modifiers.alt);
        let (offset, snap) = snap_offset(raw, &places, reach, per_point, range, free);
        self.section_handle.snapped = snap;
        let mut section = self.state.section.clone();
        section.offset = offset;
        self.commit_section(ui, section);
        drag.expected = self.state.section.clone();
        self.section_handle.drag = Some(drag);
        true
    }

    /// Where the pointer's ray at `at` (in the window's points, in the
    /// viewport `rect`) meets the line through `middle` along `normal`.
    fn ray_on_line(&self, rect: Rect, at: Pos2, middle: Vec3, normal: Vec3) -> Option<f64> {
        let projector = Projector::new(
            &self.state.camera.to_render(),
            f64::from(rect.width()),
            f64::from(rect.height()),
        )?;
        let (start, way) =
            projector.ray(f64::from(at.x - rect.left()), f64::from(at.y - rect.top()));
        nearest_on_line(middle, normal, start, way)
    }

    /// A click on the section's handle: a double click on the arrow keeps
    /// the other half, a click on the number types into it, a dot turns
    /// the plane. True when the click was on the handle, which then
    /// selects nothing.
    pub(super) fn handle_click(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        scene: Option<&Scene>,
    ) -> bool {
        self.section_handle.fresh = false;
        if !response.clicked() {
            return false;
        }
        let part = self
            .section_handle
            .place
            .as_ref()
            .zip(response.interact_pointer_pos())
            .and_then(|(place, at)| place.part_at(at));
        let Some(part) = part else {
            return false;
        };
        let mut section = self.state.section.clone();
        match part {
            HandlePart::Arrow => {
                if response.double_clicked() {
                    section.flip = !section.flip;
                }
            }
            HandlePart::Number => {
                self.typing = Some(Typing {
                    field: NUMBER_FIELD,
                    text: shown(section.offset, 1.0, 2, ""),
                    ..Typing::default()
                });
                self.fields.focus = None;
                self.section_handle.fresh = true;
            }
            HandlePart::Dot(FACE_DOT) => {
                if let Some(choice) = self.section_handle.face.clone() {
                    choose_face(&mut section, &choice);
                }
            }
            HandlePart::Dot(axis) => choose_axis(&mut section, axis, scene),
            HandlePart::Flip => section.flip = !section.flip,
        }
        self.commit_section(ui, section);
        ui.ctx().request_repaint();
        true
    }

    /// Paints the section's handle over the viewport `rect`, and keeps
    /// where it went for the next frame's pointer.
    pub(super) fn paint_section_handle(&mut self, ui: &mut egui::Ui, rect: Rect) {
        // The plates, the cube and the triad (as the last frame laid them
        // out): the arrow and its group keep clear of them.
        let keep_out: Vec<Rect> = self
            .controls_layout
            .as_ref()
            .map(|layout| layout.parts().into_iter().map(|(_, r)| r).collect())
            .unwrap_or_default();
        let frame = if self.handle_shown() {
            self.section_cache.as_ref().and_then(|cache| {
                plane_frame(
                    &self.state.camera,
                    rect,
                    &keep_out,
                    &cache.plane,
                    &cache.bounds,
                    &self.tokens,
                )
            })
        } else {
            None
        };
        let Some(frame) = frame else {
            self.section_handle.place = None;
            self.section_handle.drag = None;
            if self
                .typing
                .as_ref()
                .is_some_and(|t| t.field == NUMBER_FIELD)
            {
                self.typing = None;
            }
            return;
        };
        let tokens = self.tokens;
        let kit = Kit::plate(&tokens);
        let typing = self
            .typing
            .as_ref()
            .is_some_and(|t| t.field == NUMBER_FIELD);
        let pointer = ui.input(|i| i.pointer.hover_pos());
        // The axis's letter and the offset in the document's unit.
        let letter = chosen_axis(&self.state.section)
            .and_then(|k| SECTION_PLANES.get(k))
            .map(|(name, _)| name.to_uppercase());
        let number = shown(self.state.section.offset, 1.0, 2, self.length_unit);
        let font = kit.font();
        let letter_galley = letter.map(|l| {
            ui.painter()
                .layout_no_wrap(l, font.clone(), color(tokens.accent))
        });
        let number_galley =
            ui.painter()
                .layout_no_wrap(number, font.clone(), color(tokens.plate_text));
        let letter_width = letter_galley
            .as_ref()
            .map_or(0.0, |g| g.size().x + PAD.x * 0.7);
        let mut text_width = letter_width + number_galley.size().x;
        if typing {
            text_width = text_width.max(96.0);
        }
        // The group: the number, then the buttons X, Y, Z (and ∥), then
        // Flip side, on one plate as high as the controls on the bar.
        let height = tokens.nav_height;
        let inset = 2.0;
        let side = height - 2.0 * inset;
        let buttons = if self.face_option() { 5 } else { 4 };
        let number_width = text_width + PAD.x * 2.0;
        let size = vec2(
            number_width + buttons as f32 * (side + inset) + inset,
            height,
        );
        let group = group_place(&frame, rect, &keep_out, size);
        let number_rect = Rect::from_min_size(group.min, vec2(number_width, height));
        let centre_of = |k: usize| {
            pos2(
                group.left() + number_width + inset + side * 0.5 + k as f32 * (side + inset),
                group.center().y,
            )
        };
        let place = HandlePlace {
            frame,
            group,
            number: number_rect,
            dots: (0..buttons - 1).map(centre_of).collect(),
            flip: centre_of(buttons - 1),
            dot_radius: side * 0.5,
            hit: tokens.section_hit,
        };
        let dragging = self.section_handle.drag.as_ref().is_some_and(|d| !d.spent);
        let under = pointer
            .filter(|p| rect.contains(*p))
            .and_then(|p| place.part_at(p));
        let near = dragging
            || typing
            || pointer.is_some_and(|p| rect.contains(p) && place.near(p, tokens.section_near));
        let painter = ui.painter_at(rect);
        // The outline, dashed, in the accent alone, faint until the pointer
        // is near: one path, one dash pattern, one line seen edge on.
        let outline_alpha = if near {
            255
        } else {
            tokens.section_rest_opacity
        };
        let [dash, gap] = tokens.section_dash;
        for path in outline_paths(&place.frame.outline) {
            painter.extend(egui::Shape::dashed_line(
                &path,
                egui::Stroke::new(
                    tokens.section_outline_width,
                    color_alpha(tokens.accent, outline_alpha),
                ),
                dash,
                gap,
            ));
        }
        // The arrow, or the ring seen end on: a light fill with a dark
        // edge, so it reads on the cap, a body and the background; the
        // accent under the pointer and while it is dragged.
        let hot = dragging || under == Some(HandlePart::Arrow);
        let fill = color(if hot {
            tokens.accent
        } else {
            tokens.section_handle_fill
        });
        let edge = egui::Stroke::new(
            tokens.section_handle_edge_width,
            color(tokens.section_handle_edge),
        );
        let w = tokens.section_arrow_width;
        let at = place.frame.at;
        match place.frame.arrow {
            Some([a, b]) => {
                let way = (b - a).normalized();
                let across = vec2(-way.y, way.x);
                let head = tokens.section_head;
                let neck = b - way * head;
                let half = w * 0.5;
                let shaft = vec![
                    a + across * half,
                    neck + across * half,
                    neck - across * half,
                    a - across * half,
                ];
                let tip = vec![
                    b,
                    neck + across * (head * 0.5),
                    neck - across * (head * 0.5),
                ];
                // The edge first, all round, then the fill over it.
                let mut round = vec![
                    a + across * half,
                    neck + across * half,
                    neck + across * (head * 0.5),
                    b,
                    neck - across * (head * 0.5),
                    neck - across * half,
                    a - across * half,
                ];
                if let Some(&first) = round.first() {
                    round.push(first);
                }
                painter.add(egui::Shape::line(
                    round,
                    egui::Stroke::new(edge.width * 2.0, edge.color),
                ));
                painter.circle(a, w * 0.9, fill, edge);
                painter.add(egui::Shape::convex_polygon(shaft, fill, egui::Stroke::NONE));
                painter.add(egui::Shape::convex_polygon(tip, fill, egui::Stroke::NONE));
                // A snap: a short tick across the arrow's foot.
                if dragging && self.section_handle.snapped.is_some() {
                    let reach = across * (w + 5.0);
                    painter
                        .line_segment([a - reach, a + reach], egui::Stroke::new(3.5, edge.color));
                    painter.line_segment([a - reach, a + reach], egui::Stroke::new(1.5, fill));
                }
            }
            None => {
                let r = tokens.section_head * 0.6;
                painter.circle(at, r, fill, edge);
                if place.frame.toward {
                    painter.circle_filled(at, r * 0.3, edge.color);
                } else {
                    let d = r * 0.45;
                    for [p, q] in [
                        [at + vec2(-d, -d), at + vec2(d, d)],
                        [at + vec2(-d, d), at + vec2(d, -d)],
                    ] {
                        painter.line_segment([p, q], egui::Stroke::new(1.6, edge.color));
                    }
                }
            }
        }
        // The group's plate, always legible while the section is on.
        let radius = egui::CornerRadius::same(tokens.radius);
        painter.rect_filled(
            place.group,
            radius,
            color_alpha(tokens.plate_fill, tokens.plate_opacity),
        );
        painter.rect_stroke(
            place.group,
            radius,
            egui::Stroke::new(1.0, color_alpha(tokens.plate_rim, tokens.plate_rim_opacity)),
            egui::StrokeKind::Inside,
        );
        if typing {
            let fresh = self.section_handle.fresh;
            kit.typed_field(
                ui,
                ui.id().with("section-number"),
                place.number.shrink(inset),
                self.length_unit,
                &mut self.typing,
                fresh,
            );
        } else {
            let mut x = place.number.left() + PAD.x;
            let y = place.number.center().y - number_galley.size().y * 0.5;
            if let Some(galley) = letter_galley {
                painter.galley(pos2(x, y), galley, color(tokens.accent));
                x += letter_width;
            }
            painter.galley(pos2(x, y), number_galley, color(tokens.plate_text));
        }
        // The buttons: the axis the plane is across, and Flip side while it
        // is flipped, filled with the accent; ∥ greyed when the face
        // selected is not flat.
        let chosen = if self.section_on_face() {
            Some(FACE_DOT)
        } else {
            chosen_axis(&self.state.section)
        };
        let refused = self
            .section_handle
            .face
            .as_ref()
            .and_then(|f| f.refused.clone());
        let segment = egui::CornerRadius::same(tokens.nav_segment_radius);
        let letters = egui::FontId::proportional(tokens.control_font);
        let mut cells: Vec<(HandlePart, Pos2, bool)> = place
            .dots
            .iter()
            .enumerate()
            .map(|(k, &c)| (HandlePart::Dot(k), c, chosen == Some(k)))
            .collect();
        cells.push((HandlePart::Flip, place.flip, self.state.section.flip));
        for (part, middle, on) in cells {
            let cell = Rect::from_center_size(middle, egui::Vec2::splat(side));
            let greyed = part == HandlePart::Dot(FACE_DOT) && !on && refused.is_some();
            if on {
                painter.rect_filled(cell, segment, color(tokens.accent));
            } else if under == Some(part) && !greyed {
                painter.rect_filled(
                    cell,
                    segment,
                    color_alpha(tokens.glass, tokens.control_opacity),
                );
            }
            let ink = color(if on {
                tokens.knob
            } else if greyed {
                tokens.plate_text_dim
            } else {
                tokens.plate_text
            });
            match part {
                HandlePart::Dot(k) if k < SECTION_PLANES.len() => {
                    if let Some((name, _)) = SECTION_PLANES.get(k) {
                        painter.text(
                            middle - vec2(0.0, 1.0),
                            egui::Align2::CENTER_CENTER,
                            name.to_uppercase(),
                            letters.clone(),
                            ink,
                        );
                    }
                }
                HandlePart::Dot(_) => {
                    super::icons::paint(
                        &painter,
                        super::icons::Icon::Parallel,
                        cell.shrink(side * 0.22),
                        ink,
                        &tokens,
                    );
                }
                _ => {
                    super::icons::paint(
                        &painter,
                        super::icons::Icon::Flip,
                        cell.shrink(side * 0.2),
                        ink,
                        &tokens,
                    );
                }
            }
        }
        // What the pointer would take, and what a button does.
        if !dragging && let Some(part) = under {
            ui.ctx().set_cursor_icon(match part {
                HandlePart::Arrow => egui::CursorIcon::Grab,
                HandlePart::Number => egui::CursorIcon::Text,
                HandlePart::Dot(FACE_DOT) if refused.is_some() && chosen != Some(FACE_DOT) => {
                    egui::CursorIcon::NotAllowed
                }
                HandlePart::Dot(_) | HandlePart::Flip => egui::CursorIcon::PointingHand,
            });
            let words = match part {
                HandlePart::Dot(FACE_DOT) => Some(
                    refused
                        .clone()
                        .unwrap_or_else(|| "Parallel to the selected face".to_string()),
                ),
                HandlePart::Flip => Some("Keep the other half".to_string()),
                _ => None,
            };
            if let Some(words) = words {
                let galley =
                    painter.layout_no_wrap(words, kit.small_font(), color(tokens.notice_text));
                let below = place.group.bottom() + 6.0;
                let at = if below + galley.size().y + 6.0 < rect.bottom() {
                    pos2(place.group.left(), below)
                } else {
                    pos2(
                        place.group.left(),
                        place.group.top() - 6.0 - galley.size().y,
                    )
                };
                let back = Rect::from_min_size(at, galley.size()).expand2(vec2(6.0, 3.0));
                painter.rect_filled(
                    back,
                    radius,
                    color_alpha(tokens.notice_fill, tokens.notice_opacity),
                );
                painter.galley(at, galley, color(tokens.notice_text));
            }
        }
        self.section_handle.place = Some(place);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn place(offset: f64, kind: SnapKind) -> SectionSnap {
        SectionSnap {
            offset,
            kind,
            feature: "F1".to_string(),
            name: None,
        }
    }

    #[test]
    fn the_outline_is_where_the_plane_passes_through_the_box() {
        let (lo, hi) = (Vec3::ZERO, Vec3::new(60.0, 40.0, 20.0));
        let area = |p: &[(f64, f64)]| {
            let n = p.len();
            (0..n)
                .map(|k| {
                    let (a, b) = (p[k], p[(k + 1) % n]);
                    a.0 * b.1 - b.0 * a.1
                })
                .sum::<f64>()
                .abs()
                * 0.5
        };
        let (u, v) = in_plane(Vec3::X).unwrap();
        // Across x: the 40 × 20 cross-section, in order round it.
        let cut = box_cut(lo, hi, Vec3::new(30.0, 0.0, 0.0), Vec3::X, u, v).unwrap();
        assert_eq!(cut.len(), 4);
        assert!((area(&cut) - 800.0).abs() < 1e-9, "{cut:?}");
        // On a face of the box: that face.
        let cut = box_cut(lo, hi, Vec3::ZERO, Vec3::X, u, v).unwrap();
        assert!((area(&cut) - 800.0).abs() < 1e-9, "{cut:?}");
        // At a slant past a corner: a triangle with legs of 6 along x, y
        // and z from (60, 40, 20), in the plane x + y + z = 114.
        let n = Vec3::new(1.0, 1.0, 1.0).normalized().unwrap();
        let (u, v) = in_plane(n).unwrap();
        let cut = box_cut(lo, hi, Vec3::new(54.0, 40.0, 20.0), n, u, v).unwrap();
        assert_eq!(cut.len(), 3, "{cut:?}");
        let side = 6.0 * std::f64::consts::SQRT_2;
        assert!(
            (area(&cut) - 3.0_f64.sqrt() / 4.0 * side * side).abs() < 1e-9,
            "{cut:?}"
        );
        // Beyond the box, and touching it at a corner only: none.
        assert_eq!(
            box_cut(lo, hi, Vec3::new(99.0, 0.0, 0.0), Vec3::X, u, v),
            None
        );
        assert_eq!(box_cut(lo, hi, hi, n, u, v), None);
    }

    #[test]
    fn the_nearest_point_of_the_arrows_line_to_a_ray() {
        // Where the ray's nearest place to the line is behind its start
        // (the eye): no answer, so a drag holds (review of 2026-11-17).
        assert_eq!(
            nearest_on_line(
                Vec3::ZERO,
                Vec3::X,
                Vec3::new(3.0, 0.0, 10.0),
                Vec3::new(0.0, 0.0, 1.0)
            ),
            None
        );
        // A ray down the z axis through (3, 0, ·) meets the x axis at 3.
        let t = nearest_on_line(
            Vec3::ZERO,
            Vec3::X,
            Vec3::new(3.0, 0.0, 10.0),
            Vec3::new(0.0, 0.0, -1.0),
        );
        assert!((t.unwrap() - 3.0).abs() < 1e-12);
        // Skew: a ray along y at height 5 through x = -2.
        let t = nearest_on_line(
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::X,
            Vec3::new(-2.0, -7.0, 5.0),
            Vec3::new(0.0, 2.0, 0.0),
        );
        assert!((t.unwrap() + 3.0).abs() < 1e-12);
        // Along the line, or not a number: no answer.
        assert_eq!(nearest_on_line(Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::X), None);
        assert_eq!(
            nearest_on_line(Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::new(1.0, 1e-5, 0.0)),
            None
        );
        assert_eq!(
            nearest_on_line(Vec3::ZERO, Vec3::X, Vec3::new(f64::NAN, 0.0, 0.0), Vec3::Z),
            None
        );
        assert_eq!(
            nearest_on_line(Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::ZERO),
            None
        );
    }

    #[test]
    fn a_drag_snaps_to_places_then_round_values_then_a_points_worth() {
        let places = [place(10.0, SnapKind::Axis), place(20.0, SnapKind::Face)];
        let range = [0.0, 60.0];
        // Within reach of the axis: on it, whatever the round values.
        assert_eq!(
            snap_offset(10.4, &places, 0.5, 0.05, range, false),
            (10.0, Some(Snapped::Place(SnapKind::Axis)))
        );
        assert_eq!(
            snap_offset(19.7, &places, 0.5, 0.05, range, false),
            (20.0, Some(Snapped::Place(SnapKind::Face)))
        );
        // Out of reach of a place, near a round value of the range's size
        // (a twentieth of 60: 5).
        assert_eq!(
            snap_offset(35.3, &places, 0.5, 0.05, range, false),
            (35.0, Some(Snapped::Round))
        );
        // Else on a grid of a point's worth: 0.05 a point, a grid of 0.05.
        let (v, s) = snap_offset(37.4321, &places, 0.5, 0.05, range, false);
        assert_eq!(s, None);
        assert!((v - 37.45).abs() < 1e-12, "{v}");
        // Alt: as it is.
        assert_eq!(
            snap_offset(10.4, &places, 0.5, 0.05, range, true),
            (10.4, None)
        );
        // Held to the range, and nothing that is not a number gets out.
        assert_eq!(snap_offset(99.0, &places, 0.5, 0.05, range, true).0, 60.0);
        assert_eq!(snap_offset(-9.0, &places, 0.5, 0.05, range, false).0, 0.0);
        assert!(
            snap_offset(f64::NAN, &places, 0.5, 0.05, range, false)
                .0
                .is_nan()
        );
        let (v, _) = snap_offset(12.0, &places, f64::NAN, f64::NAN, [f64::NAN; 2], false);
        assert_eq!(v, 12.0);
        // No places at all, a range without size.
        assert_eq!(
            snap_offset(3.3, &[], 0.0, 0.0, [5.0, 5.0], false),
            (3.3, None)
        );
    }

    #[test]
    fn the_handle_lies_on_the_plane_and_is_hit_where_it_is_drawn() {
        let tokens = Tokens::default();
        let viewport = Rect::from_min_size(pos2(100.0, 50.0), vec2(800.0, 600.0));
        let bounds = Bbox3::from_points([Vec3::ZERO, Vec3::new(60.0, 40.0, 20.0)]);
        let plane = SectionPlane {
            origin: Vec3::new(30.0, 0.0, 0.0),
            normal: Vec3::X,
            base: Vec3::ZERO,
            along: Vec3::X,
        };
        let mut camera = CameraState {
            projection: crate::state::ProjectionKind::Orthographic,
            ..CameraState::default()
        };
        camera.set_view("front").unwrap();
        camera.fit(&bounds, 800.0 / 600.0);
        let frame = plane_frame(&camera, viewport, &[], &plane, &bounds, &tokens).unwrap();
        // From the front, across x: the arrow points right, as long as the
        // tokens say, from the corner of the outline furthest along
        // x - y - z: the one at (30, 0, 0), grown out of the box's cut.
        let [a, b] = frame.arrow.unwrap();
        assert!(
            (b.x - a.x - tokens.section_arrow_length).abs() < 1e-3,
            "{a:?} {b:?}"
        );
        assert!((b.y - a.y).abs() < 1e-3);
        assert!(
            (frame.middle.x - 30.0).abs() < 1e-9
                && frame.middle.y < 0.0
                && frame.middle.z < 0.0
                && frame.middle.y > -2.0
                && frame.middle.z > -2.0,
            "{:?}",
            frame.middle
        );
        assert!(viewport.contains(a));
        // Seen edge on, the outline is the plane's trace: two vertical sides.
        assert_eq!(frame.outline.len(), 4);
        for [p, q] in &frame.outline {
            assert!(
                (p.x - a.x).abs() < 1e-2 && (q.x - a.x).abs() < 1e-2,
                "{p:?} {q:?}"
            );
        }
        // The group: in view, clear of the arrow and of the cut's middle.
        let size = vec2(200.0, 28.0);
        let group = group_place(&frame, viewport, &[], size);
        assert!(viewport.contains_rect(group), "{group:?}");
        // A control over where the arrow would stand: the arrow moves
        // toward the cut's middle, clear of it, and so does the group.
        let control = Rect::from_center_size(a, vec2(40.0, 40.0));
        let moved = plane_frame(&camera, viewport, &[control], &plane, &bounds, &tokens).unwrap();
        let [ma, mb] = moved.arrow.unwrap();
        assert!(!control.expand(INSIDE - 1.0).contains(ma), "{ma:?}");
        assert!((moved.middle.x - 30.0).abs() < 1e-9, "on the plane");
        assert!(!control.contains(mb));
        let clear = group_place(&moved, viewport, &[control], size);
        assert!(!clear.intersects(control), "{clear:?}");
        assert!((0..=8).all(|k| !group.contains(a.lerp(b, k as f32 / 8.0))));
        assert!(
            !group.contains(frame.centre),
            "{group:?} {:?}",
            frame.centre
        );
        let number = Rect::from_min_size(group.min, vec2(80.0, 28.0));
        let dots: Vec<Pos2> = (0..3)
            .map(|k| pos2(number.right() + 14.0 + 26.0 * k as f32, group.center().y))
            .collect();
        let place = HandlePlace {
            frame: frame.clone(),
            group,
            number,
            dots: dots.clone(),
            flip: pos2(number.right() + 14.0 + 26.0 * 3.0, group.center().y),
            dot_radius: 12.0,
            hit: tokens.section_hit,
        };
        assert_eq!(place.part_at(a.lerp(b, 0.5)), Some(HandlePart::Arrow));
        assert_eq!(
            place.part_at(a.lerp(b, 0.5) - vec2(0.0, tokens.section_hit + 1.0)),
            None
        );
        assert_eq!(place.part_at(number.center()), Some(HandlePart::Number));
        assert_eq!(place.part_at(dots[2]), Some(HandlePart::Dot(2)));
        assert_eq!(place.part_at(place.flip), Some(HandlePart::Flip));
        assert!(place.near(a + vec2(0.0, 30.0), tokens.section_near));
        assert!(!place.near(pos2(110.0, 60.0), tokens.section_near));
        // A ray through a point on the arrow lands on the arrow's line at
        // that point: a model unit a `per_point` points along.
        let projector = Projector::new(&camera.to_render(), 800.0, 600.0).unwrap();
        let x = f64::from(a.x - viewport.left()) + 20.0;
        let (start, way) = projector.ray(x, f64::from(a.y - viewport.top()));
        let t = nearest_on_line(frame.middle, frame.normal, start, way).unwrap();
        assert!((t - 20.0 * frame.per_point).abs() < 1e-9, "{t}");

        // Looking along the normal: a ring, no arrow to drag.
        camera.set_view("right").unwrap();
        let end_on = plane_frame(&camera, viewport, &[], &plane, &bounds, &tokens).unwrap();
        assert_eq!(end_on.arrow, None);
        assert!(
            end_on.toward,
            "the normal +x points at a viewer on the right"
        );
        camera.set_view("left").unwrap();
        assert!(
            !plane_frame(&camera, viewport, &[], &plane, &bounds, &tokens)
                .unwrap()
                .toward
        );

        // Nothing to draw: an empty box, a camera that looks nowhere, a
        // plane that is not one.
        assert_eq!(
            plane_frame(&camera, viewport, &[], &plane, &Bbox3::EMPTY, &tokens),
            None
        );
        let mut nowhere = camera;
        nowhere.distance = f64::NAN;
        assert_eq!(
            plane_frame(&nowhere, viewport, &[], &plane, &bounds, &tokens),
            None
        );
        let broken = SectionPlane {
            origin: Vec3::new(f64::NAN, 0.0, 0.0),
            ..plane
        };
        assert_eq!(
            plane_frame(&camera, viewport, &[], &broken, &bounds, &tokens),
            None
        );
        let none = SectionPlane {
            along: Vec3::ZERO,
            normal: Vec3::ZERO,
            ..plane
        };
        assert_eq!(
            plane_frame(&camera, viewport, &[], &none, &bounds, &tokens),
            None
        );
    }
}
