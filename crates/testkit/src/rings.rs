//! Bodies built by hand whose faces are bounded by loops of arcs of one
//! circle, walked along a path of angles: the faces a loop that goes
//! round its circle twice, or runs back over itself, makes, and the
//! valid ones beside them (`docs/design/topology.md` › *Loops that are
//! simple*). A path `[θ0, θ1, …, θn]` is a loop of arcs from `θi` to
//! `θi+1` (counter-clockwise about the circle's frame where `θi+1 > θi`,
//! clockwise where it is less), with `θn` the same angle as `θ0` up to
//! whole turns: `[0, 2π/3, 4π/3, 2π]` is a circle of three arcs, and
//! `[0, 4π/3, 8π/3, 4π]` the same circle gone round twice. Arcs meet at
//! one vertex per angle (taken modulo a turn), so a loop that goes round
//! twice has the vertices a circle of its arcs has.

use craft3d_geom::{Circle, Curve3, Cylinder, GeomError, Line, NurbsCurve, Plane, Surface};
use craft3d_math::Portable;
use craft3d_math::{Frame, Vec3};
use craft3d_topo::{Body, BodyKind, Coedge, VertexId};
use std::f64::consts::TAU;

fn invalid(message: &'static str) -> GeomError {
    GeomError::InvalidParameter(message)
}

/// The coedges walked the other way: last first, each turned.
pub fn turned(coedges: &[Coedge]) -> Vec<Coedge> {
    coedges
        .iter()
        .rev()
        .map(|c| Coedge {
            edge: c.edge,
            reversed: !c.reversed,
        })
        .collect()
}

/// An angle modulo a turn, in `[0, 2π)`.
fn turn_of(angle: f64) -> f64 {
    let a = angle.rem_euclid(TAU);
    if TAU - a < 1e-12 { 0.0 } else { a }
}

/// The arcs of `path` on the circle of `radius` in the xy plane of
/// `frame`, added to `body` (one vertex per angle modulo a turn, one edge
/// per step), as the coedges of the loop that walks them.
pub fn arc_chain(
    body: &mut Body,
    frame: &Frame,
    radius: f64,
    path: &[f64],
) -> Result<Vec<Coedge>, GeomError> {
    if path.len() < 2 || path.iter().any(|a| !a.is_finite()) {
        return Err(invalid("a path of two finite angles at least"));
    }
    let circle = Circle::new(*frame, radius)?;
    let mut vertices: Vec<(f64, VertexId)> = Vec::new();
    let mut vertex = |body: &mut Body, angle: f64| {
        let key = turn_of(angle);
        let near = |k: f64| (k - key).abs() < 1e-9 || TAU - (k - key).abs() < 1e-9;
        if let Some((_, id)) = vertices.iter().find(|(k, _)| near(*k)) {
            return *id;
        }
        let (s, c) = key.psin_cos();
        let id = body.add_vertex(frame.point(radius * c, radius * s, 0.0));
        vertices.push((key, id));
        id
    };
    let mut coedges = Vec::with_capacity(path.len() - 1);
    for pair in path.windows(2) {
        let [a, b] = pair else {
            continue;
        };
        if a == b {
            return Err(invalid("an arc of no length"));
        }
        let (low, high) = if a < b { (*a, *b) } else { (*b, *a) };
        let start = turn_of(low);
        let range = (start, start + (high - low));
        let (v0, v1) = (vertex(body, low), vertex(body, high));
        let edge = body.add_edge(Curve3::Circle(circle), range, v0, v1);
        coedges.push(Coedge {
            edge,
            reversed: b < a,
        });
    }
    Ok(coedges)
}

