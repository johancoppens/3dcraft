//! TEMPORARY: a stand-in for `craft3d_geom::intersect::meet_surfaces`,
//! which is being built in another branch. Delete this file when that
//! lands.
//!
//! To swap: in `section.rs` replace
//! `use super::meet::{MeetingCurve, SurfaceMeeting, meet_surfaces};` by
//! `use craft3d_geom::intersect::{MeetingCurve, SurfaceMeeting, meet_surfaces};`,
//! remove `mod meet;` and the `approximate_circles` re-export from
//! `mod.rs`, and delete this file. The types and the function here have
//! exactly the agreed shape; nothing else in the booleans knows which pair
//! of surfaces produced a curve. The tests that call
//! `boolean::approximate_circles` (to see curves that are only
//! approximations before the real ones exist) then go as well, or are
//! pointed at pairs that really meet in such curves.
//!
//! What it knows:
//!
//! - what `intersect_surfaces` knows (planes and cylinders with parallel
//!   axes), with lines cut down to `within`;
//! - any two surfaces of revolution about one axis, by intersecting their
//!   profiles in a half-plane through it: a plane across the axis, a
//!   cylinder, a cone, a torus, and a sphere centred on it. Every plane
//!   and sphere, and every two spheres, are such a pair. They meet in
//!   circles about the axis;
//! - a plane through the apex of a cone (two rulings, one where it is
//!   tangent, or the apex alone) and a plane through the axis of a torus
//!   (two circles around the tube).
//!
//! Everything else is `GeomError::Unsupported`, unless a sphere is
//! plainly clear of the other surface.

use super::hits::signed_distance;
use super::outline::line_in_box;
use craft3d_geom::intersect::{NEARLY_PARALLEL, SurfaceIntersection, intersect_surfaces};
use craft3d_geom::{Circle, Curve3, GeomError, Line, NurbsCurve, Surface};
use craft3d_math::{Bbox3, Frame, Tolerance, Vec3};
use std::cell::Cell;
use std::f64::consts::TAU;

