//! What asking freeform faces costs where it costs most, and what it does
//! with faces it was not made for (`craft3d_kernel::tweak::passes`,
//! `docs/design/face-operations.md` › *Freeform faces asked again*).
//!
//! A face operation asks a freeform face whether an edge made again went
//! through it only where the edge comes near one of the face's spans, by
//! a walk of a few evaluations. The dearest case is a body whose freeform
//! walls run within a span of each other for their whole length: every
//! step of every edge is asked about its neighbours. Its counted work
//! (`craft3d_geom::allowance`: the same on every machine) is held to
//! what the operation counted before the freeform faces were asked, and
//! a fifth.

// Test code: a failure in a helper is a failed test.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod boolean_common;
mod hunt_common;

use craft3d_geom::Surface;
use craft3d_kernel::KernelError;
use craft3d_kernel::boolean::{BooleanOp, boolean};
use craft3d_kernel::extrude::extrude;
use craft3d_kernel::offset_face::{FaceOffset, offset_faces};
use craft3d_kernel::primitives;
use craft3d_kernel::profile::{Curve2, SketchCurve, Spline2, detect_profiles};
use craft3d_math::{Frame, Portable, Tolerance, Vec3};
use craft3d_topo::{Body, FaceId, check};
use hunt_common::{crosses_itself, digest, measured, tolerance};

/// The prism of closed outlines in the plane z = 0, `height` high.
fn prism(outlines: Vec<Curve2>, height: f64, tol: &Tolerance) -> Body {
    let curves: Vec<SketchCurve> = outlines
        .into_iter()
        .enumerate()
        .map(|(i, curve)| SketchCurve {
            id: i as u64 + 1,
            curve,
        })
        .collect();
    let profiles = detect_profiles(&curves, tol).unwrap();
    extrude(&profiles, &Frame::WORLD, 0.0, height, tol)
        .unwrap()
        .body
}

fn line(start: [f64; 2], end: [f64; 2]) -> Curve2 {
    Curve2::Line { start, end }
}

/// A clamped cubic of `spans` spans from (`x`, 5) to (`x`, 55) that bows
/// out by `bow` and whose control points wave by 0.4 from one to the next
/// (the curve itself by 0.13); `back`: run from the far end.
fn wavy(x: f64, bow: f64, spans: usize, back: bool) -> Curve2 {
    let count = spans + 3;
    let mut control: Vec<[f64; 2]> = (0..count)
        .map(|i| {
            let along = i as f64 / (count - 1) as f64;
            let aside = if i == 0 || i == count - 1 {
                0.0
            } else {
                bow + if i % 2 == 0 { 0.4 } else { -0.4 }
            };
            [x + aside, 5.0 + 50.0 * along]
        })
        .collect();
    if back {
        control.reverse();
    }
    let mut knots = vec![0.0; 4];
    knots.extend((1..spans).map(|i| i as f64 / spans as f64));
    knots.extend([1.0; 4]);
    Curve2::Spline(Spline2 {
        degree: 3,
        knots,
        control,
    })
}

/// A plate 60 deep and 5 high with `slots` slots through it, each between
/// two wavy splines of `spans` spans that bow 1.5 apart and meet at their
/// ends, `pitch` from one slot to the next: at a pitch of 3.3 the web
/// between two slots is 0.3 thick along its whole length (the walls wave
/// together), and each wall runs inside the boxes of the spans of the
/// wall across the web.
fn webbed(slots: usize, spans: usize, pitch: f64, tol: &Tolerance) -> Body {
    let wide = pitch * slots as f64 + 20.0;
    let mut outlines = vec![
        line([0.0, 0.0], [wide, 0.0]),
        line([wide, 0.0], [wide, 60.0]),
        line([wide, 60.0], [0.0, 60.0]),
        line([0.0, 60.0], [0.0, 0.0]),
    ];
    for i in 0..slots {
        let x = 10.0 + pitch * (i as f64 + 0.5);
        outlines.push(wavy(x, 1.5, spans, false));
        outlines.push(wavy(x, -1.5, spans, true));
    }
    prism(outlines, 5.0, tol)
}

/// The flat face of `body` square to z at the height `at`.
fn level(body: &Body, at: f64) -> FaceId {
    let found: Vec<FaceId> = (0..body.faces.len() as u32)
        .map(FaceId)
        .filter(|f| match &body.face(*f).unwrap().surface {
            Surface::Plane(p) => {
                p.frame().z().z.abs() > 0.99 && (p.frame().origin().z - at).abs() < 1e-6
            }
            _ => false,
        })
        .collect();
    assert_eq!(found.len(), 1, "one flat face at {at}");
    found[0]
}

