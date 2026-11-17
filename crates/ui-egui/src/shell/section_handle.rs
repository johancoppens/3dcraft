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
use crate::state::{CameraState, SectionMade, SectionState};
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

/// The fourth button's place among the buttons, and the bar's fourth
/// cell's among X, Y, Z: the plane from the selection.
pub const FACE_DOT: usize = 3;

/// What the fourth button says while the selection gives no plane.
pub const TAKES_FACES: &str = "Takes one or two flat faces: select them first";

/// Two faces' normals closer than this (the sine of the angle between
/// them, or between one and the other's opposite) are parallel, where the
/// document gives no angular tolerance: its default, in radians.
pub const PARALLEL: f64 = 1e-9;

/// What the fourth button would make of the selection
/// ([`Shell::sync_face_choice`]): the plane along the one flat face
/// selected, midway between two parallel flat faces, or halving the angle
/// of two that are not; or why it cannot.
#[derive(Clone, Debug, PartialEq)]
pub struct PlaneChoice {
    /// How the plane is made; `None` when the selection gives none (then
    /// `refused` says why).
    pub made: Option<SectionMade>,
    /// The faces selected, by their body's feature (`F3`) and persistent
    /// name, in the selection's order.
    pub faces: Vec<(String, String)>,
    /// The planes two faces give, each a point near the faces and a unit
    /// normal: the bisector a person means first, then the other one,
    /// square to it, where it cuts the model ([`planes_of`]).
    pub planes: Vec<([f64; 3], [f64; 3])>,
    pub refused: Option<String>,
}

impl PlaneChoice {
    fn refused(faces: Vec<(String, String)>, why: &str) -> Self {
        PlaneChoice {
            made: None,
            faces,
            planes: Vec::new(),
            refused: Some(why.to_string()),
        }
    }

    /// Which of `planes` a click takes for `section` as it is: the other
    /// bisector when the section lies on the first one already and there
    /// is another.
    fn next(&self, section: &SectionState) -> usize {
        match (self.made, self.planes.first()) {
            (Some(SectionMade::Bisector), Some((point, normal)))
                if self.planes.len() > 1
                    && section.made == Some(SectionMade::Bisector)
                    && section.point == Some(*point)
                    && section.normal == Some(*normal) =>
            {
                1
            }
            _ => 0,
        }
    }

    /// What a click on the fourth button does, for `section` as it is; or
    /// why it does nothing.
    pub fn words(&self, section: &SectionState) -> String {
        if let Some(why) = &self.refused {
            return why.clone();
        }
        match self.made {
            Some(SectionMade::Face) => "Along the selected face, through it",
            Some(SectionMade::Mid) => "Midway between the two parallel faces",
            Some(SectionMade::Bisector) if self.next(section) == 1 => {
                "The other bisector of the two faces, square to this one"
            }
            Some(SectionMade::Bisector) => "Halving the angle of the two faces",
            None => TAKES_FACES,
        }
        .to_string()
    }
}

/// The plane `choice` makes, set on `section`, offset 0, nothing kept
/// flipped: along the one face, through it, with its outward normal
/// (what lies outside the face is cut away, so nothing is yet: the arrow,
/// dragged in, cuts into the body); midway between two; or the bisector
/// through the material, and at a second click the other one. The plane
/// it lies on already (but a bisector) keeps its offset; a choice that
/// gives no plane leaves the section as it is.
pub fn choose_plane(section: &mut SectionState, choice: &PlaneChoice) {
    if choice.refused.is_some() {
        return;
    }
    match choice.made {
        Some(SectionMade::Face) => {
            let Some((feature, face)) = choice.faces.first() else {
                return;
            };
            section.made = Some(SectionMade::Face);
            if section.plane.as_deref() == Some(face.as_str())
                && section.feature.as_deref() == Some(feature.as_str())
            {
                return;
            }
            section.plane = Some(face.clone());
            section.feature = Some(feature.clone());
            section.point = None;
            section.normal = None;
        }
        Some(made @ (SectionMade::Mid | SectionMade::Bisector)) => {
            let Some(&(point, normal)) = choice.planes.get(choice.next(section)) else {
                return;
            };
            if made == SectionMade::Mid
                && section.made == Some(made)
                && section.point == Some(point)
                && section.normal == Some(normal)
            {
                return;
            }
            section.plane = None;
            section.feature = None;
            section.point = Some(point);
            section.normal = Some(normal);
            section.made = Some(made);
        }
        None => return,
    }
    section.offset = 0.0;
    section.flip = false;
}

