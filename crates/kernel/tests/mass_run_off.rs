//! Faces whose fitted edges run off the domain of their spline surface:
//! the side of a lofted blade whose leading edge and tip are rounded
//! (three bodies of `crates/engine/tests/blend_findings/meet-le-tip-blade.json`,
//! face 1 of each, as a sheet; docs/design/topology.md, finding of
//! 2026-10-15 *A fitted edge that runs off its surface*).
//!
//! The fillets' contact edges are fitted within the tolerance of the
//! blade's side and end on its border `v = 1`, tangent to it, up to
//! 8e-7 off the surface: over their last half percent the foot of their
//! points is held at the border, and the parameters along the edge have a
//! kink there. With a panel across it the boundary integral never settled
//! (to 1e-12 and 3e-11 of a piece's size from one doubling to the next,
//! up to the seventh), and each face was refused as one that does not
//! settle. With the panels ending at the kink they settle at the second
//! sum.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_geom::{Curve3, Line, NurbsCurve, NurbsSurface, Surface};
use craft3d_math::{Transform, Vec3};
use craft3d_tess::{TessParams, tessellate_body};
use craft3d_topo::{Body, BodyKind, Coedge, EdgeId, VertexId, mass_properties};
use serde_json::Value;

/// The area of each face, the region its edges' feet on its surface bound,
/// by another route than the measure's (2026-11-14, the review's form):
/// Green's theorem along a polyline of the feet, 1000 and 2000 points per
/// edge (each the nearest point of the edge's point, `Surface::project`),
/// with one more step at each end of an edge that lies past a border, to
/// the end's foot on the surface continued past it; the integral of
/// `|Sᵤ × Sᵥ|` across to it by Gauss–Legendre with eight nodes on four
/// panels per knot span; the parameters' differences in place of rates;
/// extrapolated (Richardson, the error of a midpoint polyline being of
/// second order; from 2000/4000 the same to 3e-12). Edge by edge, as the
/// measure sums them: the steps between two edges' feet at a vertex are
/// not part of either (docs/design/topology.md › *The rates of the feet*,
/// open). Recomputed in [`the_region_of_the_feet_integrated_another_way`].
///
/// Why the steps at the ends: the measure follows an edge past a border
/// on the surface continued (its rates are the feet's there), so an edge
/// that starts past a border starts there. The first face's edge 4 starts
/// 5.6e-10 in u (about 1e-9 mm) past the border u = 0, where the edge
/// before it ends on that border; without that step the reference was
/// 1.2e-8 below the measure on the first face (the form of this reference
/// first pinned, at 1e-9 there).
///
/// Until 2026-11-14 the faces' areas were pinned to the measure's own slow
/// sums (`22.343_815_774_683_396`, `32.100_614_427_099_89`,
/// `22.347_896_070_048_57`: the seventh doubling of a build without the
/// convergence test), at 1e-11. Those sums took the rates of a sample off
/// its surface (these edges lie up to 1.1e-6 off it) from the
/// least-squares solve, which is the foot's rate only on the surface; they
/// are 2.2e-9, 7.5e-9 and 1.4e-9 below these areas. The measure now takes
/// the feet's rates (`topo` › `mass::foot_rates`) and is 1.8e-12 above,
/// 3.2e-11 and 9.7e-12 below them.
const FEET: [f64; 3] = [
    22.343_815_776_860_68,
    32.100_614_434_649_52,
    22.347_896_071_456_404,
];

/// How near each face's measure is held to [`FEET`], as a share of it:
/// 1e-11 (1.0e-12 at the most, measured).
const NEAR: f64 = 1e-11;

const FACES: [&str; 3] = [
    include_str!("fixtures/mass_run_off/side-1.json"),
    include_str!("fixtures/mass_run_off/side-2.json"),
    include_str!("fixtures/mass_run_off/side-3.json"),
];

fn vec3(v: &Value) -> Vec3 {
    Vec3::new(
        v[0].as_f64().unwrap(),
        v[1].as_f64().unwrap(),
        v[2].as_f64().unwrap(),
    )
}

fn floats(v: &Value) -> Vec<f64> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_f64().unwrap())
        .collect()
}

fn points(v: &Value) -> Vec<Vec3> {
    v.as_array().unwrap().iter().map(vec3).collect()
}

fn index(v: &Value) -> usize {
    usize::try_from(v.as_u64().unwrap()).unwrap()
}

fn id(v: &Value) -> u32 {
    u32::try_from(v.as_u64().unwrap()).unwrap()
}

