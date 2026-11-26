//! Where an approximated edge meets a surface at a shallow angle.
//!
//! An edge on a spline that stands for the curve in which two surfaces
//! meet (made by an earlier boolean) is only within the tolerance of that
//! curve. Where it runs onto a third surface at a shallow angle, the point
//! where the spline itself meets that surface ([`super::hits`]) slides
//! along it: a spline a fraction `e` of the tolerance off its surfaces,
//! meeting the third surface at an angle `a`, meets it up to `e / sin a`
//! from where the three surfaces meet. The third surface meets the edge's
//! own faces in curves of their own that go through where the three meet,
//! not through that point: where two of the surfaces are nearly tangent (a
//! tube in a tube, their walls crossing at a few degrees), such a curve
//! passes several tolerances from it, the edge is not cut on it, and a
//! piece of the curve runs on across a hole of the face
//! (`kernel/tests/boolean_tube_clusters.rs`).
//!
//! So such a hit is taken to the point where the three surfaces meet
//! ([`refine`]), found by Newton's method from the hit, when that point
//! lies on all three surfaces (measured, not only by the Newton step), is
//! within the tolerance of the edge, and is on the edge strictly between
//! the hits of the edge on the surface on either side of this one (two
//! hits never become one place, nor change order). How far it may be
//! from the hit is bounded by the slide the angle explains (the hit's
//! distance from the edge's own surfaces over the sine of the angle at
//! which the edge meets the surface), up to [`CAP`] tolerances: a hit
//! that would have to slide further, and is not within [`MOVE`]
//! tolerances of where it stands for, is refused as too shallow an angle
//! ([`SHALLOW`]). Every face pair that asks where the edge meets the
//! surface asks it of the same edge and surface, and gets the same point.
//!
//! Where a spline edge only touches the surface (its distance from it
//! has a least value within the tolerance and does not change sign), the
//! three surfaces meet in no point of their own, and the hit is where the
//! spline comes nearest the surface. Where the surface is tangent to one of
//! the edge's own faces along a curve that the edge crosses (the round face
//! of a fillet's tool against the wall it rounds, along the circle where
//! the round begins, and the wall's border a spline that crosses that
//! circle), the curve the spline stands for reaches the surface exactly
//! where it crosses that circle, and that place is a vertex: the circle
//! leaves the wall there, and the curve in which the edge's other face
//! meets the round face ends there. But near it the distance grows only
//! with the square of the way along the edge, so the spline, a fit within
//! the tolerance of its curve, comes nearest the surface wherever its fit
//! puts it in that valley: in random design 103578, 31 and 6.6 tolerances
//! along the border from the crossing, 7.2 and 1.5 tolerances off the
//! circle. Such a touch is taken to where the curve the spline stands for
//! (its places taken to the nearest point of both its faces' surfaces)
//! comes nearest the surface: where the slope of its distance from it (the
//! surface's normal at the foot against the curve's direction) changes
//! sign, found to rounding by halving ([`touch`]), when the curve reaches
//! the surface there (within a thousandth of the tolerance: a meeting, not
//! a curve passing within the tolerance of a surface it misses), and more
//! than [`MOVE`] tolerances from the hit; and the section takes it where it
//! lies on the boundary of the face it touches (where the edge crosses
//! that face's edge: a vertex of the result), and inside that face only
//! where the result keeps both that face and the edge's face tangent to it
//! ([`kept_both`]). Inside the face, a touch whose curve the result keeps
//! one face of, or neither, divides nothing, and where it lies along the
//! edge changes nothing the result needs: there the hit stands. (Moved
//! there too, the touch made the circle along which a plane is tangent to
//! a torus a stretch of its own, which the torus's chart then refused,
//! *"a face on a torus whose boundary crosses every circle around its axis
//! and every circle around its tube"*: random designs 20791237 and
//! 20812686, built on `main`, where the torus lies wholly on the side
//! taken away.) Where the result keeps both, the curve along which they
//! touch is an edge of the result, the place ends it, and with the hit
//! standing the curve went uncut and the faces undivided, touching inside
//! both: random design 20781906, three parts that touch along two arcs of
//! a circle built as one shell (docs/design/booleans.md › *A pinch along a
//! curve*).

use super::Operand;
use super::Reason;
use super::outline::{
    Outline, Sheet, boxes_meet, locate, nearest, onto, outward_normal, surface_distance,
};
use super::section::{Kept, Solid};
use craft3d_geom::Curve3;
use craft3d_math::{Frame, Vec3};
use craft3d_topo::EdgeId;

/// How far a hit must be from where the three surfaces meet, in
/// tolerances, to be moved there. Closer than that it is merged with any
/// other place found there (half the tolerance) with room to spare, and is
/// left as it was found.
const MOVE: f64 = 0.25;

/// How far, in tolerances, the point where the three surfaces meet may
/// always be from the hit; further only as far as the angle explains
/// (four times the slide it allows), never further than [`CAP`].
const REACH: f64 = 256.0;

/// The furthest, in tolerances, a hit is ever moved: a slide longer than
/// this is refused ([`SHALLOW`]).
pub(super) const CAP: f64 = 65536.0;

/// The surfaces' unit normals must span at least this volume (the
/// determinant of the three, a measure free of scale) for the point
/// where they meet to be told.
const SPAN: f64 = 1e-10;