/// The planes two flat faces give, each face by a point of its plane and
/// its unit outward normal `n`, `m`: the two bisectors of the angle of
/// their planes, square to each other through the line where those meet
/// (the review of 2026-11-17, round three: a rule by where the planes meet
/// counted sides 28 degrees apart as parallel). `x · (n − m) = p · n −
/// q · m` holds the points as far behind the one face as behind the other
/// (through the material, behind both); `x · (n + m) = p · n + q · m` those
/// as far behind the one as in front of the other.
///
/// First the better conditioned of the two, the longer of `n − m` and
/// `n + m`: the plane between the faces a person means, through the
/// material where they face apart, between them where they face the same
/// way. As the faces turn parallel it turns, continuously, into the plane
/// midway between them, which it is when they are (the other then has no
/// direction and is left out); the choice changes only where the faces
/// are square to each other, where the two bisectors are alike. Only
/// where that one does not cut the model (`cuts`: a convex edge's outer
/// bisector, which touches it along the edge) and the other does, the
/// other comes first. The other second, where it has a direction and cuts
/// the model (a pair a hair off parallel meets far outside, and its other
/// bisector cuts nothing). `made` is midway where the faces are parallel
/// within `parallel` (radians: the document's angular tolerance), else a
/// bisector. Each plane by its point nearest the middle of the two points
/// given and its unit normal; `None` when a number is not one.
pub fn planes_of(
    a: (Vec3, Vec3),
    b: (Vec3, Vec3),
    cuts: &dyn Fn(Vec3, Vec3) -> bool,
    parallel: f64,
) -> Option<(SectionMade, Vec<(Vec3, Vec3)>)> {
    let ((p, n), (q, m)) = (a, b);
    let middle = p.lerp(q, 0.5);
    // The point of the plane `x . w = c` nearest the middle.
    let nearest = |w: Vec3, c: f64| {
        let length = w.length_squared();
        let point = middle + w * ((c - middle.dot(w)) / length);
        let normal = w.normalized()?;
        (length > 0.0 && point.is_finite() && normal.is_finite()).then_some((point, normal))
    };
    let unit = |v: Vec3| (v.length() - 1.0).abs() < 1e-6;
    if !(middle.is_finite() && unit(n) && unit(m)) {
        return None;
    }
    let through = (n - m, p.dot(n) - q.dot(m));
    let between = (n + m, p.dot(n) + q.dot(m));
    let (good, poor) = if through.0.length_squared() >= between.0.length_squared() {
        (through, between)
    } else {
        (between, through)
    };
    let good = nearest(good.0, good.1)?;
    let poor = nearest(poor.0, poor.1);
    let (first, second) = match poor {
        Some(poor) if !cuts(good.0, good.1) && cuts(poor.0, poor.1) => (poor, None),
        _ => (good, poor.filter(|&(point, normal)| cuts(point, normal))),
    };
    let made = if n.cross(m).length() <= parallel.max(0.0) {
        SectionMade::Mid
    } else {
        SectionMade::Bisector
    };
    Some((made, std::iter::once(first).chain(second).collect()))
}

/// Whether the plane through `point` with unit `normal` cuts the model
/// `scene` shows: its mesh's corners lie on both sides, beyond a
/// billionth of the model's size.
fn model_cut(scene: &Scene, point: Vec3, normal: Vec3) -> bool {
    let reach = scene
        .bodies_box
        .corners()
        .map_or(1.0, |(lo, hi)| lo.distance(hi));
    let eps = if reach.is_finite() { reach * 1e-9 } else { 0.0 };
    separates(&scene.mesh.positions, point, normal, eps)
}

/// Whether the plane through `point` with unit `normal` cuts what
/// `points` hold: some lie farther than `eps` on either side of it. The
/// model's mesh in the window; a box's corners in a test.
pub fn separates(points: &[Vec3], point: Vec3, normal: Vec3, eps: f64) -> bool {
    let (mut below, mut above) = (false, false);
    for &q in points {
        let s = (q - point).dot(normal);
        below |= s < -eps;
        above |= s > eps;
        if below && above {
            return true;
        }
    }
    false
}

