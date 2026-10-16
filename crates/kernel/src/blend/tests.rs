//! The two constructions held against each other on the same edges: a
//! class the migration routes in place (`in_place`, built by `construct`)
//! gives the solid the tools and the boolean give (`build`), for the same
//! requests. Each is an independent reference for the other: they share
//! the plans (the section in closed form) and nothing of the construction.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::boolean::{BooleanOp, boolean};
use craft3d_topo::{check, mass_properties};

const SCALES: [f64; 3] = [1e-3, 1.0, 1e3];

fn tolerance(k: f64) -> Tolerance {
    Tolerance::new(1e-6 * k, 1e-9).unwrap()
}

/// The straight edge whose middle is nearest `p`.
fn edge_at(body: &Body, p: Vec3) -> EdgeId {
    let (index, _) = body
        .edges
        .iter()
        .enumerate()
        .map(|(i, e)| (i, e.curve.eval(0.5 * (e.range.0 + e.range.1)).distance(p)))
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap();
    EdgeId(u32::try_from(index).unwrap())
}

fn length_of_edges(body: &Body) -> f64 {
    body.edges
        .iter()
        .map(|e| {
            (0..64)
                .map(|j| {
                    let t = |i: u32| e.range.0 + (e.range.1 - e.range.0) * f64::from(i) / 64.0;
                    e.curve.eval(t(j)).distance(e.curve.eval(t(j + 1)))
                })
                .sum::<f64>()
        })
        .sum()
}

/// The foot of a boss standing `d` behind a wall's face, whose cylinder's
/// seam lies at `turn` about its axis (engine-block finding E2): both
/// root edges rounded or chamfered in place and by the tools, the same
/// solid within the tolerance (volume within the tolerance times the
/// area, area within it times the length of the edges, the same counts),
/// whatever the seam's angle.
#[test]
fn a_boss_root_in_place_is_the_tools_result_at_every_seam_angle() {
    let (big, d, height) = (1.0_f64, 0.3_f64, 2.0_f64);
    let ex = (big * big - d * d).sqrt();
    let frame = Frame::new(
        Vec3::new(0.3, -0.2, 0.5),
        Vec3::new(0.2, 0.3, 1.0),
        Vec3::new(1.0, 0.0, -0.2),
    )
    .unwrap();
    let sizes = [
        Size::Radius(0.5),
        Size::Chamfer(ChamferSize::Distances {
            first: 0.4,
            second: 0.4,
        }),
        Size::Chamfer(ChamferSize::Distances {
            first: 0.25,
            second: 0.45,
        }),
    ];
    for k in SCALES {
        let tol = tolerance(k);
        let at = |x: f64, y: f64, z: f64| frame.point(x * k, y * k, z * k);
        for degrees in [0.0_f64, 45.0, 90.0, 180.0, 270.0] {
            let turn = degrees.to_radians();
            let wall_frame = frame.at(at(-4.0, -3.0, 0.0)).unwrap();
            let wall = primitives::block(&wall_frame, 8.0 * k, 3.0 * k, height * k).unwrap();
            let boss_frame = Frame::new(
                at(0.0, d, 0.0),
                frame.z(),
                frame.vector(turn.cos(), turn.sin(), 0.0),
            )
            .unwrap();
            let boss = primitives::cylinder(&boss_frame, big * k, height * k).unwrap();
            let body = boolean(&wall, &boss, BooleanOp::Union, &tol).unwrap().body;
            let edges = [
                edge_at(&body, at(ex, 0.0, 0.5 * height)),
                edge_at(&body, at(-ex, 0.0, 0.5 * height)),
            ];
            for size in sizes {
                let size = match size {
                    Size::Radius(r) => Size::Radius(r * k),
                    Size::Chamfer(ChamferSize::Distances { first, second }) => {
                        Size::Chamfer(ChamferSize::Distances {
                            first: first * k,
                            second: second * k,
                        })
                    }
                    other => other,
                };
                let requests: Vec<Request> = edges
                    .iter()
                    .map(|edge| Request {
                        edge: *edge,
                        first: None,
                        size,
                    })
                    .collect();
                let uses = edge_uses(&body);
                let at_vertex = vertex_edges(&body);
                let plans: Vec<Plan> = requests
                    .iter()
                    .map(|r| plan(&body, &uses, r, tol.linear).unwrap())
                    .collect();
                let routed = in_place(&body, &uses, &at_vertex, &requests, &plans);
                assert_eq!(routed, [0, 1], "both edges go in place");
                let what = format!("scale {k}, seam at {degrees}°, {size:?}");
                let rolled = construct(
                    &body,
                    &uses,
                    &at_vertex,
                    &requests,
                    plans.clone(),
                    &routed,
                    &tol,
                )
                .unwrap_or_else(|f| panic!("{what}: in place: {:?}", f.error));
                let tools = build(&body, &uses, &at_vertex, plans, &tol)
                    .unwrap_or_else(|f| panic!("{what}: tools: {:?}", f.error));
                assert_eq!(check(&rolled.body, &tol), [], "{what}");
                let (a, b) = (
                    mass_properties(&rolled.body).unwrap(),
                    mass_properties(&tools.body).unwrap(),
                );
                let edges_long = length_of_edges(&tools.body);
                assert!(
                    (a.volume - b.volume).abs() <= tol.linear * b.area,
                    "{what}: volume {} against {}",
                    a.volume,
                    b.volume
                );
                assert!(
                    (a.area - b.area).abs() <= 2.0 * tol.linear * edges_long,
                    "{what}: area {} against {}",
                    a.area,
                    b.area
                );
                assert_eq!(
                    (
                        rolled.body.faces.len(),
                        rolled.body.edges.len(),
                        rolled.body.vertices.len()
                    ),
                    (
                        tools.body.faces.len(),
                        tools.body.edges.len(),
                        tools.body.vertices.len()
                    ),
                    "{what}: counts"
                );
            }
        }
    }
}