/// The most Newton steps.
const STEPS: usize = 32;

/// The most looks either way from a touch for where the slope of its
/// distance changes sign ([`touch`]): from [`MOVE`] tolerances, doubling,
/// to [`CAP`], 2^18 times as far, the 19th look being the last within it.
const LOOKS: usize = 19;

pub(super) const SHALLOW: Reason = "an edge on a spline meets a surface at an angle so shallow that where it meets it cannot be placed within the tolerance (the surfaces it lies between are nearly tangent to that surface there)";

/// The point near `start`, no further than `reach`, where the three
/// surfaces meet: Newton's method on the planes tangent to each surface
/// at the point of it nearest to the current guess, until the guess is
/// within a millionth of the tolerance of the three tangent planes or a
/// step that small, and accepted only when the point is measured within
/// a quarter of the tolerance of each surface. `None` when the surfaces' normals there are too close to
/// dependent to tell the point, or the steps do not settle.
pub(super) fn meet(sheets: [Sheet; 3], start: Vec3, reach: f64, tol: f64) -> Option<Vec3> {
    // On all three surfaces, the point is where it stays: measured, since
    // a small step only says the tangent planes go through it (a nearest
    // point clamped at a border, or a local one, is no foot of the point).
    let on = |p: Vec3| {
        sheets
            .iter()
            .all(|&sheet| surface_distance(sheet, p) <= 0.25 * tol)
    };
    let mut p = start;
    for _ in 0..STEPS {
        let mut normals = [Vec3::ZERO; 3];
        let mut offsets = [0.0; 3];
        for ((sheet, normal), offset) in sheets.iter().zip(&mut normals).zip(&mut offsets) {
            let (u, v) = sheet.project(p)?;
            let q = sheet.surface.eval(u, v);
            let n = sheet.surface.normal(u, v)?.normalized()?;
            *normal = n;
            *offset = (q - p).dot(n);
        }
        // Within a millionth of the tolerance of the three tangent planes:
        // what is left is rounding, which a step only magnifies where the
        // surfaces are close to tangent.
        if offsets.iter().all(|o| o.abs() <= 1e-6 * tol) {
            return on(p).then_some(p);
        }
        let [n0, n1, n2] = normals;
        let [b0, b1, b2] = offsets;
        let det = n0.dot(n1.cross(n2));
        if !(det.is_finite() && det.abs() > SPAN) {
            return None;
        }
        let step = (n1.cross(n2) * b0 + n2.cross(n0) * b1 + n0.cross(n1) * b2) * (1.0 / det);
        p += step;
        if !p.is_finite() || p.distance(start) > reach {
            return None;
        }
        if step.length() <= 1e-6 * tol {
            return on(p).then_some(p);
        }
    }
    None
}

/// Where edge `id` of `own`, on a spline, meets `surface`, given `hit`,
/// the point where its spline does, at parameter `at` of the edge
/// (`touch` when it only touches the surface there: such a hit stands),
/// and
/// `between`, the parameters of the hits of the edge on the surface on
/// either side of it (or the ends of the edge): the point where the
/// surfaces of the edge's two faces and `surface` meet, when the hit is
/// more than [`MOVE`] tolerances from it, and that point is within the
/// tolerance of the edge strictly between those parameters; `Ok(None)`
/// when the hit stands; [`SHALLOW`] when it would have to slide further
/// than [`CAP`] tolerances.
pub(super) fn refine(
    own: &Solid,
    id: EdgeId,
    surface: Sheet,
    (hit, at, touch): (Vec3, f64, bool),
    between: (f64, f64),
    tol: f64,
) -> Result<Option<Vec3>, Reason> {
    // Where the edge only touches the surface, the three surfaces meet
    // in no point of their own: they are tangent there, every point of a
    // band along the edge is as near all three, and Newton creeps along
    // it to wherever it stops (48.0 or 48.7 tolerances from the hit by
    // two stopping rules, at a rolling fillet's end on a cone,
    // `fillet_contact.rs`). A spline's touch is placed by [`touch`].
    if touch {
        return Ok(None);
    }
    let Some(edge) = own.body.edge(id) else {
        return Ok(None);
    };
    let Some([a, b]) = sides(own, id, tol) else {
        return Ok(None);
    };
    // How far the hit can have slid: its distance from the edge's own
    // surfaces over the sine of the angle at which the edge meets the
    // surface.
    let off = surface_distance(a, hit).max(surface_distance(b, hit));
    let slide = match slide(&edge.curve, at, surface, off) {
        Some(slide) => slide,
        None => return Ok(None),
    };
    let reach = (REACH * tol).max(4.0 * slide);
    if reach > CAP * tol {
        return Err(SHALLOW);
    }
    let Some(point) = meet([a, b, surface], hit, reach, tol) else {
        return Ok(None);
    };
    if point.distance(hit) <= MOVE * tol {
        return Ok(None);
    }
    let Some(t) = locate(&edge.curve, edge.range, point, tol) else {
        return Ok(None);
    };
    let (lo, hi) = (between.0.min(between.1), between.0.max(between.1));
    if !(t > lo && t < hi) {
        return Ok(None);
    }
    Ok(Some(point))
}