/// A disc of `radius` and `height` standing on the xy plane of `frame`,
/// its lid bounded by the arcs of `top` and its floor by those of
/// `bottom` (both paths counter-clockwise about the frame's z), its wall
/// one face of the cylinder with the two as its band loops. With
/// `[0, 2π/3, 4π/3, 2π]` for both it is the cylinder; a path that goes
/// round twice gives a body every count of which is right.
pub fn disc(
    frame: &Frame,
    radius: f64,
    height: f64,
    top: &[f64],
    bottom: &[f64],
) -> Result<Body, GeomError> {
    let mut body = Body::new(BodyKind::Solid);
    let top_frame = frame
        .at(frame.point(0.0, 0.0, height))
        .ok_or(invalid("a frame that is not finite"))?;
    let floor_frame = frame
        .reversed_at(frame.origin())
        .ok_or(invalid("a frame that is not finite"))?;
    let low = arc_chain(&mut body, frame, radius, bottom)?;
    let high = arc_chain(&mut body, &top_frame, radius, top)?;
    let wall_loops = vec![body.add_loop(low.clone()), body.add_loop(turned(&high))];
    let wall = body.add_face(
        Surface::Cylinder(Cylinder::new(*frame, radius)?),
        false,
        wall_loops,
    );
    let floor_loop = body.add_loop(turned(&low));
    let floor = body.add_face(
        Surface::Plane(Plane::new(floor_frame)),
        false,
        vec![floor_loop],
    );
    let lid_loop = body.add_loop(high);
    let lid = body.add_face(Surface::Plane(Plane::new(top_frame)), false, vec![lid_loop]);
    body.add_shell(vec![wall, floor, lid]);
    Ok(body)
}

/// One face of the xy plane of `frame` bounded by the arcs of `path` on
/// the circle of `radius` about its origin, as a sheet: the lid of
/// [`disc`] alone.
pub fn lid(frame: &Frame, radius: f64, path: &[f64]) -> Result<Body, GeomError> {
    let mut body = Body::new(BodyKind::Sheet);
    let rim = arc_chain(&mut body, frame, radius, path)?;
    let lp = body.add_loop(rim);
    let face = body.add_face(Surface::Plane(Plane::new(*frame)), false, vec![lp]);
    body.add_shell(vec![face]);
    Ok(body)
}

/// One face of the cylinder of `radius` about the z axis of `frame`
/// between the arcs of `bottom` (at height 0) and of `top` (at
/// `height`), as a sheet: the wall of [`disc`] alone.
pub fn band(
    frame: &Frame,
    radius: f64,
    height: f64,
    top: &[f64],
    bottom: &[f64],
) -> Result<Body, GeomError> {
    let mut body = Body::new(BodyKind::Sheet);
    let top_frame = frame
        .at(frame.point(0.0, 0.0, height))
        .ok_or(invalid("a frame that is not finite"))?;
    let low = arc_chain(&mut body, frame, radius, bottom)?;
    let high = arc_chain(&mut body, &top_frame, radius, top)?;
    let loops = vec![body.add_loop(low), body.add_loop(turned(&high))];
    let wall = body.add_face(
        Surface::Cylinder(Cylinder::new(*frame, radius)?),
        false,
        loops,
    );
    body.add_shell(vec![wall]);
    Ok(body)
}

