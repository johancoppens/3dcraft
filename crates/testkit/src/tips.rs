//! Solids closed round in one point over a flat end: the faces of
//! `craft3d_geom::tip` (rational B-spline faces with one border in the
//! tip, a pole) put together into a body by hand, over a prism on a
//! polygon. Nothing in the kernel builds a tip yet.
//!
//! Their volumes are closed forms (oracle 1 in docs/testing.md): with
//! quarter ellipses across (fullness `√2 − 1`) two thirds of base times
//! length, with parabolas (fullness ½) eleven fifteenths, wherever the
//! tip stands over the base. With the tip near a corner of the polygon,
//! or over a thin one, the meridians of the faces beside it leave the
//! pole within a small angle of each other: the bodies the exact measure
//! and the tessellator are held to in `kernel/tests/pole_measure.rs`.

use craft3d_geom::pole::Border;
use craft3d_geom::tip::{Tip, TipRim, TipSpec, tip_nets};
use craft3d_geom::{Curve3, Line, NurbsCurve, NurbsSurface, Plane, Surface};
use craft3d_math::{Frame, Vec3};
use craft3d_topo::{Body, BodyKind, Coedge, EdgeId};

/// The fullness of quarter ellipses across the tip, and their share of
/// base times length.
pub const ROUND: (f64, f64) = (std::f64::consts::SQRT_2 - 1.0, 2.0 / 3.0);
/// The fullness of parabolas, and their share.
pub const PARABOLA: (f64, f64) = (0.5, 11.0 / 15.0);

/// The area of the polygon through `corners`, counter-clockwise.
pub fn polygon_area(corners: &[[f64; 2]]) -> f64 {
    let mut sum = 0.0;
    for (k, &[x, y]) in corners.iter().enumerate() {
        let [nx, ny] = corners
            .get((k + 1) % corners.len())
            .copied()
            .unwrap_or([x, y]);
        sum += 0.5 * (x * ny - y * nx);
    }
    sum
}

/// One Bézier piece of a section in its plane: degree, control points,
/// weights.
pub type Piece = (usize, Vec<[f64; 2]>, Vec<f64>);

/// The sides of the polygon through `corners`, as pieces.
pub fn polygon(corners: &[[f64; 2]]) -> Vec<Piece> {
    corners
        .iter()
        .enumerate()
        .map(|(k, &from)| {
            let to = corners
                .get((k + 1) % corners.len())
                .copied()
                .unwrap_or(from);
            (1, vec![from, to], vec![1.0; 2])
        })
        .collect()
}

/// The ellipse with the half axes `a` along x and `b` along y about the
/// origin, counter-clockwise, as four rational quarters: its area is
/// `π a b`.
pub fn ellipse(a: f64, b: f64) -> Vec<Piece> {
    let quarters = [
        [(1.0, 0.0), (1.0, 1.0), (0.0, 1.0)],
        [(0.0, 1.0), (-1.0, 1.0), (-1.0, 0.0)],
        [(-1.0, 0.0), (-1.0, -1.0), (0.0, -1.0)],
        [(0.0, -1.0), (1.0, -1.0), (1.0, 0.0)],
    ];
    quarters
        .iter()
        .map(|quarter| {
            (
                2,
                quarter.iter().map(|&(x, y)| [a * x, b * y]).collect(),
                vec![1.0, std::f64::consts::FRAC_1_SQRT_2, 1.0],
            )
        })
        .collect()
}

/// The side faces of a prism on a section (counter-clockwise, in the xy
/// plane of `frame`, times `k`), each from `2 k` behind the plane up to
/// it.
fn sides(frame: &Frame, k: f64, section: &[Piece]) -> Result<Vec<NurbsSurface>, String> {
    let at = |[x, y]: [f64; 2], z: f64| frame.point(x * k, y * k, z * k);
    let mut faces = Vec::with_capacity(section.len());
    for (degree, points, weights) in section {
        let mut net = Vec::with_capacity(2 * points.len());
        let mut net_weights = Vec::with_capacity(2 * points.len());
        for (&point, &weight) in points.iter().zip(weights) {
            net.extend([at(point, -2.0), at(point, 0.0)]);
            net_weights.extend([weight, weight]);
        }
        let mut knots = vec![0.0; degree + 1];
        knots.extend(std::iter::repeat_n(1.0, degree + 1));
        faces.push(
            NurbsSurface::new(
                *degree,
                1,
                knots,
                vec![0.0, 0.0, 1.0, 1.0],
                points.len(),
                2,
                net,
                net_weights,
            )
            .map_err(|e| e.to_string())?,
        );
    }
    Ok(faces)
}

/// The tip over the prism on the polygon through `corners`:
/// [`section_tip`] of [`polygon`].
pub fn prism_tip(
    frame: &Frame,
    k: f64,
    corners: &[[f64; 2]],
    foot: [f64; 2],
    length: f64,
    fullness: f64,
) -> Result<(Tip, Body), String> {
    section_tip(frame, k, &polygon(corners), foot, length, fullness)
}