/// The two faces of `own` on either side of edge `id`, in the order of
/// the body's faces; `None` where there are not two.
fn faces_of(own: &Solid, id: EdgeId, tol: f64) -> Option<[usize; 2]> {
    let edge_box = own.edge_box(id);
    let mut faces = Vec::with_capacity(2);
    for face in own.candidates(|b| boxes_meet(b, &edge_box, tol)) {
        let f = own.body.faces.get(face)?;
        let borders = f
            .loops
            .iter()
            .filter_map(|&l| own.body.loop_(l))
            .any(|l| l.coedges.iter().any(|c| c.edge == id));
        if borders {
            faces.push(face);
        }
    }
    let [first, second] = faces.as_slice() else {
        return None;
    };
    Some([*first, *second])
}

/// The surfaces of the faces on either side of edge `id` of `own`.
fn sides<'s>(own: &'s Solid, id: EdgeId, tol: f64) -> Option<[Sheet<'s>; 2]> {
    let [first, second] = faces_of(own, id, tol)?;
    let (Ok(a), Ok(b)) = (own.outline(first), own.outline(second)) else {
        return None;
    };
    Some([a.sheet(), b.sheet()])
}

/// How far apart (the sine of the angle) two surfaces must cross for the
/// curve in which they meet to be followed ([`onto_both`]).
const CROSS: f64 = 1e-3;

/// The point of both surfaces nearest `start` (Newton's method on their
/// tangent planes, the least step that reaches both) and the direction
/// of the curve in which they meet there, when they cross at more than
/// [`CROSS`] and the steps settle within a millionth of the tolerance,
/// within [`CAP`] tolerances of `start`.
fn onto_both([a, b]: [Sheet; 2], start: Vec3, tol: f64) -> Option<(Vec3, Vec3)> {
    let mut p = start;
    for _ in 0..STEPS {
        let mut normals = [Vec3::ZERO; 2];
        let mut offsets = [0.0; 2];
        for ((sheet, normal), offset) in [a, b].iter().zip(&mut normals).zip(&mut offsets) {
            let (u, v) = sheet.project(p)?;
            let q = sheet.surface.eval(u, v);
            let n = sheet.surface.normal(u, v)?.normalized()?;
            *normal = n;
            *offset = (q - p).dot(n);
        }
        let [n0, n1] = normals;
        let along = n0.cross(n1);
        let sine = along.length();
        if sine.is_nan() || sine <= CROSS {
            return None;
        }
        if offsets.iter().all(|o| o.abs() <= 1e-6 * tol) {
            return Some((p, along.normalized()?));
        }
        // The least step to both tangent planes lies along the two
        // normals: x n0 + y n1, with x + c y = b0 and c x + y = b1.
        let c = n0.dot(n1);
        let det = 1.0 - c * c;
        let [b0, b1] = offsets;
        p += n0 * ((b0 - c * b1) / det) + n1 * ((b1 - c * b0) / det);
        if !p.is_finite() || p.distance(start) > CAP * tol {
            return None;
        }
    }
    None
}

/// How near the surface, as a share of the tolerance, the curve a spline
/// edge stands for must come where it is nearest for a touch to be moved
/// there ([`touch`]): it reaches the surface.
const REACHES: f64 = 1e-3;