/// A square plate of `side` and `thickness` on the xy plane of `frame`,
/// centred on its z axis, with a round hole of `radius` through it whose
/// rim on the top is the arcs of `top` and on the bottom those of
/// `bottom` (both paths counter-clockwise about the frame's z: the holes'
/// loops walk them the other way). Its volume is `side² · thickness −
/// π radius² · thickness` when each path goes round once.
pub fn plate_with_hole(
    frame: &Frame,
    side: f64,
    thickness: f64,
    radius: f64,
    top: &[f64],
    bottom: &[f64],
) -> Result<Body, GeomError> {
    if !(side > 2.0 * radius && thickness > 0.0 && radius > 0.0 && side.is_finite()) {
        return Err(invalid("a hole inside a plate of positive size"));
    }
    let mut body = Body::new(BodyKind::Solid);
    let h = 0.5 * side;
    let corners = [(-h, -h), (h, -h), (h, h), (-h, h)];
    let low: Vec<VertexId> = corners
        .iter()
        .map(|&(x, y)| body.add_vertex(frame.point(x, y, 0.0)))
        .collect();
    let high: Vec<VertexId> = corners
        .iter()
        .map(|&(x, y)| body.add_vertex(frame.point(x, y, thickness)))
        .collect();
    let point = |body: &Body, v: VertexId| body.vertex(v).map(|v| v.point);
    let line = |body: &mut Body, a: VertexId, b: VertexId| -> Result<_, GeomError> {
        let (pa, pb) = (
            point(body, a).ok_or(invalid("no vertex"))?,
            point(body, b).ok_or(invalid("no vertex"))?,
        );
        let edge = body.add_edge(
            Curve3::Line(Line::through(pa, pb)?),
            (0.0, pa.distance(pb)),
            a,
            b,
        );
        Ok(edge)
    };
    let mut bottom_edges = Vec::new();
    let mut top_edges = Vec::new();
    let mut posts = Vec::new();
    for k in 0..4 {
        let next = (k + 1) % 4;
        let (Some(&a), Some(&b), Some(&c), Some(&d)) =
            (low.get(k), low.get(next), high.get(k), high.get(next))
        else {
            return Err(invalid("a corner missing"));
        };
        bottom_edges.push(line(&mut body, a, b)?);
        top_edges.push(line(&mut body, c, d)?);
        posts.push(line(&mut body, a, c)?);
    }
    let top_frame = frame
        .at(frame.point(0.0, 0.0, thickness))
        .ok_or(invalid("a frame that is not finite"))?;
    let floor_frame = frame
        .reversed_at(frame.origin())
        .ok_or(invalid("a frame that is not finite"))?;
    let hole_low = arc_chain(&mut body, frame, radius, bottom)?;
    let hole_high = arc_chain(&mut body, &top_frame, radius, top)?;
    let forward = |edge| Coedge {
        edge,
        reversed: false,
    };
    let backward = |edge| Coedge {
        edge,
        reversed: true,
    };
    let mut faces = Vec::new();
    // The lid: the square counter-clockwise, the hole clockwise.
    let outer = body.add_loop(top_edges.iter().copied().map(forward).collect());
    let hole = body.add_loop(turned(&hole_high));
    faces.push(body.add_face(
        Surface::Plane(Plane::new(top_frame)),
        false,
        vec![outer, hole],
    ));
    // The floor, seen from below.
    let outer = body.add_loop(bottom_edges.iter().rev().copied().map(backward).collect());
    let hole = body.add_loop(hole_low.clone());
    faces.push(body.add_face(
        Surface::Plane(Plane::new(floor_frame)),
        false,
        vec![outer, hole],
    ));
    // The four sides, each counter-clockwise seen from outside.
    for k in 0..4 {
        let next = (k + 1) % 4;
        let (Some(&b), Some(&t), Some(&up), Some(&down), Some(&(x0, y0)), Some(&(x1, y1))) = (
            bottom_edges.get(k),
            top_edges.get(k),
            posts.get(next),
            posts.get(k),
            corners.get(k),
            corners.get(next),
        ) else {
            return Err(invalid("a side missing"));
        };
        let along = frame.vector(x1 - x0, y1 - y0, 0.0);
        let outward = along.cross(frame.z());
        let side_frame = Frame::new(frame.point(x0, y0, 0.0), outward, along)
            .ok_or(invalid("a degenerate side"))?;
        let lp = body.add_loop(vec![forward(b), forward(up), backward(t), backward(down)]);
        faces.push(body.add_face(Surface::Plane(Plane::new(side_frame)), false, vec![lp]));
    }
    // The hole's wall, its outward normal towards the axis.
    let wall_loops = vec![body.add_loop(hole_high), body.add_loop(turned(&hole_low))];
    faces.push(body.add_face(
        Surface::Cylinder(Cylinder::new(*frame, radius)?),
        true,
        wall_loops,
    ));
    body.add_shell(faces);
    Ok(body)
}