fn offset(body: &Body, face: FaceId, distance: f64, tol: &Tolerance) -> Result<Body, KernelError> {
    offset_faces(body, &[FaceOffset { face, distance }], tol).map(|made| made.body)
}

/// Holds a body built to the checker, the self-check and a volume.
fn sound(what: &str, body: &Body, volume: f64, tol: &Tolerance) {
    assert!(check(body, tol).is_empty(), "{what}: invalid");
    assert!(!crosses_itself(body, tol), "{what}: passes through itself");
    let m = measured(body).unwrap();
    let allowed = m.bound + 2.0 * tol.linear * m.area + 1e-9 * volume.abs();
    assert!(
        (m.volume - volume).abs() <= allowed,
        "{what}: volume {} for {volume}",
        m.volume
    );
}

/// What the top of the webbed plate of twelve slots, offset by 1, counted
/// on the commit before freeform faces were asked (1d22524a, 2026-11-15).
/// To take it again after a change that moves it for another reason: the
/// test prints what it counts; run it on the commit before that change
/// and on the one after, and say in the commit what moved it.
const WEBBED_BEFORE: u64 = 8_298_299;

/// Twelve wavy slots 0.3 apart, the plate's top offset outward by 1. Each
/// of the 24 walls' top edges is made again, and every step of each lies
/// within the span boxes of the wall across its web, and of the walls
/// next to that one: each is asked about them, by walks. Built, and the
/// plate a fifth higher; and the whole operation counts no more than a
/// fifth over what it counted with no freeform face asked.
#[test]
fn walls_within_a_span_of_each_other_are_asked_within_a_fifth_of_the_operation() {
    let tol = tolerance(1.0);
    let body = webbed(12, 16, 3.3, &tol);
    let volume = measured(&body).unwrap().volume;
    let top = level(&body, 5.0);
    let (made, work) = craft3d_geom::allowance::measured(|| offset(&body, top, 1.0, &tol));
    println!("the webbed plate's top offset: {work} units");
    let made = made.unwrap();
    sound("the webbed plate", &made, volume * 6.0 / 5.0, &tol);
    assert!(
        work <= WEBBED_BEFORE + WEBBED_BEFORE / 5,
        "{work} units for {WEBBED_BEFORE} before"
    );
}