/// Where spline edge `id` of `own` only touches `surface`, given `hit`,
/// where the spline comes nearest it, at parameter `at` of the edge, and
/// `between`, the parameters of the edge's hits on the surface on either
/// side of it (or its ends): the place where the curve the edge stands for
/// comes nearest the surface (see the module's text), when it reaches the
/// surface there (within [`REACHES`] of the tolerance) and that place is
/// more than [`MOVE`] tolerances from the hit, within the tolerance of the
/// edge strictly between those parameters and within a quarter of it of
/// its faces' surfaces; `None` when the hit stands.
///
/// The slope changes sign where the distance is least, and where it is
/// most. The search looks at the slope on either side of the hit, from
/// [`MOVE`] tolerances out (a change within them leaves the hit where it
/// is, after three looks), doubling, up to [`CAP`] tolerances and the hits
/// on either side, at most [`LOOKS`] times; the first look that finds the
/// sign changed against the hit's brackets a change, which halving then
/// finds. That is the change nearest the hit among those the looks tell
/// apart, not always the nearest there is: two changes between one look
/// and the next leave the sign as it was and are not seen, and a farther
/// one may be taken. Whatever is found is taken only on the conditions
/// above (it reaches the surface, lies on the edge between its
/// neighbouring hits, on its faces' surfaces, away from the hit), and the
/// section takes it on the touched face's border, and inside that face
/// where the result keeps it and the edge's face tangent to it
/// ([`kept_both`]).
pub(super) fn touch(
    own: &Solid,
    id: EdgeId,
    surface: Sheet,
    (hit, at): (Vec3, f64),
    between: (f64, f64),
    tol: f64,
) -> Option<Vec3> {
    let edge = own.body.edge(id)?;
    if !matches!(edge.curve, Curve3::Nurbs(_)) {
        return None;
    }
    let both = sides(own, id, tol)?;
    // The place of the curve the edge stands for at parameter `s` of the
    // edge, and its direction there (the edge's way).
    let place = |s: f64| -> Option<(Vec3, Vec3)> {
        let (p, d) = edge.curve.derivatives(s);
        let (q, along) = onto_both(both, p, tol)?;
        Some((q, if along.dot(d) < 0.0 { -along } else { along }))
    };
    // The slope of the distance from the surface along the curve: the
    // normal at the foot, pointing to the place, against the direction.
    let slope = |s: f64| -> Option<f64> {
        let (p, d) = place(s)?;
        let (u, v) = surface.project(p)?;
        let n = surface.surface.normal(u, v)?.normalized()?;
        let n = if (p - surface.surface.eval(u, v)).dot(n) < 0.0 {
            -n
        } else {
            n
        };
        let value = n.dot(d);
        value.is_finite().then_some(value)
    };
    let (lo, hi) = (between.0.min(between.1), between.0.max(between.1));
    let speed = edge.curve.derivatives(at).1.length();
    if !(speed.is_finite() && speed > 0.0 && lo < at && at < hi) {
        return None;
    }
    let here = slope(at)? > 0.0;
    let mut step = MOVE * tol / speed;
    let mut bracket = None;
    // Bounded by the count as well as by the cap. Doubling is exact, so
    // `step * speed` is `MOVE * tol` times a power of two to rounding, and
    // stays within `CAP * tol` for the first 19 looks at most ([`LOOKS`]):
    // the count never stops a search the cap would let go on, and changes
    // no answer. It bounds only a step that has underflowed to zero (or below
    // the normal numbers, where doubling no longer scales the product
    // exactly: a tolerance under about 1e-308 of the edge's parametric
    // speed, which the engine never makes), which would otherwise never
    // grow.
    for _ in 0..LOOKS {
        if bracket.is_some() || step * speed > CAP * tol {
            break;
        }
        for s in [at - step, at + step] {
            if s > lo && s < hi && (slope(s)? > 0.0) != here {
                bracket = Some(if s < at { (s, at) } else { (at, s) });
                break;
            }
        }
        if bracket.is_some() && step * speed <= MOVE * tol {
            // Where it is nearest is within a quarter of the tolerance of
            // the hit.
            return None;
        }
        step *= 2.0;
    }
    let (mut a, mut b) = bracket?;
    let rising = slope(b)? > 0.0;
    for _ in 0..STEPS * 2 {
        let middle = 0.5 * (a + b);
        if !(middle > a && middle < b) {
            break;
        }
        if (slope(middle)? > 0.0) == rising {
            b = middle;
        } else {
            a = middle;
        }
    }
    let (point, _) = place(0.5 * (a + b))?;
    if surface_distance(surface, point) > REACHES * tol
        || both
            .iter()
            .any(|&sheet| surface_distance(sheet, point) > 0.25 * tol)
    {
        return None;
    }
    let t = locate(&edge.curve, edge.range, point, tol)?;
    if !(t > lo && t < hi) || point.distance(hit) <= MOVE * tol {
        return None;
    }
    Some(point)
}

/// How near parallel (the sine of the angle between them) the normals of
/// an edge's face and of the surface the edge touches must be at the touch
/// for the face to be read as tangent to that surface there
/// ([`kept_both`]). The place [`touch`] finds is where the curve the edge
/// stands for reaches the surface, so a face tangent along a curve the
/// edge crosses is tangent there to rounding; a face that crosses the
/// surface is turned away by the walks of [`side_of`] as well.
const TANGENT: f64 = 1e-3;

/// The most steps of one walk of [`side_of`].
const WALK: usize = 160;

/// Whether, at `p`, where spline edge `id` of `own` only touches the
/// surface of `against` (a face of the other operand, `p` inside it), one
/// of the edge's two faces is tangent to that surface and keeps to one
/// side of it about `p` ([`side_of`]), and the boolean keeps both that face
/// and `against` there (`kept`, by which side of the other operand each
/// lies on). The two surfaces then touch along a curve that the edge
/// crosses at `p`, and the result has both faces on both sides of that
/// curve: its material between them on both sides (two walls that thin to
/// nothing at the curve from either side), or beyond them on both (two
/// parts that rest on each other along it). The curve is an edge of the
/// result where four faces meet, and `p` is where it ends on the edge: the
/// place has to be a vertex, or the curve is not cut there, goes uncut as
/// a touch that runs over a face's boundary, and the faces stay undivided,
/// touching along it inside both, where nothing looks for zero thickness
/// (random design 20781906: three parts that touch along two arcs of a
/// circle, built as one shell). Where the result keeps one of the two
/// faces or neither, the curve divides nothing.
pub(super) fn kept_both(
    own: &Solid,
    id: EdgeId,
    against: &Outline,
    p: Vec3,
    kept: Kept,
    tol: f64,
) -> bool {
    let Some(faces) = faces_of(own, id, tol) else {
        return false;
    };
    faces.into_iter().any(|face| {
        own.outline(face).is_ok_and(|outline| {
            keeps_both(own.operand, outline, against, p, kept, tol) == Some(true)
        })
    })
}