/// A disc as [`disc`] whose lid and floor are each bounded by `turns`
/// edges, every one the whole rational quadratic circle of `radius` (a
/// spline, not a circle), closed at one vertex: one turn is the cylinder,
/// two its rims gone round twice.
pub fn spline_disc(
    frame: &Frame,
    radius: f64,
    height: f64,
    turns: usize,
) -> Result<Body, GeomError> {
    if turns == 0 {
        return Err(invalid("a turn at least"));
    }
    let mut body = Body::new(BodyKind::Solid);
    let top_frame = frame
        .at(frame.point(0.0, 0.0, height))
        .ok_or(invalid("a frame that is not finite"))?;
    let floor_frame = frame
        .reversed_at(frame.origin())
        .ok_or(invalid("a frame that is not finite"))?;
    let rim = |body: &mut Body, f: &Frame| -> Result<Vec<Coedge>, GeomError> {
        let v = body.add_vertex(f.point(radius, 0.0, 0.0));
        let curve = NurbsCurve::circle(f, radius)?;
        Ok((0..turns)
            .map(|_| Coedge {
                edge: body.add_edge(Curve3::Nurbs(curve.clone()), (0.0, 1.0), v, v),
                reversed: false,
            })
            .collect())
    };
    let low = rim(&mut body, frame)?;
    let high = rim(&mut body, &top_frame)?;
    let wall_loops = vec![body.add_loop(low.clone()), body.add_loop(turned(&high))];
    let wall = body.add_face(
        Surface::Cylinder(Cylinder::new(*frame, radius)?),
        false,
        wall_loops,
    );
    let floor_loop = body.add_loop(turned(&low));
    let floor = body.add_face(
        Surface::Plane(Plane::new(floor_frame)),
        false,
        vec![floor_loop],
    );
    let lid_loop = body.add_loop(high);
    let lid = body.add_face(Surface::Plane(Plane::new(top_frame)), false, vec![lid_loop]);
    body.add_shell(vec![wall, floor, lid]);
    Ok(body)
}

/// The rim of [`spline_disc_back`] on the circle of `radius` in the xy
/// plane of `f`, on the rational quadratic circle (a spline): from its
/// start nearly all the way round (to the parameter 0.98), back to 0.75,
/// and on again to the start (three edges between three vertices, as the
/// arcs of `[0, 359.4°, 300°, 360°]`): once round, the stretch from 0.75
/// to 0.98 run over three times.
fn spline_rim_back(body: &mut Body, f: &Frame, radius: f64) -> Result<Vec<Coedge>, GeomError> {
    let curve = NurbsCurve::circle(f, radius)?;
    let start = body.add_vertex(curve.eval(0.0));
    let far = body.add_vertex(curve.eval(0.98));
    let back = body.add_vertex(curve.eval(0.75));
    let out = body.add_edge(Curve3::Nurbs(curve.clone()), (0.0, 0.98), start, far);
    let returning = body.add_edge(Curve3::Nurbs(curve.clone()), (0.75, 0.98), back, far);
    let again = body.add_edge(Curve3::Nurbs(curve), (0.75, 1.0), back, start);
    Ok(vec![
        Coedge {
            edge: out,
            reversed: false,
        },
        Coedge {
            edge: returning,
            reversed: true,
        },
        Coedge {
            edge: again,
            reversed: false,
        },
    ])
}

/// A disc as [`spline_disc`] whose lid is bounded by the spline circle
/// gone nearly round, back and on again (once round: its volume and area
/// are the cylinder's), its floor by the whole circle.
pub fn spline_disc_back(frame: &Frame, radius: f64, height: f64) -> Result<Body, GeomError> {
    let mut body = Body::new(BodyKind::Solid);
    let top_frame = frame
        .at(frame.point(0.0, 0.0, height))
        .ok_or(invalid("a frame that is not finite"))?;
    let floor_frame = frame
        .reversed_at(frame.origin())
        .ok_or(invalid("a frame that is not finite"))?;
    let v = body.add_vertex(frame.point(radius, 0.0, 0.0));
    let low = vec![Coedge {
        edge: body.add_edge(
            Curve3::Nurbs(NurbsCurve::circle(frame, radius)?),
            (0.0, 1.0),
            v,
            v,
        ),
        reversed: false,
    }];
    let high = spline_rim_back(&mut body, &top_frame, radius)?;
    let wall_loops = vec![body.add_loop(low.clone()), body.add_loop(turned(&high))];
    let wall = body.add_face(
        Surface::Cylinder(Cylinder::new(*frame, radius)?),
        false,
        wall_loops,
    );
    let floor_loop = body.add_loop(turned(&low));
    let floor = body.add_face(
        Surface::Plane(Plane::new(floor_frame)),
        false,
        vec![floor_loop],
    );
    let lid_loop = body.add_loop(high);
    let lid = body.add_face(Surface::Plane(Plane::new(top_frame)), false, vec![lid_loop]);
    body.add_shell(vec![wall, floor, lid]);
    Ok(body)
}

