//! Review of `check/fade-tip` (review-fade-law), not for `main`: the
//! hull's stem (`plan/loft-tip/bow/out/trimmed.step`, tolerance 0.02,
//! copied into `review_fade_docs`) rounded 3 and faded to nothing over 15
//! towards its foot, rebuilt here: which `back` builds, each body held to
//! validity, the deep check's search and the measure, its fillet face to
//! the review's own oracle (`review_fade_common`), and pictures printed
//! as evidence lines (`EV <name> <i> <base64>`).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss
)]

#[path = "../../kernel/tests/boolean_common/mod.rs"]
mod boolean_common;
#[path = "../../kernel/tests/review_fade_common/mod.rs"]
mod review_fade_common;

use craft3d_kernel::fillet::{Fade, FilletFace, FilletLawEdge, RadiusLaw, fillet_by_law};
use craft3d_math::{Bbox3, Tolerance, Vec3};
use craft3d_render::{Camera, Highlight, RenderParams, render};
use craft3d_stepread::{Options, parse};
use craft3d_tess::{SelfCheckParams, TessParams, body_self_intersections, tessellate_body};
use craft3d_topo::{Body, EdgeId, check, mass_properties};
use review_fade_common::{ByLength, Law, Oracle, faces_of};

fn load() -> (Body, Tolerance) {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/review_fade_docs/trimmed.step"
    );
    let bytes = std::fs::read(path).unwrap();
    let tolerance = Tolerance::new(0.02, 1e-9).unwrap();
    let options = Options {
        metres_per_unit: 1e-3,
        tolerance,
        loosest: 1000.0 * tolerance.linear,
    };
    (
        parse(&bytes).unwrap().build(0, &options).unwrap().body,
        tolerance,
    )
}

/// The stem: of the edges that end at the foot (5, 0, −8.9), the one that
/// rises (the keel runs aft, low). Whether the foot is its start.
fn stem(body: &Body) -> (EdgeId, bool) {
    let foot = Vec3::new(5.0, 0.0, -8.9);
    let mut found = None;
    for (i, e) in body.edges.iter().enumerate() {
        let (a, b) = (
            body.vertices[e.start.0 as usize].point,
            body.vertices[e.end.0 as usize].point,
        );
        for (here, there, at_start) in [(a, b, true), (b, a, false)] {
            if here.distance(foot) < 0.5 && there.z > here.z + 20.0 {
                found = Some((EdgeId(i as u32), at_start));
            }
        }
    }
    found.unwrap()
}

fn picture(name: &str, body: &Body, fillet: Option<usize>, focus: Bbox3, toward: Vec3) {
    let mesh = tessellate_body(
        body,
        &TessParams {
            chordal_tolerance: 0.01,
            max_angle: Some(4.0_f64.to_radians()),
            ..TessParams::default()
        },
    )
    .unwrap();
    let whole = Bbox3::from_points(mesh.positions.iter().copied());
    let camera = Camera::framing_clear_of(
        &focus,
        &whole,
        toward,
        Vec3::new(0.0, 0.0, 1.0),
        30.0,
        4.0 / 3.0,
    )
    .unwrap();
    let highlight = match fillet {
        Some(face) => Highlight::new([230, 150, 60], [face as u32]).0,
        None => Highlight::NONE,
    };
    let frame = render(
        &mesh,
        &camera,
        &RenderParams {
            width: 1000,
            height: 750,
            ssaa: 2,
            background: [250, 250, 250],
            color: [150, 170, 200],
            highlight,
            ..RenderParams::default()
        },
    )
    .unwrap();
    craft3d_testkit::evidence::print(name, &frame.to_png().unwrap());
}

/// The finding: on the hull's stem a fade over little more than its
/// radius (3 over 5, back 0.5) is refused in the post-check's words,
/// "internal error (a bug in 3DCraft …)": input reaches it. It must be
/// built, or refused with a reason one can act on.
#[test]
fn a_short_fade_on_the_stem_is_not_called_a_bug() {
    let (body, tol) = load();
    let (edge, foot_at_start) = stem(&body);
    for (radius, over, back) in [(3.0, 5.0, 0.5), (5.0, 5.0, 1.0), (5.0, 10.0, 0.5)] {
        let fade = Some(Fade::tip(over, back));
        let law = RadiusLaw {
            radius,
            fades: if foot_at_start {
                [fade, None]
            } else {
                [None, fade]
            },
        };
        if let Err(e) = fillet_by_law(&body, &[FilletLawEdge { edge, law }], &tol) {
            let said = e.to_string();
            println!("HULL radius {radius} over {over} back {back}: {said}");
            assert!(
                !said.contains("internal error"),
                "radius {radius} over {over} back {back}: {said}"
            );
        }
    }
}