/// Whether the boolean keeps both `face`, of `operand`, and `against`, of
/// the other operand, about `p`, where the two are tangent and `face`
/// keeps to one side of `against` ([`side_of`]): `kept` says which faces
/// of each operand the boolean keeps, by the side of the other operand
/// they lie on. `None` where they are not tangent at `p` (their normals
/// further apart than [`TANGENT`]), or where the side cannot be read (the
/// surfaces cross there, or part too slowly for the walks).
///
/// Two faces that touch along a curve are both in the result on both sides
/// of it, or one of them is, or neither: the one is on the same side of
/// the other operand on both sides of the curve. Only where both are kept
/// is the curve an edge of the result ([`kept_both`]); where one is kept or
/// neither, it divides nothing.
pub(super) fn keeps_both(
    operand: Operand,
    face: &Outline,
    against: &Outline,
    p: Vec3,
    kept: Kept,
    tol: f64,
) -> Option<bool> {
    let other = against.sheet();
    let n_other = outward_normal(other, against.reversed, p).and_then(Vec3::normalized)?;
    let sheet = face.sheet();
    let n_own = outward_normal(sheet, face.reversed, p).and_then(Vec3::normalized)?;
    let sine = n_own.cross(n_other).length();
    if sine.is_nan() || sine > TANGENT {
        return None;
    }
    // The face beyond `against` (on its outward side) is outside the other
    // operand. Then `against` is on the face's inner side, inside
    // `operand`, where their outward normals agree, and outside it where
    // they are opposed; and the other way round.
    let beyond = side_of(sheet, (other, against.reversed), p, n_other, tol)?;
    let against_outside = beyond != (n_own.dot(n_other) > 0.0);
    let other_operand = match operand {
        Operand::A => Operand::B,
        Operand::B => Operand::A,
    };
    Some(kept.outside(operand) == beyond && kept.outside(other_operand) == against_outside)
}

/// Which side of `other` (by its outward normal, `reversed` as its face
/// has it) `surface` keeps to about `p`, where the two are tangent with
/// the normal `normal`: walked away from `p` in eight directions square to
/// `normal`, each until `surface` is twice the tolerance from `other`
/// (steps of a quarter of the way walked, two tolerances at least, up to
/// [`CAP`] tolerances), `Some(true)` where every walk that got that far
/// found it on the outward side, `Some(false)` where every one found it on
/// the inner side; `None` where none got that far, or where they disagree
/// (the surfaces cross there).
fn side_of(
    surface: Sheet,
    (other, reversed): (Sheet, bool),
    p: Vec3,
    normal: Vec3,
    tol: f64,
) -> Option<bool> {
    let frame = Frame::from_z(p, normal)?;
    let (x, y) = (frame.x(), frame.y());
    let diagonal = std::f64::consts::FRAC_1_SQRT_2;
    let directions = [
        x,
        (x + y) * diagonal,
        y,
        (y - x) * diagonal,
        -x,
        -(x + y) * diagonal,
        -y,
        (x - y) * diagonal,
    ];
    let mut found = None;
    for direction in directions {
        let mut s = 0.0;
        for _ in 0..WALK {
            s += (2.0 * tol).max(0.25 * s);
            if s > CAP * tol {
                break;
            }
            let Some(q) = onto(surface, p + direction * s) else {
                break;
            };
            let Some(foot) = onto(other, q) else {
                break;
            };
            let Some(outward) = outward_normal(other, reversed, foot).and_then(Vec3::normalized)
            else {
                break;
            };
            let d = (q - foot).dot(outward);
            if !d.is_finite() {
                break;
            }
            if d.abs() >= 2.0 * tol {
                let side = d > 0.0;
                if found.is_some_and(|known| known != side) {
                    return None;
                }
                found = Some(side);
                break;
            }
        }
    }
    found
}

/// How far from parallel (the sine of the angle between them) two curves
/// on one surface must cross for [`border_crossings`] to take the place
/// where they do: nearer parallel, a rounding of their distance moves that
/// place along them by more than the tolerance.
const ACROSS: f64 = 1e-2;

/// How many pieces a spline edge is looked at in for where it comes
/// nearest a border of the other face ([`border_crossings`]).
const LOOKS_ALONG: usize = 64;

/// The unit normal of `sheet` at the point of it nearest `p`.
fn normal_at(sheet: Sheet, p: Vec3) -> Option<Vec3> {
    let (u, v) = sheet.project(p)?;
    sheet.surface.normal(u, v)?.normalized()
}

/// Whether the two surfaces are tangent at (the points of them nearest)
/// `p`: their normals within the sine [`TANGENT`] of parallel, either way.
fn tangent_at(a: Sheet, b: Sheet, p: Vec3) -> bool {
    match (normal_at(a, p), normal_at(b, p)) {
        (Some(n), Some(m)) => {
            let sine = n.cross(m).length();
            sine.is_finite() && sine <= TANGENT
        }
        _ => false,
    }
}