/// The lid of [`spline_disc_back`] alone, as a sheet.
pub fn spline_lid_back(frame: &Frame, radius: f64) -> Result<Body, GeomError> {
    let mut body = Body::new(BodyKind::Sheet);
    let rim = spline_rim_back(&mut body, frame, radius)?;
    let lp = body.add_loop(rim);
    let face = body.add_face(Surface::Plane(Plane::new(*frame)), false, vec![lp]);
    body.add_shell(vec![face]);
    Ok(body)
}

/// A disc as [`disc`] with every rim once round in three arcs, whose lid's
/// loop also runs out along a line from its vertex at angle 0 to half the
/// radius and back along the same edge (a spur: no width there). Its
/// volume and area are the cylinder's.
pub fn disc_with_spur(frame: &Frame, radius: f64, height: f64) -> Result<Body, GeomError> {
    let path = [0.0, TAU / 3.0, 2.0 * TAU / 3.0, TAU];
    let mut body = disc(frame, radius, height, &path, &path)?;
    let lid = body.faces.get(2).and_then(|f| f.loops.first().copied());
    let first = lid
        .and_then(|l| body.loop_(l))
        .and_then(|l| l.coedges.first().copied())
        .ok_or(invalid("a disc without its lid"))?;
    let edge = body
        .edge(first.edge)
        .ok_or(invalid("a lid without its edge"))?;
    let start = if first.reversed { edge.end } else { edge.start };
    let from = body.vertex(start).ok_or(invalid("no vertex"))?.point;
    let to = frame.point(0.5 * radius, 0.0, height);
    let tip = body.add_vertex(to);
    let spur = body.add_edge(
        Curve3::Line(Line::through(from, to)?),
        (0.0, from.distance(to)),
        start,
        tip,
    );
    let lid = lid.ok_or(invalid("a disc without its lid"))?;
    let lp = body
        .loops
        .get_mut(lid.0 as usize)
        .ok_or(invalid("a disc without its lid"))?;
    lp.coedges.splice(
        0..0,
        [
            Coedge {
                edge: spur,
                reversed: false,
            },
            Coedge {
                edge: spur,
                reversed: true,
            },
        ],
    );
    Ok(body)
}

/// The cubic Bézier through `[p0, c1, c2, p3]` as a spline.
fn bezier(points: [Vec3; 4]) -> Result<NurbsCurve, GeomError> {
    NurbsCurve::polynomial(
        3,
        vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
        points.to_vec(),
    )
}