/// A curve in which two surfaces meet.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct MeetingCurve {
    pub curve: Curve3,
    pub range: (f64, f64),
    pub closed: bool,
    /// The surfaces touch along the curve without crossing.
    pub tangent: bool,
    /// False for an approximation within `deviation` of both surfaces.
    pub exact: bool,
    pub deviation: f64,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub(super) struct Meeting {
    pub curves: Vec<MeetingCurve>,
    /// Isolated points where the surfaces touch.
    pub points: Vec<Vec3>,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum SurfaceMeeting {
    None,
    Coincident { same_normal: bool },
    Meets(Meeting),
}

const STUB: &str = "surfaces whose intersection is not a line or a circle (waiting for the general surface intersection)";
const NEARLY: &str = "surfaces of revolution whose axes are nearly, but not, the same";

thread_local! {
    static ARCS: Cell<usize> = const { Cell::new(0) };
}

/// TEMPORARY, for tests: from now on, on this thread, every circle in
/// which two surfaces meet is handed to the booleans as a spline of `arcs`
/// cubic pieces that only approximates it (0 turns that off again). It
/// stands in for the approximate curves that the general surface
/// intersection will give.
pub fn approximate_circles(arcs: usize) {
    ARCS.with(|cell| cell.set(arcs.min(4096)));
}

fn unsupported() -> GeomError {
    GeomError::Unsupported(STUB)
}

fn degenerate() -> GeomError {
    GeomError::InvalidParameter("an intersection has no finite position")
}

fn across(v: Vec3, axis: Vec3) -> Vec3 {
    v - axis * v.dot(axis)
}

fn exact(curve: Curve3, range: (f64, f64), closed: bool, tangent: bool) -> MeetingCurve {
    MeetingCurve {
        curve,
        range,
        closed,
        tangent,
        exact: true,
        deviation: 0.0,
    }
}

/// The curve as the booleans get it: a line cut down to `within` (nothing
/// when it misses it), a circle or ellipse whole.
fn bounded(curve: Curve3, tangent: bool, within: &Bbox3) -> Option<MeetingCurve> {
    match &curve {
        Curve3::Line(line) => {
            let range = line_in_box(line.origin(), line.direction(), within, 0.0)?;
            (range.1 > range.0).then(|| exact(curve, range, false, tangent))
        }
        Curve3::Circle(_) | Curve3::Ellipse(_) => Some(exact(curve, (0.0, TAU), true, tangent)),
        Curve3::Nurbs(c) => {
            let range = c.domain();
            Some(exact(curve, range, false, tangent))
        }
    }
}

/// A circle as a closed cubic spline of `arcs` pieces, and how far it
/// strays from the circle.
fn spline_circle(circle: &Circle, arcs: usize) -> Result<(NurbsCurve, f64), GeomError> {
    let (frame, radius) = (circle.frame(), circle.radius());
    let arcs = arcs.max(3);
    let step = TAU / arcs as f64;
    // The usual cubic for an arc: handles of this length along the
    // tangents at its ends.
    let handle = 4.0 / 3.0 * (0.25 * step).tan() * radius;
    let at = |k: usize| {
        let (s, c) = (step * k as f64).sin_cos();
        (
            frame.point(radius * c, radius * s, 0.0),
            frame.vector(-s, c, 0.0),
        )
    };
    let mut points = Vec::with_capacity(3 * arcs + 1);
    let mut knots = vec![0.0; 4];
    let (start, _) = at(0);
    points.push(start);
    for k in 0..arcs {
        let ((p0, t0), (p1, t1)) = (at(k), at(k + 1));
        points.push(p0 + t0 * handle);
        points.push(p1 - t1 * handle);
        // Close exactly where it started.
        points.push(if k + 1 == arcs { start } else { p1 });
        let knot = (k + 1) as f64;
        knots.extend(std::iter::repeat_n(knot, if k + 1 == arcs { 4 } else { 3 }));
    }
    let curve = NurbsCurve::polynomial(3, knots, points)?;
    let mut deviation = 0.0_f64;
    for i in 0..=16 {
        let p = curve.eval(f64::from(i) / 16.0);
        let (x, y, z) = frame.local(p);
        deviation = deviation.max((x.hypot(y) - radius).hypot(z));
    }
    Ok((curve, 1.5 * deviation))
}

/// The test switch: circles as splines.
fn approximated(meeting: SurfaceMeeting) -> Result<SurfaceMeeting, GeomError> {
    let arcs = ARCS.with(Cell::get);
    let SurfaceMeeting::Meets(mut meeting) = meeting else {
        return Ok(meeting);
    };
    if arcs > 0 {
        for curve in &mut meeting.curves {
            if let Curve3::Circle(circle) = &curve.curve {
                let (spline, deviation) = spline_circle(circle, arcs)?;
                *curve = MeetingCurve {
                    range: spline.domain(),
                    curve: Curve3::Nurbs(spline),
                    closed: true,
                    tangent: curve.tangent,
                    exact: false,
                    deviation,
                };
            }
        }
    }
    Ok(SurfaceMeeting::Meets(meeting))
}

/// The curves in which two surfaces meet within `within`.
pub(super) fn meet_surfaces(
    a: &Surface,
    b: &Surface,
    within: &Bbox3,
    tolerance: &Tolerance,
) -> Result<SurfaceMeeting, GeomError> {
    let tol = tolerance.linear;
    let closed_form = matches!(
        (a, b),
        (
            Surface::Plane(_) | Surface::Cylinder(_),
            Surface::Plane(_) | Surface::Cylinder(_)
        )
    );
    if closed_form {
        let (curves, tangent) = match intersect_surfaces(a, b, tolerance)? {
            SurfaceIntersection::None => return Ok(SurfaceMeeting::None),
            SurfaceIntersection::Coincident { same_normal } => {
                return Ok(SurfaceMeeting::Coincident { same_normal });
            }
            SurfaceIntersection::Curves(curves) => (curves, false),
            SurfaceIntersection::Tangent(curves) => (curves, true),
        };
        let curves = curves
            .into_iter()
            .filter_map(|curve| bounded(curve, tangent, within))
            .collect();
        return approximated(SurfaceMeeting::Meets(Meeting {
            curves,
            points: Vec::new(),
        }));
    }
    if let Some(meeting) = through_apex_or_axis(a, b, within, tolerance)? {
        return approximated(meeting);
    }
    match common_axis(a, b, tolerance)? {
        Some(axis) => approximated(revolved(a, b, &axis, tolerance)?),
        None => {
            // A sphere that does not reach the other surface.
            for (ball, other) in [(a, b), (b, a)] {
                if let Surface::Sphere(s) = ball
                    && let Some(d) = signed_distance(other, s.frame().origin())
                    && d.abs() > s.radius() + tol
                {
                    return Ok(SurfaceMeeting::None);
                }
            }
            Err(unsupported())
        }
    }
}

/// A plane through the apex of a cone or through the axis of a torus.
fn through_apex_or_axis(
    a: &Surface,
    b: &Surface,
    within: &Bbox3,
    tolerance: &Tolerance,
) -> Result<Option<SurfaceMeeting>, GeomError> {
    let tol = tolerance.linear;
    let (plane, other) = match (a, b) {
        (Surface::Plane(p), other) | (other, Surface::Plane(p)) => (p.frame(), other),
        _ => return Ok(None),
    };
    let n = plane.z();
    match other {
        Surface::Cone(cone) => {
            let frame = cone.frame();
            let apex = frame.point(0.0, 0.0, cone.apex_v());
            if (apex - plane.origin()).dot(n).abs() > tol {
                return Ok(None);
            }
            // Rulings leave the apex at the half-angle from the direction
            // in which the cone opens. Those in the plane are across its
            // normal.
            let opens = if cone.half_angle() > 0.0 {
                frame.z()
            } else {
                -frame.z()
            };
            let (sine, cosine) = cone.half_angle().abs().sin_cos();
            let (along, sideways) = (n.dot(opens), across(n, opens));
            let lean = sideways.length();
            let only_apex = SurfaceMeeting::Meets(Meeting {
                curves: Vec::new(),
                points: vec![apex],
            });
            if lean <= tolerance.angular {
                // Across the axis: the plane meets the cone at its apex.
                return Ok(Some(only_apex));
            }
            let e1 = sideways * (1.0 / lean);
            let e2 = opens.cross(e1);
            let reach = -(cosine * along) / (sine * lean);
            let ruling = |angle: f64| {
                let (s, c) = angle.sin_cos();
                opens * cosine + (e1 * c + e2 * s) * sine
            };
            let (directions, tangent) = if (reach.abs() - 1.0).abs() <= tolerance.angular {
                (
                    vec![ruling(if reach > 0.0 { 0.0 } else { TAU / 2.0 })],
                    true,
                )
            } else if reach.abs() > 1.0 {
                return Ok(Some(only_apex));
            } else {
                let spread = reach.acos();
                (vec![ruling(-spread), ruling(spread)], false)
            };
            let mut curves = Vec::new();
            for direction in directions {
                let line = Line::new(apex, direction)?;
                // From the apex outwards: the other way is not the cone.
                if let Some((_, leave)) = line_in_box(apex, line.direction(), within, 0.0)
                    && leave > 0.0
                {
                    curves.push(exact(Curve3::Line(line), (0.0, leave), false, tangent));
                }
            }
            Ok(Some(SurfaceMeeting::Meets(Meeting {
                curves,
                points: Vec::new(),
            })))
        }
        Surface::Torus(torus) => {
            let frame = torus.frame();
            let (major, minor) = torus.radii();
            let tilt = n.dot(frame.z()).abs();
            if tilt > tolerance.angular || (frame.origin() - plane.origin()).dot(n).abs() > tol {
                return Ok(None);
            }
            // Out from the axis, in the plane.
            let out = frame.z().cross(n).normalized().ok_or_else(degenerate)?;
            let mut curves = Vec::new();
            for side in [1.0, -1.0] {
                let center = frame.origin() + out * (major * side);
                let around = Frame::new(center, n, out * side).ok_or_else(degenerate)?;
                let circle = Curve3::Circle(Circle::new(around, minor)?);
                curves.push(exact(circle, (0.0, TAU), true, false));
            }
            Ok(Some(SurfaceMeeting::Meets(Meeting {
                curves,
                points: Vec::new(),
            })))
        }
        _ => Ok(None),
    }
}

/// What a surface is a surface of revolution about.
enum Axis {
    /// Any line through this point (a sphere).
    Through(Vec3),
    /// Any line in this direction (a plane).
    Along(Vec3),
    /// This line, with the surface's own frame on it.
    Line(Frame),
}

fn axis_of(surface: &Surface) -> Option<Axis> {
    Some(match surface {
        Surface::Plane(s) => Axis::Along(s.frame().z()),
        Surface::Sphere(s) => Axis::Through(s.frame().origin()),
        Surface::Cylinder(s) => Axis::Line(*s.frame()),
        Surface::Cone(s) => Axis::Line(*s.frame()),
        Surface::Torus(s) => Axis::Line(*s.frame()),
        Surface::Bezier(_) | Surface::Nurbs(_) => return None,
    })
}

/// A frame whose z axis both surfaces are surfaces of revolution about,
/// when there is one.
fn common_axis(a: &Surface, b: &Surface, tol: &Tolerance) -> Result<Option<Frame>, GeomError> {
    let (Some(first), Some(second)) = (axis_of(a), axis_of(b)) else {
        return Ok(None);
    };
    // Parallel, not parallel, or too close to call.
    let parallel = |u: Vec3, v: Vec3| -> Result<bool, GeomError> {
        let sine = u.cross(v).length();
        if sine <= tol.angular {
            Ok(true)
        } else if sine < NEARLY_PARALLEL {
            Err(GeomError::Unsupported(NEARLY))
        } else {
            Ok(false)
        }
    };
    let on = |frame: &Frame, p: Vec3| across(p - frame.origin(), frame.z()).length() <= tol.linear;
    Ok(match (first, second) {
        (Axis::Line(f), Axis::Line(g)) => {
            (parallel(f.z(), g.z())? && on(&f, g.origin())).then_some(f)
        }
        (Axis::Line(f), Axis::Along(n)) | (Axis::Along(n), Axis::Line(f)) => {
            parallel(f.z(), n)?.then_some(f)
        }
        (Axis::Line(f), Axis::Through(c)) | (Axis::Through(c), Axis::Line(f)) => {
            on(&f, c).then_some(f)
        }
        (Axis::Through(c), Axis::Along(n)) | (Axis::Along(n), Axis::Through(c)) => {
            Frame::from_z(c, n)
        }
        (Axis::Through(c), Axis::Through(d)) => {
            if c.distance(d) <= tol.linear {
                Frame::from_z(c, Vec3::Z)
            } else {
                Frame::from_z(c, d - c)
            }
        }
        (Axis::Along(_), Axis::Along(_)) => None,
    })
}

/// The section of a surface of revolution with a half-plane through its
/// axis, in (distance from the axis, height along it).
#[derive(Clone, Copy)]
enum Profile {
    /// A line through `p` along the unit direction `d`.
    Line {
        p: (f64, f64),
        d: (f64, f64),
    },
    Circle {
        c: (f64, f64),
        r: f64,
    },
}

fn profile(surface: &Surface, axis: &Frame) -> Option<Profile> {
    let height = |p: Vec3| (p - axis.origin()).dot(axis.z());
    Some(match surface {
        Surface::Plane(s) => Profile::Line {
            p: (0.0, height(s.frame().origin())),
            d: (1.0, 0.0),
        },
        Surface::Cylinder(s) => Profile::Line {
            p: (s.radius(), 0.0),
            d: (0.0, 1.0),
        },
        Surface::Cone(s) => {
            // Its own axis may point the other way.
            let up = s.frame().z().dot(axis.z()).signum();
            let slope = s.half_angle().tan();
            let length = slope.hypot(1.0);
            Profile::Line {
                p: (s.radius(), height(s.frame().origin())),
                d: (slope / length, up / length),
            }
        }
        Surface::Sphere(s) => Profile::Circle {
            c: (0.0, height(s.frame().origin())),
            r: s.radius(),
        },
        Surface::Torus(s) => Profile::Circle {
            c: (s.radii().0, height(s.frame().origin())),
            r: s.radii().1,
        },
        Surface::Bezier(_) | Surface::Nurbs(_) => return None,
    })
}

/// Where two profiles meet: the points, each with whether they only touch
/// there; `None` when they are the same curve.
#[allow(clippy::type_complexity)]
fn profiles_meet(
    first: Profile,
    second: Profile,
    tol: &Tolerance,
) -> Result<Option<Vec<((f64, f64), bool)>>, GeomError> {
    let cross = |u: (f64, f64), v: (f64, f64)| u.0 * v.1 - u.1 * v.0;
    let sub = |u: (f64, f64), v: (f64, f64)| (u.0 - v.0, u.1 - v.1);
    let along = |p: (f64, f64), d: (f64, f64), t: f64| (p.0 + d.0 * t, p.1 + d.1 * t);
    Ok(Some(match (first, second) {
        (Profile::Line { p, d }, Profile::Line { p: q, d: e }) => {
            let sine = cross(d, e);
            if sine.abs() <= tol.angular {
                return Ok(if cross(sub(q, p), d).abs() <= tol.linear {
                    None
                } else {
                    Some(Vec::new())
                });
            }
            if sine.abs() < NEARLY_PARALLEL {
                return Err(GeomError::Unsupported(NEARLY));
            }
            vec![(along(p, d, cross(sub(q, p), e) / sine), false)]
        }
        (Profile::Line { p, d }, Profile::Circle { c, r })
        | (Profile::Circle { c, r }, Profile::Line { p, d }) => {
            // The foot of the centre on the line, and how far off it is.
            let to_center = sub(c, p);
            let foot = to_center.0 * d.0 + to_center.1 * d.1;
            let off = cross(d, to_center).abs();
            if off > r + tol.linear {
                Vec::new()
            } else if (off - r).abs() <= tol.linear {
                vec![(along(p, d, foot), true)]
            } else {
                let half = ((r - off) * (r + off)).max(0.0).sqrt();
                vec![
                    (along(p, d, foot - half), false),
                    (along(p, d, foot + half), false),
                ]
            }
        }
        (Profile::Circle { c, r }, Profile::Circle { c: k, r: s }) => {
            let between = sub(k, c);
            let distance = between.0.hypot(between.1);
            if distance <= tol.linear {
                return Ok(if (r - s).abs() <= tol.linear {
                    None
                } else {
                    Some(Vec::new())
                });
            }
            let toward = (between.0 / distance, between.1 / distance);
            let side = (-toward.1, toward.0);
            if distance > r + s + tol.linear || distance < (r - s).abs() - tol.linear {
                Vec::new()
            } else if (distance - (r + s)).abs() <= tol.linear {
                vec![(along(c, toward, r), true)]
            } else if (distance - (r - s).abs()).abs() <= tol.linear {
                vec![(along(c, toward, if r >= s { r } else { -r }), true)]
            } else {
                let reach = (distance * distance + r * r - s * s) / (2.0 * distance);
                let half = ((r - reach) * (r + reach)).max(0.0).sqrt();
                let foot = along(c, toward, reach);
                vec![
                    (along(foot, side, -half), false),
                    (along(foot, side, half), false),
                ]
            }
        }
    }))
}

/// True when the normals of two coincident surfaces agree.
fn normals_agree(a: &Surface, b: &Surface) -> Result<bool, GeomError> {
    let (u, v) = match a {
        Surface::Cone(cone) => (
            0.3,
            if cone.radius() > 0.0 {
                0.0
            } else {
                cone.half_angle().signum()
            },
        ),
        _ => (0.3, 0.2),
    };
    let p = a.eval(u, v);
    let (s, t) = b.project(p)?;
    match (a.normal(u, v), b.normal(s, t)) {
        (Some(n), Some(m)) => Ok(n.dot(m) > 0.0),
        _ => Err(degenerate()),
    }
}

/// Two surfaces of revolution about the z axis of `axis`.
fn revolved(
    a: &Surface,
    b: &Surface,
    axis: &Frame,
    tolerance: &Tolerance,
) -> Result<SurfaceMeeting, GeomError> {
    let tol = tolerance.linear;
    let (Some(first), Some(second)) = (profile(a, axis), profile(b, axis)) else {
        return Err(unsupported());
    };
    let Some(found) = profiles_meet(first, second, tolerance)? else {
        return Ok(SurfaceMeeting::Coincident {
            same_normal: normals_agree(a, b)?,
        });
    };
    let mut meeting = Meeting::default();
    let mut radii: Vec<(f64, f64)> = Vec::new();
    for ((radius, height), tangent) in found {
        // The other side of the axis is the same circle, or the other
        // half of a cone, which is not there.
        if radius < -tol || !(radius.is_finite() && height.is_finite()) {
            continue;
        }
        let center = axis.point(0.0, 0.0, height);
        if radius <= tol {
            if !meeting.points.iter().any(|p| p.distance(center) <= tol) {
                meeting.points.push(center);
            }
            continue;
        }
        if radii
            .iter()
            .any(|&(r, h)| (r - radius).abs() <= tol && (h - height).abs() <= tol)
        {
            continue;
        }
        radii.push((radius, height));
        let ring = axis.at(center).ok_or_else(degenerate)?;
        let circle = Curve3::Circle(Circle::new(ring, radius)?);
        meeting
            .curves
            .push(exact(circle, (0.0, TAU), true, tangent));
    }
    if meeting.curves.is_empty() && meeting.points.is_empty() {
        return Ok(SurfaceMeeting::None);
    }
    Ok(SurfaceMeeting::Meets(meeting))
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft3d_geom::{Cone, Cylinder, Plane, Sphere, Torus};

    fn tol() -> Tolerance {
        Tolerance::default()
    }

    fn everywhere() -> Bbox3 {
        Bbox3::from_points([Vec3::new(-50.0, -50.0, -50.0), Vec3::new(50.0, 50.0, 50.0)])
    }

    fn tilted() -> Frame {
        Frame::new(
            Vec3::new(1.0, -2.0, 0.5),
            Vec3::new(0.3, -0.4, 1.0),
            Vec3::new(1.0, 0.2, 0.0),
        )
        .unwrap()
    }

    fn curves(a: &Surface, b: &Surface) -> Meeting {
        // Either way round.
        let forward = meet_surfaces(a, b, &everywhere(), &tol()).unwrap();
        let backward = meet_surfaces(b, a, &everywhere(), &tol()).unwrap();
        let (SurfaceMeeting::Meets(meeting), SurfaceMeeting::Meets(other)) = (forward, backward)
        else {
            panic!("{} and {} should meet", a.kind(), b.kind());
        };
        assert_eq!(meeting.curves.len(), other.curves.len());
        assert_eq!(meeting.points.len(), other.points.len());
        // Every curve lies on both surfaces, over its whole range.
        for curve in &meeting.curves {
            for k in 0..=16 {
                let t = curve.range.0 + (curve.range.1 - curve.range.0) * f64::from(k) / 16.0;
                let p = curve.curve.eval(t);
                for surface in [a, b] {
                    let d = signed_distance(surface, p).unwrap();
                    assert!(d.abs() <= curve.deviation + 1e-9, "{}: {d}", surface.kind());
                }
            }
        }
        for &p in &meeting.points {
            for surface in [a, b] {
                assert!(signed_distance(surface, p).unwrap().abs() < 1e-9);
            }
        }
        meeting
    }

    #[test]
    fn surfaces_about_one_axis_meet_in_circles() {
        let f = tilted();
        let up = |h: f64| f.at(f.point(0.0, 0.0, h)).unwrap();
        let plane = |h: f64| Surface::Plane(Plane::new(up(h)));
        let ball = Surface::Sphere(Sphere::new(up(1.0), 2.0).unwrap());
        let cylinder = Surface::Cylinder(Cylinder::new(f, 1.5).unwrap());
        // Radius 2 at height 0, closing at height 4.
        let cone = Surface::Cone(Cone::new(f, 2.0, -(0.5_f64).atan()).unwrap());
        let torus = Surface::Torus(Torus::new(f, 4.0, 1.0).unwrap());

        let radius = |m: &Meeting, i: usize| match &m.curves[i].curve {
            Curve3::Circle(c) => c.radius(),
            other => panic!("expected a circle, got {}", other.kind()),
        };
        // A plane and a sphere: a circle; at a pole a point; beyond it
        // nothing.
        let cut = curves(&plane(2.0), &ball);
        assert_eq!((cut.curves.len(), cut.points.len()), (1, 0));
        assert!((radius(&cut, 0) - 3.0_f64.sqrt()).abs() < 1e-12);
        assert!(!cut.curves[0].tangent && cut.curves[0].closed && cut.curves[0].exact);
        let touch = curves(&plane(3.0), &ball);
        assert_eq!((touch.curves.len(), touch.points.len()), (0, 1));
        assert!(touch.points[0].distance(f.point(0.0, 0.0, 3.0)) < 1e-12);
        assert_eq!(
            meet_surfaces(&plane(3.5), &ball, &everywhere(), &tol()).unwrap(),
            SurfaceMeeting::None
        );
        // A plane at any angle, through a tilted frame of its own.
        let slanted = Surface::Plane(Plane::new(
            Frame::new(f.point(0.5, 0.0, 1.0), Vec3::new(1.0, 2.0, 0.5), Vec3::X).unwrap(),
        ));
        assert_eq!(curves(&slanted, &ball).curves.len(), 1);
        // Two spheres: a circle about the line through their centres.
        let other =
            Surface::Sphere(Sphere::new(f.at(f.point(2.0, 1.0, 1.0)).unwrap(), 1.5).unwrap());
        let lens = curves(&ball, &other);
        assert_eq!(lens.curves.len(), 1);
        let kissing =
            Surface::Sphere(Sphere::new(f.at(f.point(3.5, 0.0, 1.0)).unwrap(), 1.5).unwrap());
        let touch = curves(&ball, &kissing);
        assert_eq!((touch.curves.len(), touch.points.len()), (0, 1));
        // A cylinder through a sphere on its axis: two circles; of the
        // sphere's own radius: one, where they touch.
        let through = curves(&cylinder, &ball);
        assert_eq!(through.curves.len(), 2);
        assert!((radius(&through, 0) - 1.5).abs() < 1e-12);
        let snug = Surface::Cylinder(Cylinder::new(f, 2.0).unwrap());
        let belt = curves(&snug, &ball);
        assert_eq!(belt.curves.len(), 1);
        assert!(belt.curves[0].tangent);
        // A cone: with a plane across it a circle, at its apex a point;
        // with a cylinder a circle; with a sphere about its axis two.
        let ring = curves(&plane(2.0), &cone);
        assert!((radius(&ring, 0) - 1.0).abs() < 1e-12);
        let tip = curves(&plane(4.0), &cone);
        assert_eq!((tip.curves.len(), tip.points.len()), (0, 1));
        assert_eq!(
            meet_surfaces(&plane(5.0), &cone, &everywhere(), &tol()).unwrap(),
            SurfaceMeeting::None
        );
        assert_eq!(curves(&cylinder, &cone).curves.len(), 1);
        assert_eq!(curves(&ball, &cone).curves.len(), 2);
        // The same cone described from its other end is the same surface.
        let flipped = Frame::new(f.point(0.0, 0.0, 2.0), -f.z(), f.x()).unwrap();
        let same = Surface::Cone(Cone::new(flipped, 1.0, (0.5_f64).atan()).unwrap());
        assert_eq!(
            meet_surfaces(&cone, &same, &everywhere(), &tol()).unwrap(),
            SurfaceMeeting::Coincident { same_normal: true }
        );
        // A torus: a plane across its axis cuts two circles, on top of it
        // it touches along one; a cylinder through the middle of the
        // tube two.
        let sliced = curves(&plane(0.5), &torus);
        assert_eq!(sliced.curves.len(), 2);
        let lid = curves(&plane(1.0), &torus);
        assert_eq!(lid.curves.len(), 1);
        assert!(lid.curves[0].tangent);
        assert!((radius(&lid, 0) - 4.0).abs() < 1e-12);
        let core = Surface::Cylinder(Cylinder::new(f, 4.0).unwrap());
        assert_eq!(curves(&core, &torus).curves.len(), 2);
        assert_eq!(
            meet_surfaces(&torus, &torus, &everywhere(), &tol()).unwrap(),
            SurfaceMeeting::Coincident { same_normal: true }
        );
    }

    #[test]
    fn a_plane_through_an_apex_or_an_axis() {
        let f = tilted();
        let cone = Surface::Cone(Cone::new(f, 2.0, -(0.5_f64).atan()).unwrap());
        let apex = f.point(0.0, 0.0, 4.0);
        // Through the axis: two rulings from the apex.
        let through = Surface::Plane(Plane::new(Frame::new(f.origin(), f.y(), f.x()).unwrap()));
        let rulings = curves(&through, &cone);
        assert_eq!(rulings.curves.len(), 2);
        for curve in &rulings.curves {
            assert_eq!(curve.range.0, 0.0);
            assert!(curve.curve.eval(0.0).distance(apex) < 1e-12);
            assert!(!curve.tangent && !curve.closed);
        }
        // Through the apex but flatter than the cone: the apex alone.
        let flat = Surface::Plane(Plane::new(
            Frame::new(apex, f.vector(0.2, 0.0, 1.0), f.y()).unwrap(),
        ));
        let tip = curves(&flat, &cone);
        assert_eq!((tip.curves.len(), tip.points.len()), (0, 1));
        // Along one ruling: tangent there.
        let ruling = f.vector(2.0, 0.0, -4.0);
        let along = Surface::Plane(Plane::new(
            Frame::new(apex, f.vector(4.0, 0.0, 2.0), ruling).unwrap(),
        ));
        let touch = curves(&along, &cone);
        assert_eq!(touch.curves.len(), 1);
        assert!(touch.curves[0].tangent);
        // Not through the apex and not across the axis: not here.
        let beside = Surface::Plane(Plane::new(
            Frame::new(f.point(0.5, 0.0, 0.0), f.x(), f.y()).unwrap(),
        ));
        assert!(matches!(
            meet_surfaces(&beside, &cone, &everywhere(), &tol()),
            Err(GeomError::Unsupported(_))
        ));
        // A plane through a torus' axis: the two sections of the tube.
        let torus = Surface::Torus(Torus::new(f, 4.0, 1.0).unwrap());
        let halves = curves(&through, &torus);
        assert_eq!(halves.curves.len(), 2);
    }

    #[test]
    fn lines_are_cut_down_to_the_box_and_circles_can_be_splines() {
        let a = Surface::Plane(Plane::new(Frame::WORLD));
        let b = Surface::Plane(Plane::new(
            Frame::new(Vec3::ZERO, Vec3::Y, Vec3::X).unwrap(),
        ));
        let small = Bbox3::from_points([Vec3::new(-1.0, -1.0, -1.0), Vec3::new(2.0, 1.0, 1.0)]);
        let SurfaceMeeting::Meets(meeting) = meet_surfaces(&a, &b, &small, &tol()).unwrap() else {
            panic!("expected a line");
        };
        let line = &meeting.curves[0];
        let (p, q) = (line.curve.eval(line.range.0), line.curve.eval(line.range.1));
        assert!((p.x.min(q.x) + 1.0).abs() < 1e-5 && (p.x.max(q.x) - 2.0).abs() < 1e-5);
        let away = Bbox3::from_points([Vec3::new(-1.0, 5.0, 5.0), Vec3::new(2.0, 6.0, 6.0)]);
        assert_eq!(
            meet_surfaces(&a, &b, &away, &tol()).unwrap(),
            SurfaceMeeting::Meets(Meeting::default())
        );

        let ball = Surface::Sphere(Sphere::new(Frame::WORLD, 2.0).unwrap());
        approximate_circles(12);
        let meeting = curves(&a, &ball);
        approximate_circles(0);
        let curve = &meeting.curves[0];
        assert!(matches!(curve.curve, Curve3::Nurbs(_)));
        assert!(curve.closed && !curve.exact);
        // Really an approximation, and within what it says.
        assert!(
            curve.deviation > 1e-9 && curve.deviation < 2e-6,
            "{}",
            curve.deviation
        );
        assert!(
            curve
                .curve
                .eval(curve.range.0)
                .distance(curve.curve.eval(curve.range.1))
                == 0.0
        );
    }
}