/// The tip over the prism on `section` (in the xy plane of `frame`,
/// times `k`) that stands `length` (times `k`) over `foot`, and the
/// solid between its faces and the plane: one face per piece of the
/// section, bounded by its rim curve and two rails, and the flat base.
/// The faces come first, in the order of the pieces, then the base.
pub fn section_tip(
    frame: &Frame,
    k: f64,
    section: &[Piece],
    foot: [f64; 2],
    length: f64,
    fullness: f64,
) -> Result<(Tip, Body), String> {
    let rims = sides(frame, k, section)?
        .iter()
        .map(|side| TipRim::of_border(side, Border::VEnd).map_err(|e| format!("{e:?}")))
        .collect::<Result<Vec<_>, _>>()?;
    let spec = TipSpec {
        tip: frame.point(foot[0] * k, foot[1] * k, length * k),
        normal: frame.z(),
        fullness,
        tolerance: 1e-9 * k,
        angular: 1e-9,
    };
    let tip = tip_nets(&rims, &spec).map_err(|e| format!("{e:?}"))?;
    let body = pod(&tip, frame)?;
    Ok((tip, body))
}

/// The solid between a tip's faces and the plane of `frame` its rim
/// lies in.
pub fn pod(tip: &Tip, frame: &Frame) -> Result<Body, String> {
    let forward = |edge: EdgeId| Coedge {
        edge,
        reversed: false,
    };
    let backward = |edge: EdgeId| Coedge {
        edge,
        reversed: true,
    };
    let mut body = Body::new(BodyKind::Solid);
    let count = tip.faces.len();
    if count == 0 || tip.rims.len() != count || tip.rails.len() != count {
        return Err("a tip without faces".to_string());
    }
    let mut corners = Vec::with_capacity(count);
    for rim in &tip.rims {
        let first: Vec3 = *rim.control_points().first().ok_or("an empty rim curve")?;
        corners.push(body.add_vertex(first));
    }
    let top = body.add_vertex(tip.tip);
    let corner = |k: usize| corners.get(k % count).copied().ok_or("no such corner");
    let mut rims = Vec::with_capacity(count);
    let mut rails = Vec::with_capacity(count);
    for (k, (rim, rail)) in tip.rims.iter().zip(&tip.rails).enumerate() {
        rims.push(body.add_edge(
            Curve3::Nurbs(rim.clone()),
            rim.domain(),
            corner(k)?,
            corner(k + 1)?,
        ));
        rails.push(body.add_edge(Curve3::Nurbs(rail.clone()), (0.0, 1.0), corner(k)?, top));
    }
    let rim = |k: usize| rims.get(k % count).copied().ok_or("no such rim");
    let rail = |k: usize| rails.get(k % count).copied().ok_or("no such rail");
    let mut faces = Vec::with_capacity(count + 1);
    for (k, face) in tip.faces.iter().enumerate() {
        // Round the face counter-clockwise seen from outside: along the
        // rim, up the next rail, down its own; the other way round where
        // the faces' own normals point inward.
        let coedges = if tip.counter_clockwise {
            vec![forward(rim(k)?), forward(rail(k + 1)?), backward(rail(k)?)]
        } else {
            vec![forward(rail(k)?), backward(rail(k + 1)?), backward(rim(k)?)]
        };
        let outline = body.add_loop(coedges);
        faces.push(body.add_face(
            Surface::Nurbs(face.clone()),
            !tip.counter_clockwise,
            vec![outline],
        ));
    }
    let down = frame
        .reversed_at(frame.origin())
        .ok_or("a frame that cannot be turned over")?;
    let mut coedges = Vec::with_capacity(count);
    for k in 0..count {
        coedges.push(if tip.counter_clockwise {
            backward(rim(count - 1 - k)?)
        } else {
            forward(rim(k)?)
        });
    }
    let outline = body.add_loop(coedges);
    faces.push(body.add_face(Surface::Plane(Plane::new(down)), false, vec![outline]));
    body.add_shell(faces);
    Ok(body)
}

/// A solid prism on `section` (counter-clockwise, in the xy plane of
/// `frame`, times `k`) from `depth` (times `k`) behind the plane up to
/// it, its sides spline surfaces (each piece's own degree along, 1 up),
/// so that its flat face in the plane, the *end*, is a face a tip takes:
/// the sides first, in the order of the pieces, then the base, then the
/// end. The end's plane is `frame`'s xy plane, its outward normal
/// `frame`'s z.
pub fn spline_prism(frame: &Frame, k: f64, section: &[Piece], depth: f64) -> Result<Body, String> {
    spline_frustum(frame, k, section, depth, 1.0)
}