/// A sheet of one face on the xy plane of `frame`, `scale` across: the
/// region between a cubic spline from `(0, 0)` to `(2, 0)` (its first
/// half) and the lines on to `(2, 2)` and back, with, when `spike` is
/// true, the spline's second half on to `(4, 0)` walked out and back as
/// a spike (a spline of its own, split from the first by de Casteljau, so
/// the two run over each other along it: no width there). Without the
/// spike the face is the region alone, which measures the same.
pub fn spline_spike(frame: &Frame, scale: f64, spike: bool) -> Result<Body, GeomError> {
    if !(scale.is_finite() && scale > 0.0) {
        return Err(invalid("a positive, finite scale"));
    }
    let at = |x: f64, y: f64| frame.point(x * scale, y * scale, 0.0);
    let (p0, c1, c2, p3) = (at(0.0, 0.0), at(1.0, 1.0), at(3.0, -1.0), at(4.0, 0.0));
    let half = |a: Vec3, b: Vec3| (a + b) * 0.5;
    let (q0, q1, q2) = (half(p0, c1), half(c1, c2), half(c2, p3));
    let (r0, r1) = (half(q0, q1), half(q1, q2));
    let middle = half(r0, r1);
    let corner = at(2.0, 2.0);
    let mut body = Body::new(BodyKind::Sheet);
    let v0 = body.add_vertex(p0);
    let vm = body.add_vertex(middle);
    let vq = body.add_vertex(corner);
    let forward = |edge| Coedge {
        edge,
        reversed: false,
    };
    let mut coedges = Vec::new();
    if spike {
        let v1 = body.add_vertex(p3);
        // The whole spline out to the far end, and its second half back.
        let out = body.add_edge(Curve3::Nurbs(bezier([p0, c1, c2, p3])?), (0.0, 1.0), v0, v1);
        let back = body.add_edge(
            Curve3::Nurbs(bezier([middle, r1, q2, p3])?),
            (0.0, 1.0),
            vm,
            v1,
        );
        coedges.push(forward(out));
        coedges.push(Coedge {
            edge: back,
            reversed: true,
        });
    } else {
        let first = body.add_edge(
            Curve3::Nurbs(bezier([p0, q0, r0, middle])?),
            (0.0, 1.0),
            v0,
            vm,
        );
        coedges.push(forward(first));
    }
    let up = body.add_edge(
        Curve3::Line(Line::through(middle, corner)?),
        (0.0, middle.distance(corner)),
        vm,
        vq,
    );
    let home = body.add_edge(
        Curve3::Line(Line::through(corner, p0)?),
        (0.0, corner.distance(p0)),
        vq,
        v0,
    );
    coedges.push(forward(up));
    coedges.push(forward(home));
    let lp = body.add_loop(coedges);
    let face = body.add_face(Surface::Plane(Plane::new(*frame)), false, vec![lp]);
    body.add_shell(vec![face]);
    Ok(body)
}

/// Degrees to radians, for paths written in degrees.
pub fn degrees(path: &[f64]) -> Vec<f64> {
    path.iter().map(|d| d.to_radians()).collect()
}

/// A curved flange made by the sheet-metal operation, all sizes times
/// `k` (tolerance `1e-6 k`): with `pan` false a plate 100 × 60, 1 thick,
/// with a round hole of radius 15 at (50, 30) whose edge is bent 90° down
/// into a collar 4 long (bend radius 1.5); with `pan` true a disc of radius
/// 40 whose rim is bent 90° up into a pan's rim 12 long (bend radius 3).
/// Their faces close on themselves round the hole and the rim, and the
/// flat end of each is an annulus written as one loop with a bridge edge
/// used twice (`crates/kernel/tests/sheetmetal_curved.rs` builds the same
/// and holds them to their closed forms).
pub fn curved_flange(k: f64, pan: bool) -> Result<Body, craft3d_kernel::KernelError> {
    use craft3d_kernel::profile::{Curve2, SketchCurve, detect_profiles};
    use craft3d_kernel::sheetmetal::curved::CurvedSpec;
    use craft3d_kernel::sheetmetal::{Closing, Direction, LengthFrom, Spec, build_with};
    let tolerance = craft3d_math::Tolerance::new(1e-6 * k, 1e-9).ok_or(
        craft3d_kernel::KernelError::InvalidSize {
            name: "scale",
            value: k,
        },
    )?;
    let line = |id: u64, a: [f64; 2], b: [f64; 2]| SketchCurve {
        id,
        curve: Curve2::Line {
            start: [a[0] * k, a[1] * k],
            end: [b[0] * k, b[1] * k],
        },
    };
    let circle = |id: u64, c: [f64; 2], r: f64| SketchCurve {
        id,
        curve: Curve2::Circle {
            center: [c[0] * k, c[1] * k],
            radius: r * k,
        },
    };
    let (curves, edge, length, up, radius, kk) = if pan {
        (vec![circle(1, [0.0, 0.0], 40.0)], 1, 12.0, true, 3.0, 0.5)
    } else {
        (
            vec![
                line(1, [0.0, 0.0], [100.0, 0.0]),
                line(2, [100.0, 0.0], [100.0, 60.0]),
                line(3, [100.0, 60.0], [0.0, 60.0]),
                line(4, [0.0, 60.0], [0.0, 0.0]),
                circle(5, [50.0, 30.0], 15.0),
            ],
            5,
            4.0,
            false,
            1.5,
            0.4,
        )
    };
    let base = detect_profiles(&curves, &tolerance)?
        .into_iter()
        .next()
        .ok_or(craft3d_kernel::KernelError::InvalidSize {
            name: "profile",
            value: k,
        })?;
    let spec = Spec {
        base,
        plane: Frame::WORLD,
        thickness: k,
        flanges: Vec::new(),
        relief: None,
        bend_relief: None,
        corner: Closing::OPEN,
        corners: Vec::new(),
        cutouts: Vec::new(),
        jogs: Vec::new(),
        ids: Vec::new(),
    };
    let flange = CurvedSpec {
        edge,
        length: length * k,
        angle: std::f64::consts::FRAC_PI_2,
        direction: if up { Direction::Up } else { Direction::Down },
        radius: radius * k,
        k: kk,
        from: LengthFrom::Tangent,
        id: 1,
    };
    Ok(build_with(&spec, &[flange], None, &tolerance)?.solid.body)
}