/// The two faces the plane midway or the bisector was made from, and the
/// plane, as the fourth button set it.
#[derive(Clone, Debug, PartialEq)]
struct MadeFrom {
    faces: Vec<(String, String)>,
    made: SectionMade,
    point: [f64; 3],
    normal: [f64; 3],
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
/// goes beside the arrow's foot in the `viewport`, and the way it went
/// from the foot on the screen. It keeps the way it went last (`kept`)
/// while that place stays clear and in view (the review of 2026-11-17: it
/// hopped sides hundreds of times while the view turned), so it follows
/// the foot smoothly (pushed along the viewport's edge where it reaches
/// it). Else, the first clear place of: away from the cut's
/// middle (so it never covers it), square to the arrow, back along it,
/// then round the compass. Clear: off the arrow and its foot, [`GAP`]
/// clear of `keep_out` (the controls over the view), within the viewport;
/// where no place is, the one that covers the controls least.
pub fn group_place(
    frame: &PlaneFrame,
    viewport: Rect,
    keep_out: &[Rect],
    size: Vec2,
    kept: Option<Vec2>,
) -> (Rect, Vec2) {
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
    if let Some(way) = arrow.filter(|w| w.x.is_finite() && w.y.is_finite()) {
        let across = vec2(-way.y, way.x);
        let across = if across.dot(out) >= 0.0 {
            across
        } else {
            -across
        };
        ways.extend([across, -across, -way]);
    }
    ways.extend((0..8).map(|k| {
        let t = k as f32 * std::f32::consts::FRAC_PI_4;
        vec2(t.pcos(), t.psin())
    }));
    // The place a way gives, and whether it had to be pushed into view.
    let placed = |way: Vec2| {
        let reach = way.x.abs() * size.x * 0.5 + way.y.abs() * size.y * 0.5;
        let place = Rect::from_center_size(at + way * (GAP + reach), size);
        let shift = vec2(
            (room.left() - place.left()).max(0.0) - (place.right() - room.right()).max(0.0),
            (room.top() - place.top()).max(0.0) - (place.bottom() - room.bottom()).max(0.0),
        );
        if shift.x.is_finite() && shift.y.is_finite() {
            (place.translate(shift), shift.length() > 0.5)
        } else {
            (place, false)
        }
    };
    let on_arrow = |place: &Rect| {
        let grown = place.expand(GAP * 0.5);
        frame
            .arrow
            .is_some_and(|[a, b]| (0..=8).any(|k| grown.contains(a.lerp(b, k as f32 / 8.0))))
            || grown.contains(at)
    };
    // How much of the controls (with air round them) a place covers.
    let covered = |place: &Rect| -> f32 {
        keep_out
            .iter()
            .map(|r| {
                let both = r.expand(GAP).intersect(*place);
                if both.is_positive() { both.area() } else { 0.0 }
            })
            .sum()
    };
    let clear = |place: &Rect| !on_arrow(place) && covered(place) <= 0.0;
    // Kept while clear: pushed along the viewport's edge, it slides
    // with the foot rather than hop.
    if let Some(way) = kept.filter(|w| w.x.is_finite() && w.y.is_finite()) {
        let (place, _) = placed(way);
        if clear(&place) {
            return (place, way);
        }
    }
    let candidates: Vec<(Rect, bool, Vec2)> = ways
        .iter()
        .map(|&way| {
            let (place, pushed) = placed(way);
            (place, pushed, way)
        })
        .collect();
    candidates
        .iter()
        .find(|(place, pushed, _)| !pushed && clear(place))
        .or_else(|| candidates.iter().find(|(place, _, _)| clear(place)))
        .or_else(|| {
            candidates
                .iter()
                .filter(|(place, _, _)| !on_arrow(place))
                .min_by(|a, b| covered(&a.0).total_cmp(&covered(&b.0)))
        })
        .map(|&(place, _, way)| (place, way))
        .unwrap_or_else(|| {
            let (place, _) = placed(out);
            (place, out)
        })
}

/// The head of the slim arrow (the developer's choice of 2026-11-17,
/// variant B), its tip at `tip`, pointing along `way` (a unit vector): a
/// notched dart [`Tokens::section_head`] long and
/// [`Tokens::section_head_width`] wide, its corners in order round it
/// (a side, the tip, the other side, the notch). The dimension lines
/// (`dimension.rs`) have the same, so the section's arrow and a dimension
/// read as one family.
pub fn dart(tip: Pos2, way: Vec2, tokens: &Tokens) -> [Pos2; 4] {
    let across = vec2(-way.y, way.x);
    let back = tip - way * tokens.section_head;
    let half = tokens.section_head_width * 0.5;
    [
        back + across * half,
        tip,
        back - across * half,
        back + way * tokens.section_head_notch,
    ]
}

/// Paints the [`dart`] at `tip` along `way`: `fill` inside `edge`.
pub(super) fn paint_dart(
    painter: &egui::Painter,
    tip: Pos2,
    way: Vec2,
    tokens: &Tokens,
    fill: egui::Color32,
    edge: egui::Stroke,
) {
    if !(way.x.is_finite() && way.y.is_finite()) {
        return;
    }
    let [side, point, other, notch] = dart(tip, way, tokens);
    // Two convex halves, either side of the line from the notch to the
    // tip, then the edge all round.
    painter.add(egui::Shape::convex_polygon(
        vec![side, point, notch],
        fill,
        egui::Stroke::NONE,
    ));
    painter.add(egui::Shape::convex_polygon(
        vec![notch, point, other],
        fill,
        egui::Stroke::NONE,
    ));
    painter.add(egui::Shape::closed_line(
        vec![side, point, other, notch],
        edge,
    ));
}

/// Paints a slim line from `a` to `b` with round ends: a core `width`
/// wide of `fill` with `edge` along each side.
pub(super) fn paint_slim_line(
    painter: &egui::Painter,
    a: Pos2,
    b: Pos2,
    width: f32,
    fill: egui::Color32,
    edge: egui::Stroke,
) {
    let outer = width + 2.0 * edge.width;
    for (w, ink) in [(outer, edge.color), (width, fill)] {
        painter.line_segment([a, b], egui::Stroke::new(w, ink));
        painter.circle_filled(a, w * 0.5, ink);
        painter.circle_filled(b, w * 0.5, ink);
    }
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
    /// The plane the fourth button makes of the selection, and the
    /// selection and scene it was found for (`Shell::sync_face_choice`).
    pub(crate) face: Option<PlaneChoice>,
    face_for: Option<(u64, u64, (u64, Option<u64>))>,
    /// The way the group went from the arrow's foot last frame, kept
    /// while its place stays clear ([`group_place`]).
    group_way: Option<Vec2>,
    /// The faces a plane midway or a bisector was made from, per open
    /// document (each tab has its own section; [`Shell::remember_made`]).
    made_from: std::collections::BTreeMap<u64, MadeFrom>,
}

impl Shell {
    /// Where the section's handle was drawn in the last frame: `None`
    /// while it is not shown (the section off, the narrow layout, a
    /// document without bodies).
    pub fn section_handle(&self) -> Option<&HandlePlace> {
        self.section_handle.place.as_ref()
    }

    /// The plane the fourth button (the handle's, and the display bar's
    /// cell in the narrow layout) makes of the selection, with why it
    /// cannot when it does not give one. `None` before a scene is drawn.
    pub fn section_face_choice(&self) -> Option<&PlaneChoice> {
        self.section_handle.face.as_ref()
    }

    /// Whether the section's plane is the one the selection made (the
    /// fourth button chosen): on a face, midway or a bisector.
    pub(super) fn section_from_selection(&self) -> bool {
        self.section_on_face()
            || matches!(
                self.state.section.made,
                Some(SectionMade::Mid | SectionMade::Bisector)
            )
    }