/// [`spline_prism`] with the section at the end scaled by `scale` about
/// the frame's origin (the base keeps it as it is): below 1 the sides
/// lean in and would meet `depth · scale / (1 − scale)` beyond the end.
pub fn spline_frustum(
    frame: &Frame,
    k: f64,
    section: &[Piece],
    depth: f64,
    scale: f64,
) -> Result<Body, String> {
    let count = section.len();
    if count < 2 {
        return Err("a prism needs two pieces at least".to_string());
    }
    let at = |[x, y]: [f64; 2], z: f64| {
        let s = if z == 0.0 { scale } else { 1.0 };
        frame.point(x * k * s, y * k * s, z * k)
    };
    let clamped = |degree: usize| {
        let mut knots = vec![0.0; degree + 1];
        knots.extend(std::iter::repeat_n(1.0, degree + 1));
        knots
    };
    let mut body = Body::new(BodyKind::Solid);
    let mut low = Vec::with_capacity(count);
    let mut high = Vec::with_capacity(count);
    for (_, points, _) in section {
        let first = *points.first().ok_or("an empty piece")?;
        low.push(body.add_vertex(at(first, -depth)));
        high.push(body.add_vertex(at(first, 0.0)));
    }
    let corner = |list: &Vec<craft3d_topo::VertexId>, k: usize| {
        list.get(k % count).copied().ok_or("no such corner")
    };
    let mut bottoms = Vec::with_capacity(count);
    let mut tops = Vec::with_capacity(count);
    let mut laterals = Vec::with_capacity(count);
    let mut surfaces = Vec::with_capacity(count);
    for (index, (degree, points, weights)) in section.iter().enumerate() {
        let row = |z: f64| points.iter().map(|&p| at(p, z)).collect::<Vec<_>>();
        let curve = |z: f64| {
            NurbsCurve::new(*degree, clamped(*degree), row(z), weights.clone())
                .map_err(|e| e.to_string())
        };
        let (bottom, top) = (curve(-depth)?, curve(0.0)?);
        bottoms.push(body.add_edge(
            Curve3::Nurbs(bottom.clone()),
            bottom.domain(),
            corner(&low, index)?,
            corner(&low, index + 1)?,
        ));
        tops.push(body.add_edge(
            Curve3::Nurbs(top.clone()),
            top.domain(),
            corner(&high, index)?,
            corner(&high, index + 1)?,
        ));
        let first = *points.first().ok_or("an empty piece")?;
        let (a, b) = (at(first, -depth), at(first, 0.0));
        let line = Line::through(a, b).map_err(|e| e.to_string())?;
        laterals.push(body.add_edge(
            Curve3::Line(line),
            (0.0, a.distance(b)),
            corner(&low, index)?,
            corner(&high, index)?,
        ));
        let mut net = Vec::with_capacity(2 * points.len());
        let mut net_weights = Vec::with_capacity(2 * points.len());
        for (&point, &weight) in points.iter().zip(weights) {
            net.extend([at(point, -depth), at(point, 0.0)]);
            net_weights.extend([weight, weight]);
        }
        surfaces.push(
            NurbsSurface::new(
                *degree,
                1,
                clamped(*degree),
                clamped(1),
                points.len(),
                2,
                net,
                net_weights,
            )
            .map_err(|e| e.to_string())?,
        );
    }
    let forward = |edge: EdgeId| Coedge {
        edge,
        reversed: false,
    };
    let backward = |edge: EdgeId| Coedge {
        edge,
        reversed: true,
    };
    let pick = |list: &Vec<EdgeId>, k: usize| list.get(k % count).copied().ok_or("no such edge");
    let mut faces = Vec::with_capacity(count + 2);
    for (index, surface) in surfaces.into_iter().enumerate() {
        let outline = body.add_loop(vec![
            forward(pick(&bottoms, index)?),
            forward(pick(&laterals, index + 1)?),
            backward(pick(&tops, index)?),
            backward(pick(&laterals, index)?),
        ]);
        faces.push(body.add_face(Surface::Nurbs(surface), false, vec![outline]));
    }
    let mut base = Vec::with_capacity(count);
    for index in (0..count).rev() {
        base.push(backward(pick(&bottoms, index)?));
    }
    let outline = body.add_loop(base);
    let down = frame
        .reversed_at(frame.point(0.0, 0.0, -depth * k))
        .ok_or("a frame that cannot be turned over")?;
    faces.push(body.add_face(Surface::Plane(Plane::new(down)), false, vec![outline]));
    let mut end = Vec::with_capacity(count);
    for index in 0..count {
        end.push(forward(pick(&tops, index)?));
    }
    let outline = body.add_loop(end);
    faces.push(body.add_face(Surface::Plane(Plane::new(*frame)), false, vec![outline]));
    body.add_shell(faces);
    Ok(body)
}