/// A washer of outer radius `2 r`, inner radius `r` and height `r` on the
/// xy plane of `frame`, its lid and floor each an annulus written as one
/// loop (the outer circle, a bridge across to the inner circle, the inner
/// circle back, the bridge again), its walls band faces of the two
/// cylinders: a right solid of genus 1, the bridges one edge used twice by
/// one loop at one place of its plane.
pub fn bridged_washer(frame: &Frame, r: f64) -> Result<Body, GeomError> {
    let mut body = Body::new(BodyKind::Solid);
    let top_frame = frame
        .at(frame.point(0.0, 0.0, r))
        .ok_or(invalid("a frame that is not finite"))?;
    let floor_frame = frame
        .reversed_at(frame.origin())
        .ok_or(invalid("a frame that is not finite"))?;
    let rims = |body: &mut Body, f: &Frame| -> Result<_, GeomError> {
        let outer_v = body.add_vertex(f.point(2.0 * r, 0.0, 0.0));
        let inner_v = body.add_vertex(f.point(r, 0.0, 0.0));
        let outer = body.add_edge(
            Curve3::Circle(Circle::new(*f, 2.0 * r)?),
            (0.0, TAU),
            outer_v,
            outer_v,
        );
        let inner = body.add_edge(
            Curve3::Circle(Circle::new(*f, r)?),
            (0.0, TAU),
            inner_v,
            inner_v,
        );
        let (a, b) = (f.point(2.0 * r, 0.0, 0.0), f.point(r, 0.0, 0.0));
        let bridge = body.add_edge(
            Curve3::Line(Line::through(a, b)?),
            (0.0, r),
            outer_v,
            inner_v,
        );
        Ok((outer, inner, bridge))
    };
    let (bo, bi, bb) = rims(&mut body, frame)?;
    let (to, ti, tb) = rims(&mut body, &top_frame)?;
    let co = |edge, reversed| Coedge { edge, reversed };
    // The lid: the outer circle counter-clockwise about +z, across, the
    // inner circle clockwise, back.
    let lid_loop = body.add_loop(vec![
        co(to, false),
        co(tb, false),
        co(ti, true),
        co(tb, true),
    ]);
    let lid = body.add_face(Surface::Plane(Plane::new(top_frame)), false, vec![lid_loop]);
    // The floor, seen from below: the same walked the other way.
    let floor_loop = body.add_loop(vec![
        co(bb, false),
        co(bi, false),
        co(bb, true),
        co(bo, true),
    ]);
    let floor = body.add_face(
        Surface::Plane(Plane::new(floor_frame)),
        false,
        vec![floor_loop],
    );
    let outer_wall_loops = vec![
        body.add_loop(vec![co(bo, false)]),
        body.add_loop(vec![co(to, true)]),
    ];
    let outer_wall = body.add_face(
        Surface::Cylinder(Cylinder::new(*frame, 2.0 * r)?),
        false,
        outer_wall_loops,
    );
    let inner_wall_loops = vec![
        body.add_loop(vec![co(ti, false)]),
        body.add_loop(vec![co(bi, true)]),
    ];
    let inner_wall = body.add_face(
        Surface::Cylinder(Cylinder::new(*frame, r)?),
        true,
        inner_wall_loops,
    );
    body.add_shell(vec![lid, floor, outer_wall, inner_wall]);
    Ok(body)
}