/// Where spline edge `id` of `own`, whose two faces are tangent to each
/// other (a blend's tangent edge), crosses an edge of `against` that lies
/// on the surface of one of those faces, where the surface of `against` is
/// tangent to that face (it touches it along that edge): the parameters
/// along the edge and the places, in the edge's order.
///
/// Two such edges cross on the surface both lie on (random design 121704:
/// half a torus whose end rim is filleted, less a copy moved along its
/// axis; the two end caps lie in one plane, each bounded by its fillet's
/// tangent edge, and those cross twice). The place is a vertex of the
/// result: one edge leaves the other's face there, and the curve in which
/// the two fillets meet ends there. The edge's own hit on the other face's
/// surface stands for it badly or not at all: the edge only touches that
/// surface (it is tangent to the edge's face along the border), and the
/// surface ends at the border, so the edge's distance from it grows with
/// the square of the way along it on one side and the way itself on the
/// other, and its sign there, beside the surface's rim, is noise
/// ([`super::hits`]). In 121704 one crossing had its hit within 0.06 of the
/// tolerance; the other had none, so the curve along which the plane
/// touches the copy's fillet had no cut there, and the cap was not divided
/// along it. [`touch`] cannot follow the curve such an edge stands for: its
/// two faces do not cross. Here the place is where the two curves cross,
/// which they do at an angle: each piece of the border found within the
/// tolerance of a face of the edge at its ends, its quarters and its
/// middle; the edge looked at in [`LOOKS_ALONG`] pieces for where it comes
/// nearest the border, and each such place taken to where the two curves
/// come nearest each other (Gauss–Newton on both parameters). A place is
/// kept where the curves are within the tolerance of each other there and
/// cross at more than the sine [`ACROSS`], it is clear of the edge's ends
/// by twice the tolerance, the edge's two faces are tangent there and
/// `against`'s surface is tangent to the face the border lies on. Empty for
/// every other edge.
pub(super) fn border_crossings(
    own: &Solid,
    id: EdgeId,
    against: &Outline,
    tol: f64,
) -> Vec<(f64, Vec3)> {
    let mut found: Vec<(f64, Vec3)> = Vec::new();
    let Some(edge) = own.body.edge(id) else {
        return found;
    };
    if !matches!(edge.curve, Curve3::Nurbs(_)) {
        return found;
    }
    let (t0, t1) = edge.range;
    if !(t0.is_finite() && t1.is_finite() && t1 > t0) {
        return found;
    }
    // The surfaces of the edge's faces as they are, without the index a
    // face's outline makes for a freeform surface: a face that the section
    // has not outlined is not outlined here (where its index would be made
    // for this look alone).
    let Some([f0, f1]) = faces_of(own, id, tol) else {
        return found;
    };
    let (Some(face0), Some(face1)) = (own.body.faces.get(f0), own.body.faces.get(f1)) else {
        return found;
    };
    let faces = [Sheet::bare(&face0.surface), Sheet::bare(&face1.surface)];
    let [first, second] = faces;
    // A tangent edge, by its middle.
    if !tangent_at(first, second, edge.curve.eval(0.5 * (t0 + t1))) {
        return found;
    }
    let (Some(start), Some(end)) = (own.body.vertex(edge.start), own.body.vertex(edge.end)) else {
        return found;
    };
    let other = against.sheet();
    let step = (t1 - t0) / LOOKS_ALONG as f64;
    let looks: Vec<(f64, Vec3)> = (0..=LOOKS_ALONG)
        .map(|k| {
            let s = if k == LOOKS_ALONG {
                t1
            } else {
                t0 + step * k as f64
            };
            (s, edge.curve.eval(s))
        })
        .collect();
    for seg in against.loops.iter().flatten() {
        // A border on one of the edge's faces.
        let Some(face) = faces.into_iter().find(|&sheet| {
            [0.0, 0.25, 0.5, 0.75, 1.0]
                .into_iter()
                .all(|k| surface_distance(sheet, seg.at(k)) <= tol)
        }) else {
            continue;
        };
        let (lo, hi) = (seg.range.0.min(seg.range.1), seg.range.0.max(seg.range.1));
        if !(lo.is_finite() && hi.is_finite() && hi > lo) {
            continue;
        }
        let near: Vec<Option<(f64, f64)>> = looks
            .iter()
            .map(|&(_, p)| nearest(&seg.curve, (lo, hi), p))
            .collect();
        for (k, &(s_k, p_k)) in looks.iter().enumerate() {
            let Some(Some((t_k, d_k))) = near.get(k).copied() else {
                continue;
            };
            // Nearest among its neighbours (no further than either), and
            // within a piece's length of the border.
            let neighbours = [k.checked_sub(1), k.checked_add(1)];
            let mut reach = 0.0_f64;
            let mut least = true;
            for j in neighbours.into_iter().flatten() {
                let (Some(&(_, p_j)), Some(near_j)) = (looks.get(j), near.get(j)) else {
                    continue;
                };
                reach = reach.max(p_j.distance(p_k));
                if let Some((_, d_j)) = near_j {
                    // Strictly nearer than the one before, so that two
                    // looks equally near give one place.
                    let worse = if j < k { d_k >= *d_j } else { d_k > *d_j };
                    if worse {
                        least = false;
                    }
                }
            }
            if !least || d_k > 2.0 * reach + tol {
                continue;
            }
            let Some((s, p)) =
                curves_cross(&edge.curve, (t0, t1), &seg.curve, (lo, hi), (s_k, t_k), tol)
            else {
                continue;
            };
            let clear = p.distance(start.point) > 2.0 * tol && p.distance(end.point) > 2.0 * tol;
            if !(s > t0 && s < t1 && clear) {
                continue;
            }
            if !(tangent_at(first, second, p) && tangent_at(face, other, p)) {
                continue;
            }
            if found.iter().any(|&(_, q)| q.distance(p) <= 0.5 * tol) {
                continue;
            }
            found.push((s, p));
        }
    }
    found.sort_by(|a, b| a.0.total_cmp(&b.0));
    found
}