/// The sheet of one face: a NURBS surface bounded by lines and NURBS
/// curves, every number as it was in the body (shortest round-trip form).
fn sheet(json: &str) -> Body {
    let v: Value = serde_json::from_str(json).unwrap();
    let mut body = Body::new(BodyKind::Sheet);
    for p in v["vertices"].as_array().unwrap() {
        body.add_vertex(vec3(p));
    }
    for e in v["edges"].as_array().unwrap() {
        let c = &e["curve"];
        let curve = if let Some(l) = c.get("line") {
            Curve3::Line(Line::new(vec3(&l["o"]), vec3(&l["d"])).unwrap())
        } else {
            let n = &c["nurbs"];
            Curve3::Nurbs(
                NurbsCurve::new(
                    index(&n["degree"]),
                    floats(&n["knots"]),
                    points(&n["ctrl"]),
                    floats(&n["weights"]),
                )
                .unwrap(),
            )
        };
        let range = (
            e["range"][0].as_f64().unwrap(),
            e["range"][1].as_f64().unwrap(),
        );
        body.add_edge(
            curve,
            range,
            VertexId(id(&e["start"])),
            VertexId(id(&e["end"])),
        );
    }
    let mut loops = Vec::new();
    for l in v["loops"].as_array().unwrap() {
        let coedges = l
            .as_array()
            .unwrap()
            .iter()
            .map(|c| Coedge {
                edge: EdgeId(id(&c[0])),
                reversed: c[1].as_bool().unwrap(),
            })
            .collect();
        loops.push(body.add_loop(coedges));
    }
    let f = &v["faces"][0];
    let n = &f["surface"]["nurbs"];
    let surface = Surface::Nurbs(
        NurbsSurface::new(
            index(&n["degrees"][0]),
            index(&n["degrees"][1]),
            floats(&n["knots_u"]),
            floats(&n["knots_v"]),
            index(&n["counts"][0]),
            index(&n["counts"][1]),
            points(&n["ctrl"]),
            floats(&n["weights"]),
        )
        .unwrap(),
    );
    let face = body.add_face(surface, f["reversed"].as_bool().unwrap(), loops);
    body.add_shell(vec![face]);
    body
}

/// Each face is measured: its area is that of the region its edges' feet
/// bound, integrated the other way ([`FEET`], within [`NEAR`]), and that
/// of its mesh to what the mesh is off by.
#[test]
fn faces_whose_edges_run_off_their_surface_are_measured() {
    for (k, json) in FACES.iter().enumerate() {
        let body = sheet(json);
        let area = mass_properties(&body)
            .unwrap_or_else(|e| panic!("face {k}: {e}"))
            .area;
        let feet = FEET[k];
        assert!(
            (area - feet).abs() <= NEAR * feet,
            "face {k}: {area} against the region of the feet, {feet}"
        );
        let chord = 1e-4;
        let params = TessParams {
            chordal_tolerance: chord,
            ..TessParams::default()
        };
        let mesh = tessellate_body(&body, &params).unwrap();
        // A chord `c` short of an arc of radius r cuts off a share of
        // about c / r of the area; the blade's side curves with a few
        // millimetres at the least.
        let off = (mesh.area() - area).abs() / area;
        assert!(off < 1e-4, "face {k}: mesh {} against {area}", mesh.area());
    }
}

/// The measure does not depend on where the face is: turned and moved a
/// thousand units away, each face has the same area to 1e-12. A sum that
/// only settles by chance does not hold that (two of the three were
/// refused as they stood, one only where it stood in its body).
#[test]
fn faces_whose_edges_run_off_their_surface_measure_alike_anywhere() {
    let turn = Transform::rotation(Vec3::new(1.0, 2.0, 3.0), 0.7)
        .unwrap()
        .then(&Transform::translation(Vec3::new(1e3, -2e3, 5e2)));
    for (k, json) in FACES.iter().enumerate() {
        let body = sheet(json);
        let here = mass_properties(&body).unwrap().area;
        let moved = body.transformed(&turn).unwrap();
        let there = mass_properties(&moved)
            .unwrap_or_else(|e| panic!("face {k} moved: {e}"))
            .area;
        assert!(
            (here - there).abs() <= 1e-12 * here,
            "face {k}: {here} against {there}"
        );
    }
}