    /// Whether the section's plane is that of a face (laid on it by the
    /// fourth dot, or `ui.set` with a face's name).
    pub fn section_on_face(&self) -> bool {
        self.section_cache.as_ref().is_some_and(|cache| cache.face)
    }

    /// Finds again, when the selection or the scene changed, the plane the
    /// fourth button would make: of one flat face selected, or two (each
    /// face's plane by the engine's own test, `Session::section_plane`);
    /// anything else gives none, with why.
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
        self.check_made(session, scene);
        let items = &self.selection().items;
        let faces: Vec<(String, String)> = items
            .iter()
            .filter(|item| item.kind == craft3d_engine::SelectionKind::Face)
            .map(|item| (item.feature.clone(), item.name.clone()))
            .collect();
        if faces.is_empty() || faces.len() > 2 || faces.len() != items.len() {
            self.section_handle.face = Some(PlaneChoice::refused(faces, TAKES_FACES));
            return;
        }
        let mut planes = Vec::new();
        for (feature, face) in &faces {
            let spec = craft3d_engine::SectionSpec {
                plane: Some(face.clone()),
                feature: Some(feature.clone()),
                ..craft3d_engine::SectionSpec::default()
            };
            match session.section_plane(scene, &spec) {
                Ok(plane) => planes.push((plane.base, plane.along)),
                Err(why) => {
                    // In the window's words: the engine's speak of meshes
                    // and names.
                    let why = if why.contains("not flat") {
                        "Not a flat face: the plane takes one or two flat faces".to_string()
                    } else if [
                        "no area",
                        "no body has",
                        "no body of a feature",
                        "has no face",
                    ]
                    .iter()
                    .any(|words| why.contains(words))
                    {
                        "Its body is not in the view (hidden, or rolled back): show it to lay the plane on its face".to_string()
                    } else {
                        why
                    };
                    self.section_handle.face = Some(PlaneChoice::refused(faces, &why));
                    return;
                }
            }
        }
        let choice = match planes.as_slice() {
            [_] => PlaneChoice {
                made: Some(SectionMade::Face),
                faces,
                planes: Vec::new(),
                refused: None,
            },
            [a, b] => match planes_of(
                *a,
                *b,
                &|point, normal| model_cut(scene, point, normal),
                session.angular_tolerance().unwrap_or(PARALLEL),
            ) {
                Some((made, two)) => PlaneChoice {
                    made: Some(made),
                    faces,
                    planes: two
                        .into_iter()
                        .map(|(p, n)| ([p.x, p.y, p.z], [n.x, n.y, n.z]))
                        .collect(),
                    refused: None,
                },
                None => PlaneChoice::refused(faces, "The two faces give no plane"),
            },
            _ => PlaneChoice::refused(faces, TAKES_FACES),
        };
        self.section_handle.face = Some(choice);
    }

    /// Remembers which faces made `section`'s plane, when the fourth
    /// button made it midway or a bisector of `choice`'s two faces.
    pub(super) fn remember_made(&mut self, section: &SectionState, choice: &PlaneChoice) {
        let Some(document) = self.shown_status.as_ref().map(|s| s.id) else {
            return;
        };
        let made = match (section.made, section.point, section.normal) {
            (
                Some(made @ (SectionMade::Mid | SectionMade::Bisector)),
                Some(point),
                Some(normal),
            ) => Some(MadeFrom {
                faces: choice.faces.clone(),
                made,
                point,
                normal,
            }),
            _ => None,
        };
        match made {
            Some(made) => {
                self.section_handle.made_from.insert(document, made);
            }
            None => {
                self.section_handle.made_from.remove(&document);
            }
        }
    }