/// Whether `surface` is tangent at `p` to a face on either side of edge
/// `id` of `own` (their normals within the sine [`TANGENT`] of parallel):
/// where an edge's hit on `surface` lies within the stretch along which it
/// only touches it, however the sign of its distance read there (beside a
/// freeform surface's rim it is noise, and a touch is read as a crossing).
/// A surface that crosses the edge's face at the hit, at more than that,
/// is not.
pub(super) fn touches_there(own: &Solid, id: EdgeId, surface: Sheet, p: Vec3, tol: f64) -> bool {
    let Some([f0, f1]) = faces_of(own, id, tol) else {
        return false;
    };
    [f0, f1].into_iter().any(|face| {
        own.body
            .faces
            .get(face)
            .is_some_and(|f| tangent_at(Sheet::bare(&f.surface), surface, p))
    })
}

/// Where the curves `a` (over `range_a`) and `b` (over `range_b`) cross,
/// from the parameters `start`: Gauss–Newton on the distance between their
/// points, each parameter held within its range, until a step moves each
/// point by less than a millionth of the tolerance. The parameter along
/// `a` and its point, where the two points are within the tolerance of
/// each other and the curves cross there at more than the sine [`ACROSS`];
/// `None` otherwise, or where the steps do not settle.
fn curves_cross(
    a: &Curve3,
    range_a: (f64, f64),
    b: &Curve3,
    range_b: (f64, f64),
    start: (f64, f64),
    tol: f64,
) -> Option<(f64, Vec3)> {
    let (mut s, mut t) = start;
    for _ in 0..STEPS {
        let (p, dp) = a.derivatives(s);
        let (q, dq) = b.derivatives(t);
        let f = p - q;
        let (aa, ab, bb) = (dp.dot(dp), dp.dot(dq), dq.dot(dq));
        let det = aa * bb - ab * ab;
        if !(det.is_finite() && det > 0.0) {
            return None;
        }
        // The least squares step of [dp, -dq] (ds, dt) = -f.
        let (r1, r2) = (-dp.dot(f), dq.dot(f));
        let ds = (bb * r1 + ab * r2) / det;
        let dt = (ab * r1 + aa * r2) / det;
        if !(ds.is_finite() && dt.is_finite()) {
            return None;
        }
        let next = (
            (s + ds).max(range_a.0).min(range_a.1),
            (t + dt).max(range_b.0).min(range_b.1),
        );
        let moved = ((next.0 - s) * dp.length())
            .abs()
            .max(((next.1 - t) * dq.length()).abs());
        (s, t) = next;
        if moved <= 1e-6 * tol {
            let (p, dp) = a.derivatives(s);
            let (q, dq) = b.derivatives(t);
            let (Some(u), Some(v)) = (dp.normalized(), dq.normalized()) else {
                return None;
            };
            let sine = u.cross(v).length();
            return (p.distance(q) <= tol && sine.is_finite() && sine > ACROSS).then_some((s, p));
        }
    }
    None
}