#[test]
fn the_stem_faded_towards_its_foot() {
    let (body, tol) = load();
    let (edge, foot_at_start) = stem(&body);
    let e = body.edge(edge).unwrap().clone();
    let by_length = ByLength::new(&body, edge);
    let length = by_length.length();
    let foot = body.vertices[if foot_at_start { e.start } else { e.end }.0 as usize].point;
    println!(
        "HULL stem edge {} {:.3} long, the foot at its {}; body {} faces, defects {:?}",
        edge.0,
        length,
        if foot_at_start { "start" } else { "end" },
        body.faces.len(),
        check(&body, &tol)
    );
    let before = mass_properties(&body).unwrap().volume;
    let halves = faces_of(&body, edge);
    let focus = Bbox3::from_corners(
        foot + Vec3::new(-8.0, -12.0, -6.0),
        foot + Vec3::new(30.0, 12.0, 28.0),
    );
    let toward = Vec3::new(-1.0, -0.8, 0.35);
    picture("hull-before", &body, None, focus, toward);
    let mut drawn = 0;
    for back in [0.0, 0.2, 0.33, 0.5, 1.0, 2.0, 3.0, 5.0] {
        let fade = Some(Fade::tip(15.0, back));
        let law = RadiusLaw {
            radius: 3.0,
            fades: if foot_at_start {
                [fade, None]
            } else {
                [None, fade]
            },
        };
        let made = match fillet_by_law(&body, &[FilletLawEdge { edge, law }], &tol) {
            Ok(made) => made,
            Err(e) => {
                println!("HULL back {back}: refused: {e}");
                continue;
            }
        };
        let own = (0..made.faces.len())
            .find(|i| matches!(made.faces[*i], FilletFace::Fillet(_)))
            .unwrap();
        let defects = check(&made.body, &tol);
        let deep = body_self_intersections(&made.body, &SelfCheckParams::new(tol.linear))
            .map(|c| c.is_clean())
            .map_err(|e| e.to_string());
        let after = mass_properties(&made.body).map(|m| m.volume);
        // The oracle: the foot is the start (fade 0) or the end (fade 1).
        let kept = |old: craft3d_topo::FaceId| {
            (0..made.faces.len())
                .find(|i| matches!(&made.faces[*i], FilletFace::Kept(l) if l.contains(&old)))
                .map(|i| craft3d_topo::FaceId(i as u32))
                .unwrap()
        };
        let law = Law {
            radius: 3.0,
            fades: if foot_at_start {
                [Some((0.0, 15.0)), None]
            } else {
                [None, Some((0.0, 15.0))]
            },
            backs: if foot_at_start {
                [back, 0.0]
            } else {
                [0.0, back]
            },
        };
        let oracle = Oracle {
            body: &made.body,
            faces: [kept(halves[0]), kept(halves[1])],
            edge: ByLength::new(&body, edge),
            law,
            side: -1.0,
        };
        let tip_at = if foot_at_start { back } else { length - back };
        let tip = oracle.edge.at(tip_at).0;
        let (_, points) =
            review_fade_common::mesh_points(&made.body, craft3d_topo::FaceId(own as u32), 0.02)
                .unwrap();
        let (mut off, mut outside, mut unplaced, mut n) = (0.0_f64, 0.0_f64, 0, 0);
        let mut worst_at = 0.0;
        for p in points {
            if p.distance(tip) < 1e-4 {
                continue;
            }
            n += 1;
            match oracle.place(p) {
                Some((a, ball, o, out)) => {
                    let o = o.abs() / tol.linear;
                    if o > off {
                        off = o;
                        worst_at = (a - tip_at).abs();
                    }
                    outside = outside.max(out * ball.radius / tol.linear);
                }
                None => unplaced += 1,
            }
        }
        println!(
            "HULL back {back}: built, defects {}, deep check clean {deep:?}, volume {before:.3} -> {after:?}; oracle {n} points: off the ball {off:.3} tolerances (at {worst_at:.3} from the tip), outside the arc {outside:.3}, unplaced {unplaced}",
            defects.len()
        );
        if drawn < 3 && [0.33, 1.0, 3.0].contains(&back) {
            drawn += 1;
            picture(
                &format!("hull-back-{back}"),
                &made.body,
                Some(own),
                focus,
                toward,
            );
            if back == 1.0 {
                let close = Bbox3::from_corners(
                    tip + Vec3::new(-3.0, -5.0, -3.0),
                    tip + Vec3::new(8.0, 5.0, 14.0),
                );
                picture(
                    "hull-tip-close",
                    &made.body,
                    Some(own),
                    close,
                    Vec3::new(-1.0, -0.5, 0.1),
                );
                picture(
                    "hull-tip-below",
                    &made.body,
                    Some(own),
                    close,
                    Vec3::new(-0.6, -0.4, -1.0),
                );
            }
        }
    }
}