/// A block 30 x 20, 40 long, whose top between x 5 and x 25 is a spline
/// wall of 3000 spans that ripples six times between 19 and 21 (a face of
/// 3000 spans), with a round pocket of radius 3 from its bottom up to 12
/// under it. The pocket's floor raised to 17, under the wall: built, the
/// body the boolean cuts. Raised to 19.3, into two of the wall's dips
/// (19.02 at x 15.8 and 19.08 at x 12.5; at the rim's vertex and across
/// from it the wall is higher): the rim runs out through the wall in
/// stretches, and it is refused. A face of thousands of spans is boxed
/// span by span and asked where an edge comes near it, and answers; what
/// each costs is printed.
///
/// No largest distance that fits is said (7.02 does): the try at the
/// whole distance counts 28 million units here, most of them where the
/// body is asked beside the stretches of the rim that are out (points a
/// few tolerances off a face of 3000 spans, placed by rays), and the
/// search for the largest distance may count sixteen
/// (`craft3d_kernel::offset_face::HALVING_WORK`): it makes no try. Were
/// one said, it would be one that fits.
#[test]
fn a_face_of_thousands_of_spans_is_asked_and_answers() {
    let tol = tolerance(1.0);
    let spans = 3000;
    let count = spans + 3;
    let control: Vec<[f64; 2]> = (0..count)
        .map(|i| {
            let along = i as f64 / (count - 1) as f64;
            let ripple = (std::f64::consts::TAU * 6.0 * along).psin()
                * (std::f64::consts::PI * along).psin();
            [25.0 - 20.0 * along, 20.0 + ripple]
        })
        .collect();
    let mut knots = vec![0.0; 4];
    knots.extend((1..spans).map(|i| i as f64 / spans as f64));
    knots.extend([1.0; 4]);
    let solid = prism(
        vec![
            line([0.0, 0.0], [30.0, 0.0]),
            line([30.0, 0.0], [30.0, 20.0]),
            line([30.0, 20.0], [25.0, 20.0]),
            Curve2::Spline(Spline2 {
                degree: 3,
                knots,
                control,
            }),
            line([5.0, 20.0], [0.0, 20.0]),
            line([0.0, 20.0], [0.0, 0.0]),
        ],
        40.0,
        &tol,
    );
    let pocket = |height: f64| {
        let at = Frame::new(
            Vec3::new(15.0, -5.0, 20.0),
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
        )
        .unwrap();
        primitives::cylinder(&at, 3.0, height + 5.0).unwrap()
    };
    let cut = |height: f64| {
        boolean(&solid, &pocket(height), BooleanOp::Subtract, &tol)
            .unwrap()
            .body
    };
    let body = cut(12.0);
    let floor = (0..body.faces.len() as u32)
        .map(FaceId)
        .find(|f| match &body.face(*f).unwrap().surface {
            Surface::Plane(p) => {
                p.frame().z().y.abs() > 0.99 && (p.frame().origin().y - 12.0).abs() < 1e-6
            }
            _ => false,
        })
        .unwrap();
    let (made, work) = craft3d_geom::allowance::measured(|| offset(&body, floor, -5.0, &tol));
    println!("under a wall of 3000 spans, the floor raised to 17: {work} units");
    let direct = measured(&cut(17.0)).unwrap().volume;
    sound("the floor raised to 17", &made.unwrap(), direct, &tol);
    let (made, work) = craft3d_geom::allowance::measured(|| offset(&body, floor, -7.3, &tol));
    println!("under a wall of 3000 spans, the floor raised to 19.3: {work} units");
    match made {
        Err(KernelError::FaceOpTooFar {
            largest, reason, ..
        }) => {
            println!("refused: {reason}; {largest} fits");
            assert!(reason.contains("pass through another part"), "{reason}");
            // The wall is nowhere lower than 19 over the pocket.
            assert!(largest > 0.0 && largest < 7.03, "{largest}");
        }
        Err(KernelError::FaceOp { reason, .. }) => {
            println!("refused: {reason}");
            assert!(reason.contains("pass through another part"), "{reason}");
        }
        other => panic!("{:?}", other.map(|made| digest(&made))),
    }
}

/// The same operation gives the same answer, bit for bit, however often
/// and on however many threads at once it is asked: a floor raised to
/// just under a spline wall (a body, by its digest) and through it (a
/// refusal, by its words).
#[test]
fn the_same_answer_on_every_run_and_on_every_thread() {
    let answer = || {
        let tol = tolerance(1.0);
        let solid = prism(
            vec![
                line([0.0, 0.0], [30.0, 0.0]),
                line([30.0, 0.0], [30.0, 20.0]),
                line([30.0, 20.0], [25.0, 20.0]),
                Curve2::Spline(Spline2 {
                    degree: 3,
                    knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
                    control: vec![[25.0, 20.0], [18.0, 14.0], [12.0, 27.0], [5.0, 20.0]],
                }),
                line([5.0, 20.0], [0.0, 20.0]),
                line([0.0, 20.0], [0.0, 0.0]),
            ],
            40.0,
            &tol,
        );
        let at = Frame::new(
            Vec3::new(17.5, -5.0, 20.0),
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
        )
        .unwrap();
        let pocket = primitives::cylinder(&at, 3.0, 17.0).unwrap();
        let body = boolean(&solid, &pocket, BooleanOp::Subtract, &tol)
            .unwrap()
            .body;
        let floor = (0..body.faces.len() as u32)
            .map(FaceId)
            .find(|f| match &body.face(*f).unwrap().surface {
                Surface::Plane(p) => {
                    p.frame().z().y.abs() > 0.99 && (p.frame().origin().y - 12.0).abs() < 1e-6
                }
                _ => false,
            })
            .unwrap();
        [-6.38, -6.39, -7.0].map(|distance| match offset(&body, floor, distance, &tol) {
            Ok(made) => format!("built {:016x}", digest(&made)),
            Err(error) => format!("refused: {error}"),
        })
    };
    let first = answer();
    println!("{first:#?}");
    assert!(first[0].starts_with("built"), "{first:?}");
    assert!(
        first[1].starts_with("refused") && first[2].starts_with("refused"),
        "{first:?}"
    );
    assert_eq!(answer(), first);
    let together: Vec<_> = std::thread::scope(|scope| {
        let threads: Vec<_> = (0..4).map(|_| scope.spawn(answer)).collect();
        threads.into_iter().map(|t| t.join().unwrap()).collect()
    });
    for other in together {
        assert_eq!(other, first);
    }
}