/// `off` over the sine of the angle at which the curve, at `t`, meets the
/// surface; infinite where it runs along it. `None` without a tangent or
/// a normal there.
fn slide(curve: &Curve3, t: f64, surface: Sheet, off: f64) -> Option<f64> {
    let (p, tangent) = curve.derivatives(t);
    let tangent = tangent.normalized()?;
    let (u, v) = surface.project(p)?;
    let normal = surface.surface.normal(u, v)?.normalized()?;
    let sine = tangent.dot(normal).abs();
    if !off.is_finite() {
        return None;
    }
    Some(if off <= 0.0 {
        0.0
    } else if sine > 0.0 {
        off / sine
    } else {
        f64::INFINITY
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft3d_geom::{Cylinder, Plane, Surface};
    use craft3d_math::Frame;

    fn plane(origin: Vec3, normal: Vec3) -> Surface {
        let hint = if normal.x.abs() < 0.9 {
            Vec3::X
        } else {
            Vec3::Y
        };
        Surface::Plane(Plane::new(Frame::new(origin, normal, hint).unwrap()))
    }

    /// Three planes meet in their point; a cylinder and two planes in the
    /// point on the cylinder; planes too close to dependent, or a point
    /// out of reach, give nothing.
    #[test]
    fn three_surfaces_meet_where_all_three_are() {
        let tol = 1e-6;
        let target = Vec3::new(1.0, 2.0, 3.0);
        let planes = [
            plane(target, Vec3::X),
            plane(target, Vec3::Y),
            plane(target, Vec3::Z),
        ];
        let [a, b, c] = &planes;
        let sheets = [Sheet::bare(a), Sheet::bare(b), Sheet::bare(c)];
        let found = meet(sheets, Vec3::new(1.0, 2.0, 2.999), 1.0, tol).unwrap();
        assert!(found.distance(target) < 1e-12);
        // Out of reach.
        assert!(meet(sheets, Vec3::ZERO, 1.0, tol).is_none());

        // The cylinder of radius 2 about z, the planes x = 1 and z = 5:
        // they meet at (1, √3, 5) near the start.
        let cylinder = Surface::Cylinder(Cylinder::new(Frame::WORLD, 2.0).unwrap());
        let x1 = plane(Vec3::new(1.0, 0.0, 0.0), Vec3::X);
        let z5 = plane(Vec3::new(0.0, 0.0, 5.0), Vec3::Z);
        let sheets = [Sheet::bare(&cylinder), Sheet::bare(&x1), Sheet::bare(&z5)];
        let found = meet(sheets, Vec3::new(1.01, 1.7, 5.0), 1.0, tol).unwrap();
        assert!(found.distance(Vec3::new(1.0, 3.0_f64.sqrt(), 5.0)) < 1e-12);
        // Two planes 1e-11 rad apart and a third across them, from off the
        // point: too close to dependent to tell where they meet.
        // Two planes 1e-11 rad apart and a third across them.
        let tilted = plane(target, Vec3::new(1.0, 1e-11, 0.0));
        let sheets = [Sheet::bare(a), Sheet::bare(&tilted), Sheet::bare(c)];
        assert!(meet(sheets, target + Vec3::new(0.0, 0.5, 0.0), 1.0, tol).is_none());
    }

    /// The review of `curves_cross` (2026-11-26): at every scale from 1e-3
    /// to 1e6, two lines that cross at a sine of a half are crossed where
    /// they do; lines at a sine of 5e-3 (below [`ACROSS`]), parallel lines
    /// half a tolerance apart, and a line that touches a circle half a
    /// tolerance off it give nothing; a line that passes half a tolerance
    /// beyond the end of the other piece is crossed within the tolerance of
    /// that end (by the tolerance the piece's end lies on it); one that
    /// passes two tolerances beyond it gives nothing.
    #[test]
    fn two_curves_cross_only_where_they_cross_at_an_angle() {
        use craft3d_geom::{Circle, Line};
        for k in [1e-3, 1.0, 1e3, 1e6] {
            let tol = 1e-6 * k;
            let line = |origin: Vec3, direction: Vec3| {
                Curve3::Line(Line::new(origin * k, direction).unwrap())
            };
            let x_axis = line(Vec3::ZERO, Vec3::X);
            let range = (-10.0 * k, 10.0 * k);
            // Across at a sine of a half, through (2, 0, 0).
            let steep = line(
                Vec3::new(2.0, 0.0, 0.0),
                Vec3::new(3.0_f64.sqrt(), 1.0, 0.0),
            );
            let (s, p) = curves_cross(&x_axis, range, &steep, range, (k, -k), tol)
                .unwrap_or_else(|| panic!("k={k}: two lines at a sine of a half"));
            assert!((s - 2.0 * k).abs() <= 1e-9 * k, "k={k}: at {s}");
            assert!(p.distance(Vec3::new(2.0, 0.0, 0.0) * k) <= 1e-9 * k);
            // At a sine of 5e-3.
            let shallow = line(Vec3::new(2.0, 0.0, 0.0), Vec3::new(1.0, 5e-3, 0.0));
            assert!(
                curves_cross(&x_axis, range, &shallow, range, (k, -k), tol).is_none(),
                "k={k}: lines at a sine of 5e-3"
            );
            // Parallel, half a tolerance apart.
            let beside = line(Vec3::new(0.0, 0.5e-6, 0.0), Vec3::X);
            assert!(
                curves_cross(&x_axis, range, &beside, range, (k, k), tol).is_none(),
                "k={k}: parallel lines"
            );
            // A circle of radius 5 about (0, 5 + tol / 2) touching the axis.
            let frame = Frame::WORLD
                .at(Vec3::new(0.0, 5.0 + 0.5e-6, 0.0) * k)
                .unwrap();
            let round = Curve3::Circle(Circle::new(frame, 5.0 * k).unwrap());
            let bottom = 1.5 * std::f64::consts::PI;
            assert!(
                curves_cross(
                    &x_axis,
                    range,
                    &round,
                    (bottom - 1.0, bottom + 1.0),
                    (0.1 * k, bottom - 0.01),
                    tol
                )
                .is_none(),
                "k={k}: a circle that touches the line"
            );
            // A piece of the y axis that ends half a tolerance short of the
            // x axis, and one that ends two tolerances short of it.
            let y_axis = line(Vec3::ZERO, Vec3::Y);
            for (short, found) in [(0.5e-6, true), (2e-6, false)] {
                let piece = (short * k, 10.0 * k);
                let crossed = curves_cross(&x_axis, range, &y_axis, piece, (k, k), tol);
                assert_eq!(crossed.is_some(), found, "k={k}: ends {short} short");
                if let Some((_, p)) = crossed {
                    assert!(p.distance(Vec3::ZERO) <= tol, "k={k}: at {p:?}");
                }
            }
        }
    }

    /// The count of a touch's looks is the cap's: doubling from a quarter of
    /// the tolerance, the last look it allows is the last within the cap,
    /// so the count changes nothing the cap allows.
    #[test]
    fn a_touch_looks_as_far_as_the_cap_and_no_further() {
        let mut reach = MOVE;
        for _ in 1..LOOKS {
            reach *= 2.0;
        }
        assert!(
            reach <= CAP,
            "the {LOOKS}th look, {reach}, lies past the cap"
        );
        assert!(
            reach * 2.0 > CAP,
            "a look after the {LOOKS}th, {}, is within the cap",
            reach * 2.0
        );
    }
}