    /// Whether the plane midway or the bisector the fourth button made
    /// still is that of its two faces, after an edit: the plane stays
    /// where it was (a section is a view, like the camera, and does not
    /// move under the person), and once its faces no longer make it (the
    /// block lengthened, a face gone), it is no longer called midway or a
    /// bisector (the review of 2026-11-17).
    fn check_made(&mut self, session: &mut craft3d_engine::Session, scene: &Scene) {
        // The session's active document, whose section the state now
        // holds (the shown status follows only at the end of the frame's
        // sync: on the frame a tab is switched it is still the other).
        let Some(document) = session.document_status().map(|s| s.id) else {
            return;
        };
        // Forget what was made in documents no longer open.
        let open: Vec<u64> = session.document_statuses().iter().map(|s| s.id).collect();
        self.section_handle
            .made_from
            .retain(|id, _| open.contains(id));
        let Some(from) = self.section_handle.made_from.get(&document).cloned() else {
            return;
        };
        let section = &self.state.section;
        // Set otherwise since (another plane, an agent): not ours to say.
        if section.made != Some(from.made)
            || section.point != Some(from.point)
            || section.normal != Some(from.normal)
        {
            self.section_handle.made_from.remove(&document);
            return;
        }
        let mut planes = Vec::new();
        for (feature, face) in &from.faces {
            let spec = craft3d_engine::SectionSpec {
                plane: Some(face.clone()),
                feature: Some(feature.clone()),
                ..craft3d_engine::SectionSpec::default()
            };
            match session.section_plane(scene, &spec) {
                Ok(plane) => planes.push((plane.base, plane.along)),
                Err(_) => break,
            }
        }
        let [px, py, pz] = from.point;
        let [nx, ny, nz] = from.normal;
        let (point, normal) = (Vec3::new(px, py, pz), Vec3::new(nx, ny, nz));
        let reach = scene
            .bodies_box
            .corners()
            .map_or(1.0, |(lo, hi)| lo.distance(hi).max(1e-9));
        let holds = match planes.as_slice() {
            [a, b] => planes_of(
                *a,
                *b,
                &|point, normal| model_cut(scene, point, normal),
                session.angular_tolerance().unwrap_or(PARALLEL),
            )
            .is_some_and(|(made, two)| {
                made == from.made
                    && two.iter().any(|(p, n)| {
                        (*n - normal).length() <= 1e-9 && (*p - point).dot(*n).abs() <= 1e-9 * reach
                    })
            }),
            _ => false,
        };
        if !holds {
            self.state.section.made = None;
            self.section_handle.made_from.remove(&document);
        }
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
                    choose_plane(&mut section, &choice);
                    self.remember_made(&section, &choice);
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
            self.section_handle.group_way = None;
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
        // The axis's letter (or what the plane is: along a face, midway,
        // a bisector) and the offset in the document's unit.
        let letter = chosen_axis(&self.state.section)
            .and_then(|k| SECTION_PLANES.get(k))
            .map(|(name, _)| name.to_uppercase())
            .or_else(|| {
                let made = if self.section_on_face() {
                    Some(SectionMade::Face)
                } else {
                    self.state.section.made
                };
                made.map(|made| made.label().to_string())
            });
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
        // The group: the number, then the buttons X, Y, Z and ∥, then
        // Flip side, on one plate as high as the controls on the bar.
        let height = tokens.nav_height;
        let inset = 2.0;
        let side = height - 2.0 * inset;
        let buttons = 5;
        let number_width = text_width + PAD.x * 2.0;
        let size = vec2(
            number_width + buttons as f32 * (side + inset) + inset,
            height,
        );
        let (group, way) =
            group_place(&frame, rect, &keep_out, size, self.section_handle.group_way);
        self.section_handle.group_way = Some(way);
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
                // Slim, like a dimension arrow (the developer's choice of
                // 2026-11-17): the shaft into the head's notch, the dart,
                // the ring at the foot to take it by.
                let way = (b - a).normalized();
                let across = vec2(-way.y, way.x);
                let shaft_end = b - way * (tokens.section_head - tokens.section_head_notch);
                paint_slim_line(&painter, a, shaft_end, w, fill, edge);
                paint_dart(&painter, b, way, &tokens, fill, edge);
                painter.circle(a, tokens.section_grip, fill, edge);
                painter.circle_filled(a, tokens.section_grip_dot, edge.color);
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
        // A hairline of the handle's light colour round it, so its edge
        // shows on a dark body too.
        painter.rect_stroke(
            place.group,
            radius,
            egui::Stroke::new(
                1.0,
                color_alpha(tokens.section_handle_fill, tokens.section_rest_opacity),
            ),
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
        // The buttons: the axis the plane is across (or ∥ while the plane
        // is the selection's), and Flip side while it is flipped, filled
        // with the accent; ∥ greyed while the selection gives no plane.
        let chosen = if self.section_from_selection() {
            Some(FACE_DOT)
        } else {
            chosen_axis(&self.state.section)
        };
        let choice = self.section_handle.face.clone();
        let refused = match &choice {
            Some(choice) => choice.refused.clone(),
            None => Some(TAKES_FACES.to_string()),
        };
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
                HandlePart::Dot(FACE_DOT) => Some(match &choice {
                    Some(choice) => choice.words(&self.state.section),
                    None => TAKES_FACES.to_string(),
                }),
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
        let (group, _) = group_place(&frame, viewport, &[], size, None);
        assert!(viewport.contains_rect(group), "{group:?}");
        // A control over where the arrow would stand: the arrow moves
        // toward the cut's middle, clear of it, and so does the group.
        let control = Rect::from_center_size(a, vec2(40.0, 40.0));
        let moved = plane_frame(&camera, viewport, &[control], &plane, &bounds, &tokens).unwrap();
        let [ma, mb] = moved.arrow.unwrap();
        assert!(!control.expand(INSIDE - 1.0).contains(ma), "{ma:?}");
        assert!((moved.middle.x - 30.0).abs() < 1e-9, "on the plane");
        assert!(!control.contains(mb));
        let (clear, way) = group_place(&moved, viewport, &[control], size, None);
        assert!(!clear.intersects(control.expand(GAP - 0.5)), "{clear:?}");
        // Kept on its side while that stays clear; another control over
        // it, and it goes elsewhere, clear of both.
        assert_eq!(
            group_place(&moved, viewport, &[control], size, Some(way)),
            (clear, way)
        );
        let over = clear.expand(2.0);
        let (elsewhere, other) = group_place(&moved, viewport, &[control, over], size, Some(way));
        assert_ne!(other, way);
        assert!(!elsewhere.intersects(over) && !elsewhere.intersects(control));
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
        // The arrow is drawn slim (the developer's choice of 2026-11-17);
        // it is taken as far off as before, well past what is drawn: by
        // its shaft, its head and its ring at the foot.
        // On the side away from the group, which a press takes first.
        let away = if place.group.center().y > a.y {
            -1.0
        } else {
            1.0
        };
        let off = vec2(0.0, away * (tokens.section_hit - 0.5));
        let drawn = tokens.section_arrow_width * 0.5 + tokens.section_handle_edge_width;
        assert!(tokens.section_hit - 0.5 > 3.0 * drawn, "{drawn}");
        assert!(tokens.section_hit >= 9.0);
        for along in [0.0, 0.5, 1.0] {
            assert_eq!(
                place.part_at(a.lerp(b, along) + off),
                Some(HandlePart::Arrow),
                "{along}"
            );
        }
        let [side, point, other, notch] = dart(b, (b - a).normalized(), &tokens);
        assert!((point - b).length() < 1e-4);
        assert!((side.distance(other) - tokens.section_head_width).abs() < 1e-3);
        assert!(
            (notch.distance(b) - (tokens.section_head - tokens.section_head_notch)).abs() < 1e-3
        );
        // A finger's: the whole shape larger, the grab area a finger's.
        let touch = tokens.touch();
        assert!(touch.section_hit >= touch.touch_target * 0.5);
        let k = touch.section_arrow_length / tokens.section_arrow_length;
        assert!(k >= 1.0);
        for (big, small) in [
            (touch.section_arrow_width, tokens.section_arrow_width),
            (touch.section_head, tokens.section_head),
            (touch.section_head_width, tokens.section_head_width),
            (touch.section_grip, tokens.section_grip),
        ] {
            assert!((big - small * k).abs() < 1e-3, "{big} {small} {k}");
        }
        assert!(touch.section_hit > touch.section_head_width * 0.5);
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

    fn near(a: Vec3, b: Vec3) -> bool {
        a.distance(b) < 1e-9
    }

    /// Whether a plane cuts the box from `lo` to `hi`.
    fn in_box(lo: Vec3, hi: Vec3) -> impl Fn(Vec3, Vec3) -> bool {
        let mut corners = Vec::new();
        for x in [lo.x, hi.x] {
            for y in [lo.y, hi.y] {
                for z in [lo.z, hi.z] {
                    corners.push(Vec3::new(x, y, z));
                }
            }
        }
        let eps = 1e-9 * lo.distance(hi);
        move |point, normal| separates(&corners, point, normal, eps)
    }

    /// Any plane cuts.
    fn anywhere(_: Vec3, _: Vec3) -> bool {
        true
    }

    #[test]
    fn two_faces_give_their_bisectors_the_one_a_person_means_first() {
        // A block 60 x 40 x 20 at the origin: its right face (x = 60) and
        // its top (z = 20), each by a point of it and its outward normal.
        let right = (Vec3::new(60.0, 0.0, 0.0), Vec3::X);
        let top = (Vec3::new(0.0, 40.0, 20.0), Vec3::Z);
        let block = in_box(Vec3::ZERO, Vec3::new(60.0, 40.0, 20.0));
        let (made, planes) = planes_of(right, top, &block, PARALLEL).unwrap();
        assert_eq!(made, SectionMade::Bisector);
        let h = std::f64::consts::FRAC_1_SQRT_2;
        // Square faces: through the material first, down and in from the
        // edge along (1, 0, -1); the other only touches the block along
        // its edge, so it is not offered.
        let [(p, n)] = planes.as_slice() else {
            panic!("one bisector cuts the block: {planes:?}");
        };
        assert!(near(*n, Vec3::new(h, 0.0, -h)), "{n:?}");
        assert!((Vec3::new(60.0, 17.0, 20.0) - *p).dot(*n).abs() < 1e-9);
        assert!((Vec3::new(55.0, 3.0, 15.0) - *p).dot(*n).abs() < 1e-9);
        assert!(p.distance(Vec3::new(30.0, 20.0, 10.0)) < 40.0, "{p:?}");
        // Anywhere it would cut, the other is there too, square to it.
        let (_, both) = planes_of(right, top, &anywhere, PARALLEL).unwrap();
        let [_, (q, m)] = both.as_slice() else {
            panic!("two bisectors: {both:?}");
        };
        assert!(near(*m, Vec3::new(h, 0.0, h)), "{m:?}");
        assert!((Vec3::new(65.0, 3.0, 15.0) - *q).dot(*m).abs() < 1e-9);
        // In the other order the first is still through the material.
        let (_, swapped) = planes_of(top, right, &block, PARALLEL).unwrap();
        let first = swapped.first().unwrap();
        assert!((Vec3::new(55.0, 3.0, 15.0) - first.0).dot(first.1).abs() < 1e-9);
        // The top and a 45 degree chamfer of its edge at x = 60 (a convex
        // edge of 135 degrees), the chamfered block by its corners: the
        // outer bisector is the longer, but touches the block only along
        // the edge, so the one through the material comes first, alone.
        let slant = (
            Vec3::new(56.0, 0.0, 16.0),
            Vec3::new(1.0, 0.0, 1.0).normalized().unwrap(),
        );
        let corners: Vec<Vec3> = [0.0, 40.0]
            .into_iter()
            .flat_map(|y| {
                [
                    Vec3::new(0.0, y, 0.0),
                    Vec3::new(60.0, y, 0.0),
                    Vec3::new(0.0, y, 20.0),
                    Vec3::new(52.0, y, 20.0),
                    Vec3::new(60.0, y, 12.0),
                ]
            })
            .collect();
        let chamfered = |point: Vec3, normal: Vec3| separates(&corners, point, normal, 1e-7);
        let (_, planes) = planes_of(top, slant, &chamfered, PARALLEL).unwrap();
        let through = (Vec3::Z - slant.1).normalized().unwrap();
        assert_eq!(planes.len(), 1, "{planes:?}");
        assert!(near(planes[0].1, through), "{planes:?}");

        // Two parallel faces, facing apart (the block's left and right)
        // or the same way: the plane midway, along the first's normal.
        let left = (Vec3::new(0.0, 5.0, 5.0), Vec3::ZERO - Vec3::X);
        let (made, planes) = planes_of(right, left, &block, PARALLEL).unwrap();
        assert_eq!(made, SectionMade::Mid);
        let [(p, n)] = planes.as_slice() else {
            panic!("one plane: {planes:?}");
        };
        assert!(
            near(*n, Vec3::X) && (p.x - 30.0).abs() < 1e-9,
            "{p:?} {n:?}"
        );
        let ledge = (Vec3::new(20.0, 0.0, 0.0), Vec3::X);
        let (made, planes) = planes_of(ledge, right, &anywhere, PARALLEL).unwrap();
        assert_eq!(made, SectionMade::Mid);
        assert_eq!(planes.len(), 1);
        assert!((planes[0].0.x - 40.0).abs() < 1e-9);
        // At every scale.
        for k in [1e-3, 1.0, 1e6] {
            let (made, planes) = planes_of(
                (Vec3::new(60.0 * k, 0.0, 0.0), Vec3::X),
                (Vec3::new(0.0, 0.0, 20.0 * k), Vec3::Z),
                &in_box(Vec3::ZERO, Vec3::new(60.0 * k, 40.0 * k, 20.0 * k)),
                PARALLEL,
            )
            .unwrap();
            assert_eq!(made, SectionMade::Bisector);
            let on_edge = Vec3::new(60.0 * k, 1.0, 20.0 * k);
            for (point, normal) in planes {
                assert!((on_edge - point).dot(normal).abs() < 1e-9 * k.max(1.0));
            }
        }

        // Faces all but parallel (the review's, 2026-11-17): the +x ends
        // of two blocks, at x = 60 and x = 120, the second turned about z
        // by a twenty-millionth of a degree up to a tenth, the same way
        // and apart: the plane between them, by x = 90 or 60, cutting the
        // model's box; called midway only within the angular tolerance.
        let model = in_box(Vec3::ZERO, Vec3::new(120.0, 40.0, 20.0));
        for degrees in [5e-8_f64, 1e-7, 3e-7, 1e-3, 0.1] {
            let t = degrees.to_radians();
            let turned = Vec3::new(t.pcos(), t.psin(), 0.0);
            for (first, middle) in [
                ((Vec3::new(60.0, 0.0, 0.0), Vec3::X), 90.0),
                ((Vec3::new(0.0, 0.0, 0.0), Vec3::ZERO - Vec3::X), 60.0),
            ] {
                let (made, planes) = planes_of(
                    first,
                    (Vec3::new(120.0, 10.0, 10.0), turned),
                    &model,
                    PARALLEL,
                )
                .unwrap();
                let parallel = t <= PARALLEL;
                assert_eq!(made == SectionMade::Mid, parallel, "{degrees} {first:?}");
                let (point, normal) = planes[0];
                assert!((point.x - middle).abs() < 0.2, "{degrees}: {point:?}");
                assert!(normal.x.abs() > 0.99, "{normal:?}");
                assert!(point.y.abs() < 100.0 && point.z.abs() < 100.0, "{point:?}");
                assert!(model(point, normal), "{degrees}: it cuts the model");
                // The other bisector meets far out: not offered.
                assert_eq!(planes.len(), 1, "{degrees} {first:?}: {planes:?}");
            }
        }

        // The review's tray (round three): opposite sides x + t z = 50 and
        // -x + t z = 50, outward normals (±1, 0, t), the box from
        // (-50, -50, 0) to (50, 50, 20): whatever the slope, the plane
        // halving them is x = 0 (a rule by distance gave a plane tilted 14
        // degrees for sides 28 degrees apart, and flipped at 36.2).
        let (lo, hi) = (Vec3::new(-50.0, -50.0, 0.0), Vec3::new(50.0, 50.0, 20.0));
        let tray = in_box(lo, hi);
        for t in [
            0.0_f64, 1e-12, 0.05, 0.1, 0.2, 0.25, 0.2638, 0.3, 0.35, 0.5, 1.0,
        ] {
            let a = (
                Vec3::new(50.0, 0.0, 0.0),
                Vec3::new(1.0, 0.0, t).normalized().unwrap(),
            );
            let b = (
                Vec3::new(-50.0, 0.0, 0.0),
                Vec3::new(-1.0, 0.0, t).normalized().unwrap(),
            );
            let (made, planes) = planes_of(a, b, &tray, PARALLEL).unwrap();
            let (point, normal) = planes[0];
            assert!(near(normal, Vec3::X), "{t}: {normal:?}");
            assert!(point.x.abs() < 1e-9, "{t}: {point:?}");
            // Midway only within the angular tolerance: the sine of the
            // angle between the normals, 2 t / (1 + t²).
            let sine = 2.0 * t / (1.0 + t * t);
            assert_eq!(made == SectionMade::Mid, sine <= PARALLEL, "{t}");
        }

        // Continuity: two faces turned apart from parallel by 30 degrees
        // down to none, in a thousand steps, facing apart and the same
        // way: the first plane's normal turns by no more than half the
        // step, and its point moves by no more than a little, every step.
        let block = in_box(Vec3::new(-10.0, -60.0, -60.0), Vec3::new(70.0, 60.0, 60.0));
        for (start, sign) in [
            (Vec3::new(0.0, 0.0, 0.0), -1.0),
            (Vec3::new(20.0, 0.0, 0.0), 1.0),
        ] {
            let mut last: Option<(Vec3, Vec3)> = None;
            let steps = 1000;
            for k in (0..=steps).rev() {
                let t = (30.0_f64 * f64::from(k) / f64::from(steps)).to_radians();
                let m = Vec3::new(t.pcos(), t.psin(), 0.0);
                let (_, planes) = planes_of(
                    (start, Vec3::X * sign),
                    (Vec3::new(60.0, 5.0, 0.0), m),
                    &block,
                    PARALLEL,
                )
                .unwrap();
                let (point, normal) = planes[0];
                if let Some((p0, n0)) = last {
                    let turned = n0.dot(normal).clamp(-1.0, 1.0).pacos();
                    let step = (30.0_f64 / f64::from(steps)).to_radians();
                    assert!(turned <= 0.5 * step + 1e-9, "{k}: turned {turned}");
                    assert!(point.distance(p0) < 1.0, "{k}: {p0:?} to {point:?}");
                }
                last = Some((point, normal));
            }
            // At none, the plane midway.
            let (point, normal) = last.unwrap();
            assert!(near(normal, Vec3::X) || near(normal, Vec3::ZERO - Vec3::X));
            assert!((point.x - (start.x + 60.0) * 0.5).abs() < 1e-9, "{point:?}");
        }

        // A real angle whose planes meet just outside the box (a frustum's
        // two sides, meeting 15 above its top) keeps its bisector.
        let side = Vec3::new(1.0, 0.0, 1.0).normalized().unwrap();
        let other = Vec3::new(-1.0, 0.0, 1.0).normalized().unwrap();
        let (made, planes) = planes_of(
            (Vec3::new(55.0, 0.0, 10.0), side),
            (Vec3::new(5.0, 0.0, 10.0), other),
            &in_box(Vec3::ZERO, Vec3::new(60.0, 40.0, 20.0)),
            PARALLEL,
        )
        .unwrap();
        assert_eq!(made, SectionMade::Bisector);
        assert!(near(planes[0].1, Vec3::X), "{planes:?}");
        // Numbers that are not numbers: none.
        assert_eq!(
            planes_of(
                (Vec3::new(f64::NAN, 0.0, 0.0), Vec3::X),
                top,
                &anywhere,
                PARALLEL
            ),
            None
        );
        assert_eq!(
            planes_of(right, (Vec3::ZERO, Vec3::ZERO), &anywhere, PARALLEL),
            None
        );
    }

    #[test]
    fn the_fourth_button_lays_the_plane_and_a_second_click_takes_the_other_bisector() {
        let face = |made, planes: Vec<([f64; 3], [f64; 3])>| PlaneChoice {
            made: Some(made),
            faces: vec![("F1".to_string(), "F1:face(1)".to_string())],
            planes,
            refused: None,
        };
        let mut section = SectionState {
            on: true,
            plane: Some("xy".to_string()),
            offset: 7.0,
            flip: true,
            ..SectionState::default()
        };
        // Along one face: its name, offset 0, not flipped; again: kept.
        choose_plane(&mut section, &face(SectionMade::Face, Vec::new()));
        assert_eq!(section.plane.as_deref(), Some("F1:face(1)"));
        assert_eq!(section.feature.as_deref(), Some("F1"));
        assert_eq!(section.made, Some(SectionMade::Face));
        assert_eq!((section.offset, section.flip), (0.0, false));
        section.offset = -3.0;
        choose_plane(&mut section, &face(SectionMade::Face, Vec::new()));
        assert_eq!(section.offset, -3.0);
        // Midway: point and normal, no name.
        let mid = ([30.0, 0.0, 0.0], [1.0, 0.0, 0.0]);
        choose_plane(&mut section, &face(SectionMade::Mid, vec![mid]));
        assert_eq!(
            (section.plane.clone(), section.point, section.normal),
            (None, Some(mid.0), Some(mid.1))
        );
        assert_eq!(
            (section.made, section.offset),
            (Some(SectionMade::Mid), 0.0)
        );
        // Bisectors: the first, then at each click the other.
        let through = ([1.0, 2.0, 3.0], [0.6, 0.0, -0.8]);
        let other = ([1.0, 2.0, 3.0], [0.8, 0.0, 0.6]);
        let bisectors = face(SectionMade::Bisector, vec![through, other]);
        assert_eq!(
            bisectors.words(&section),
            "Halving the angle of the two faces"
        );
        choose_plane(&mut section, &bisectors);
        assert_eq!(section.normal, Some(through.1));
        assert_eq!(
            bisectors.words(&section),
            "The other bisector of the two faces, square to this one"
        );
        choose_plane(&mut section, &bisectors);
        assert_eq!(section.normal, Some(other.1));
        assert_eq!(section.made, Some(SectionMade::Bisector));
        choose_plane(&mut section, &bisectors);
        assert_eq!(section.normal, Some(through.1));
        // A selection that gives no plane changes nothing, and says why.
        let before = section.clone();
        let none = PlaneChoice::refused(Vec::new(), TAKES_FACES);
        choose_plane(&mut section, &none);
        assert_eq!(section, before);
        assert_eq!(none.words(&section), TAKES_FACES);
        // The state keeps `made` only with a plane it fits.
        let mut odd = section.clone();
        odd.plane = Some("xy".to_string());
        odd.point = None;
        odd.normal = None;
        assert_eq!(odd.validated().unwrap().made, None);
        assert_eq!(
            section.clone().validated().unwrap().made,
            Some(SectionMade::Bisector)
        );
    }
}