/// Gauss–Legendre, eight nodes on [0, 1].
const GAUSS_8: [(f64, f64); 8] = [
    (0.019_855_071_751_231_856, 0.050_614_268_145_188_13),
    (0.101_666_761_293_186_64, 0.111_190_517_226_687_24),
    (0.237_233_795_041_835_5, 0.156_853_322_938_943_63),
    (0.408_282_678_752_175_1, 0.181_341_891_689_181),
    (0.591_717_321_247_825, 0.181_341_891_689_181),
    (0.762_766_204_958_164_5, 0.156_853_322_938_943_63),
    (0.898_333_238_706_813_4, 0.111_190_517_226_687_24),
    (0.980_144_928_248_768_1, 0.050_614_268_145_188_13),
];

/// `∫ |Sᵤ × Sᵥ| (u, t) dt` from the start of `v` to `v`, split at the knots
/// `breaks` (the first is the start), four panels of eight nodes each.
fn across(s: &NurbsSurface, breaks: &[f64], u: f64, v: f64) -> f64 {
    let mut cuts = vec![breaks[0]];
    cuts.extend(breaks.iter().skip(1).copied().filter(|b| *b < v));
    cuts.push(v);
    let mut sum = 0.0;
    for w in cuts.windows(2) {
        for k in 0..4 {
            let (p, q) = (
                w[0] + (w[1] - w[0]) * f64::from(k) / 4.0,
                w[0] + (w[1] - w[0]) * f64::from(k + 1) / 4.0,
            );
            for (x, weight) in GAUSS_8 {
                let (_, su, sv) = s.derivatives(u, p + (q - p) * x);
                sum += su.cross(sv).length() * weight * (q - p);
            }
        }
    }
    sum
}

/// The area of the region the feet of the face's edges bound, edge by
/// edge, by Green's theorem along a polyline of `n` feet per edge (see
/// [`FEET`]): each the nearest point of the edge's point on the face's
/// surface; at each end of an edge that lies past a border, one more
/// step, between that foot and the end's foot on the surface continued a
/// ten-thousandth of its parameters past every border (where the measure,
/// which follows the feet's rates, puts the end).
fn feet_area(body: &Body, n: u32) -> f64 {
    let face = &body.faces[0];
    let Surface::Nurbs(s) = &face.surface else {
        panic!("a NURBS face")
    };
    let ((u0, u1), (v0, v1)) = s.domain();
    let reach = 1e-4;
    let wider = s
        .extended(
            (u0 - reach * (u1 - u0), u1 + reach * (u1 - u0)),
            (v0 - reach * (v1 - v0), v1 + reach * (v1 - v0)),
            1e-12,
        )
        .unwrap();
    let mut breaks: Vec<f64> = vec![v0];
    for k in s.knots_v() {
        if *k > v0 && *k < v1 && breaks.last() != Some(k) {
            breaks.push(*k);
        }
    }
    let continued = Surface::Nurbs(wider.clone());
    let mut total = 0.0;
    for l in &face.loops {
        for c in &body.loop_(*l).unwrap().coedges {
            let e = body.edge(c.edge).unwrap();
            let (t0, t1) = e.range;
            let mut feet: Vec<(f64, f64)> = (0..=n)
                .map(|k| {
                    let t = t0 + (t1 - t0) * f64::from(k) / f64::from(n);
                    face.surface.project(e.curve.eval(t)).unwrap()
                })
                .collect();
            feet.insert(0, continued.project(e.curve.eval(t0)).unwrap());
            feet.push(continued.project(e.curve.eval(t1)).unwrap());
            if c.reversed {
                feet.reverse();
            }
            for w in feet.windows(2) {
                let (a, b) = (w[0], w[1]);
                if b.0 != a.0 {
                    total -=
                        across(&wider, &breaks, 0.5 * (a.0 + b.0), 0.5 * (a.1 + b.1)) * (b.0 - a.0);
                }
            }
        }
    }
    total.abs()
}

/// [`FEET`] worked out again: the region of the feet integrated by
/// Green's theorem along polylines of 1000 and 2000 feet per edge,
/// extrapolated, within 1e-12 of the constant (in the gate: about a
/// second and a half per face).
#[test]
fn the_region_of_the_feet_integrated_another_way() {
    for (k, json) in FACES.iter().enumerate() {
        let body = sheet(json);
        let [a1, a2] = [1000, 2000].map(|n| feet_area(&body, n));
        let feet = (4.0 * a2 - a1) / 3.0;

        assert!(
            (feet - FEET[k]).abs() <= 1e-12 * FEET[k],
            "face {k}: {feet} against {}",
            FEET[k]
        );
    }
}
