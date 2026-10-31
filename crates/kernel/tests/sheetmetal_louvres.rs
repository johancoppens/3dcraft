//! Sheet-metal flanges on a cut-out's edge (docs/design/sheetmetal.md ›
//! *Flanges on a cut-out's edge*): a louvre bent out of a wall, held to
//! closed forms at 1e-3, 1 and 1e6:
//!
//! - a base 60 × 40 with a wall 30 high on its front edge, a slot 20 × 8
//!   cut in the wall (a cut-out drawn in the pattern) and a louvre 16 wide
//!   bent from the slot's edge nearer the bend: the flat area
//!   `A − slot + w (BA_l + L_l)`, the volume `T` times the flats plus
//!   `θ w T (R + T/2)` per bend, fold then unfold, the louvre's angle;
//! - the same wall closed by a mitre with a second wall: the louvre adds
//!   exactly its own area and material, whatever the corner does;
//! - names: a louvre added renames no edge but the one it is bent from,
//!   and another slot beside it keeps its names and its hole-to-bend
//!   distance;
//! - refusals by reason: no such cut-out or curve, a cut-out in no flat
//!   laid out before the flange (across a bend, or in a later flange's
//!   flat), a curved edge, a louvre too long for its slot.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_kernel::KernelError;
use craft3d_kernel::profile::{Curve2, Profile, SketchCurve, detect_profiles};
use craft3d_kernel::sheetmetal::{
    Closing, CornerKind, Cutout, Direction, FlangeSpec, LengthFrom, On, Part, SheetFace, Spec,
    build, developed_allowance, edge_numbers, flat_area, holes_to_bends, measured_angle,
};
use craft3d_math::{Frame, Portable, Tolerance, Vec3};
use craft3d_topo::{FaceId, check, mass_properties};
use std::collections::BTreeSet;
use std::f64::consts::{FRAC_PI_2, FRAC_PI_4};

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

fn tolerance(k: f64) -> Tolerance {
    Tolerance::new(1e-6 * k, 1e-9).unwrap()
}

/// A closed polygon (scaled by `k`) as sketch lines numbered from 1 in
/// order, and its one profile.
fn polygon(pts: &[[f64; 2]], k: f64) -> Profile {
    let curves: Vec<SketchCurve> = (0..pts.len())
        .map(|i| {
            let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
            SketchCurve {
                id: i as u64 + 1,
                curve: Curve2::Line {
                    start: [a[0] * k, a[1] * k],
                    end: [b[0] * k, b[1] * k],
                },
            }
        })
        .collect();
    let mut found = detect_profiles(&curves, &tolerance(k)).unwrap();
    assert_eq!(found.len(), 1);
    found.remove(0)
}

/// The rectangle `x0..x1` × `y0..y1` (unscaled numbers): curves 1 to 4
/// from the bottom edge on, counter-clockwise.
fn rect(x0: f64, y0: f64, x1: f64, y1: f64, k: f64) -> Profile {
    polygon(&[[x0, y0], [x1, y0], [x1, y1], [x0, y1]], k)
}

fn flange(on: On, length: f64, angle: f64, direction: Direction, r: f64, kf: f64) -> FlangeSpec {
    FlangeSpec {
        on,
        length,
        angle,
        direction,
        radius: r,
        k: kf,
        from: LengthFrom::Tangent,
        inset: [0.0; 2],
        outline: None,
    }
}

#[track_caller]
fn near(actual: f64, expected: f64, slack: f64, what: &str) {
    assert!(
        (actual - expected).abs() <= slack,
        "{what}: {actual} against {expected} (off by {:e}, allowed {slack:e})",
        actual - expected
    );
}

fn volume(part: &Part) -> f64 {
    mass_properties(&part.solid.body).unwrap().volume
}

fn reason(spec: &Spec, tol: &Tolerance) -> String {
    match build(spec, tol) {
        Ok(_) => panic!("built"),
        Err(KernelError::SheetMetal { reason, .. }) => reason,
        Err(other) => format!("other: {other}"),
    }
}

/// The solid passes the checker; fold then unfold: every folded flat's
/// vertices, carried back by its inverse motion, land at z = 0 on its
/// outline or holes in the pattern, and each bend's neutral band develops
/// to its strip's width.
fn healthy(part: &Part, tol: &Tolerance) {
    let defects = check(&part.solid.body, tol);
    assert!(
        defects.is_empty(),
        "the solid fails the checker: {defects:?}"
    );
    for (f, role) in part.sheet_faces.iter().enumerate() {
        let SheetFace::Flat(i) = role else { continue };
        let flat = &part.flats[*i];
        let back = flat.transform.inverse().unwrap();
        let face = part.sheet.face(FaceId(f as u32)).unwrap();
        for l in &face.loops {
            for c in &part.sheet.loop_(*l).unwrap().coedges {
                let v = part.sheet.coedge_ends(*c).unwrap().0;
                let (x, y, z) = part.plane.local(part.sheet.vertex(v).unwrap().point);
                let p = back.point(Vec3::new(x, y, z));
                assert!(p.z.abs() <= tol.linear, "off the plane by {}", p.z);
                let on = flat
                    .outer
                    .iter()
                    .chain(flat.holes.iter().flatten())
                    .map(|s| (s.start[0] - p.x).phypot(s.start[1] - p.y))
                    .fold(f64::INFINITY, f64::min);
                assert!(on <= tol.linear, "a vertex lands {on} from its corner");
            }
        }
    }
    for b in &part.bends {
        near(
            developed_allowance(b, tol).unwrap(),
            b.allowance,
            4.0 * tol.linear,
            "the developed bend allowance",
        );
    }
}

fn names(part: &Part) -> BTreeSet<Vec<u64>> {
    part.edge_keys
        .iter()
        .map(|k| edge_numbers(part, *k).unwrap())
        .collect()
}

/// The part: a base 60 × 40 (curves 1 to 4), a wall `h` high bent up from
/// its front edge (curve 1, so the wall's flat lies at `y` from `−BA` to
/// `−BA − h` in the pattern), slots 20 × 8 cut in it (each a cut-out
/// whose curve 3 is its edge nearer the bend), and on each slot listed in
/// `louvres` a louvre bent from that edge, 16 wide (2 in at each end), `l`
/// long, at `angle`, down. With `second`, a second wall on the right edge
/// (curve 2) and their corner closed by a mitre with gap `g`.
struct Wall {
    k: f64,
    t: f64,
    r: f64,
    kf: f64,
    h: f64,
    l: f64,
    angle: f64,
}

impl Wall {
    fn new(k: f64, kf: f64) -> Self {
        Wall {
            k,
            t: 1.0 * k,
            r: 1.0 * k,
            kf,
            h: 30.0,
            l: 4.0 * k,
            angle: FRAC_PI_4,
        }
    }

    fn ba(&self, angle: f64) -> f64 {
        angle * (self.r + self.kf * self.t)
    }

    /// Slot `j`'s cut-out: x from `5 + 25 j` to `25 + 25 j`, y from 12 to
    /// 20 below the wall's tangent line on its flat.
    fn slot(&self, j: usize) -> Cutout {
        let k = self.k;
        let top = -self.ba(FRAC_PI_2) / k - 12.0;
        let x0 = 5.0 + 25.0 * j as f64;
        Cutout {
            profile: rect(x0, top - 8.0, x0 + 20.0, top, k),
            from: format!("slot {j}"),
        }
    }

    fn spec(&self, slots: usize, louvres: &[usize], second: Option<f64>) -> Spec {
        let k = self.k;
        let mut flanges = vec![flange(
            On::Base(1),
            self.h * k,
            FRAC_PI_2,
            Direction::Up,
            self.r,
            self.kf,
        )];
        let mut corners = Vec::new();
        if let Some(g) = second {
            flanges.push(flange(
                On::Base(2),
                self.h * k,
                FRAC_PI_2,
                Direction::Up,
                self.r,
                self.kf,
            ));
            corners.push(craft3d_kernel::sheetmetal::CornerSpec {
                flanges: [0, 1],
                closing: Closing {
                    kind: CornerKind::Miter,
                    gap: g,
                    over: None,
                },
            });
        }
        for &j in louvres {
            flanges.push(FlangeSpec {
                inset: [2.0 * k, 2.0 * k],
                ..flange(
                    On::Cutout(j, 3),
                    self.l,
                    self.angle,
                    Direction::Down,
                    self.r,
                    self.kf,
                )
            });
        }
        Spec {
            base: rect(0.0, 0.0, 60.0, 40.0, k),
            plane: Frame::WORLD,
            thickness: self.t,
            flanges,
            relief: None,
            bend_relief: None,
            corner: Closing::OPEN,
            corners,
            cutouts: (0..slots).map(|j| self.slot(j)).collect(),
            ids: Vec::new(),
        }
    }
}

#[test]
fn a_louvre_bent_out_of_a_wall_by_its_closed_forms() {
    for k in SCALES {
        for kf in [0.33, 0.5] {
            let tol = tolerance(k);
            let w = Wall::new(k, kf);
            let part = build(&w.spec(1, &[0], None), &tol).unwrap();
            healthy(&part, &tol);
            let (t, r, h, l) = (w.t, w.r, w.h * k, w.l);
            let (wl, ww, slot) = (16.0 * k, 60.0 * k, 160.0 * k * k);
            let base = 2400.0 * k * k;
            let flats = base + ww * h - slot + wl * l;
            near(
                flat_area(&part),
                flats + ww * w.ba(FRAC_PI_2) + wl * w.ba(w.angle),
                1e-9 * base,
                "flat area",
            );
            // Each bend's material is θ w T (R + T/2) (the skins R and R + T).
            let bend = |theta: f64, width: f64| theta * width * t * (r + t / 2.0);
            let v = t * flats + bend(FRAC_PI_2, ww) + bend(w.angle, wl);
            near(volume(&part), v, 1e-9 * v, "volume");
            near(
                measured_angle(&part, 1).unwrap(),
                w.angle,
                1e-9,
                "the louvre's angle",
            );
            // The louvre's flat hangs from the wall: its parent is the
            // wall's flat (flat 1), the slot is its edge, not a hole.
            assert_eq!(part.bends[1].parent, 1);
            assert!(holes_to_bends(&part, &tol).is_empty());
            // The same part twice is the same body.
            let again = build(&w.spec(1, &[0], None), &tol).unwrap();
            assert_eq!(volume(&again).to_bits(), volume(&part).to_bits());
        }
    }
}

#[test]
fn a_louvre_in_a_wall_with_a_closed_corner_adds_only_itself() {
    for k in SCALES {
        let tol = tolerance(k);
        let w = Wall::new(k, 0.33);
        let g = 0.2 * k;
        let without = build(&w.spec(1, &[], Some(g)), &tol).unwrap();
        let with = build(&w.spec(1, &[0], Some(g)), &tol).unwrap();
        healthy(&with, &tol);
        let wl = 16.0 * k;
        near(
            flat_area(&with) - flat_area(&without),
            wl * (w.ba(w.angle) + w.l),
            1e-9 * flat_area(&with),
            "the louvre's flat area",
        );
        let dv = w.t * wl * w.l + w.angle * wl * w.t * (w.r + w.t / 2.0);
        near(
            volume(&with) - volume(&without),
            dv,
            1e-9 * volume(&with),
            "the louvre's material",
        );
    }
}

#[test]
fn a_louvre_added_renames_only_the_edge_it_is_bent_from() {
    for k in SCALES {
        let tol = tolerance(k);
        let w = Wall::new(k, 0.5);
        // Two slots; a louvre on the second only, then on both.
        let none = build(&w.spec(2, &[], None), &tol).unwrap();
        let one = build(&w.spec(2, &[1], None), &tol).unwrap();
        // The second slot's louvre keeps its id (2) when the first's is
        // added (id 3), as `sheetMetal.edit` keeps ids.
        let mut spec = w.spec(2, &[0, 1], None);
        spec.ids = vec![1, 3, 2];
        let both = build(&spec, &tol).unwrap();
        healthy(&both, &tol);
        let (n0, n1, n2) = (names(&none), names(&one), names(&both));
        // The slot's edge a louvre sits on is split at the louvre's bend:
        // its name goes, every other stays.
        let gone: Vec<_> = n0.difference(&n1).collect();
        assert_eq!(gone.len(), 1, "{gone:?}");
        assert_eq!(gone[0][0], 13, "{gone:?}");
        let gone2: Vec<_> = n1.difference(&n2).collect();
        assert_eq!(
            gone2.len(),
            1,
            "{gone2:?} {:?}",
            n2.difference(&n1).collect::<Vec<_>>()
        );
        // The first slot, without a louvre, is measured from every bend
        // beside the wall's flat: 12 from the wall's own tangent line, and
        // 7 from the second slot's louvre's (its corner at x = 25 to the
        // louvre's bend starting at x = 30 + 2, on one line), the least;
        // with its own louvre it is that louvre's edge.
        let near_bend = holes_to_bends(&one, &tol);
        assert_eq!(near_bend.len(), 1);
        near(
            near_bend[0].distance,
            7.0 * k,
            1e-9 * k,
            "the slot without a louvre to the nearest bend",
        );
        assert!(holes_to_bends(&both, &tol).is_empty());
    }
}

#[test]
fn louvres_that_cannot_be_made_are_refused_by_reason() {
    let k = 1.0;
    let tol = tolerance(k);
    let w = Wall::new(k, 0.5);
    // No such cut-out, no such curve.
    let mut spec = w.spec(1, &[0], None);
    spec.flanges[1].on = On::Cutout(3, 3);
    assert!(reason(&spec, &tol).contains("the part has 1 cut-outs"));
    spec.flanges[1].on = On::Cutout(0, 9);
    assert!(reason(&spec, &tol).contains("no edge of the cut-out"));
    // A louvre listed before the wall whose flat holds its slot.
    let mut spec = w.spec(1, &[0], None);
    spec.flanges.swap(0, 1);
    assert!(reason(&spec, &tol).contains("lies in no flat laid out before it"));
    // A slot across the wall's bend lies in no flat.
    let mut spec = w.spec(1, &[0], None);
    spec.cutouts[0] = Cutout {
        profile: rect(20.0, -5.0, 40.0, 3.0, k),
        from: "a slot across the bend".into(),
    };
    assert!(reason(&spec, &tol).contains("lies in no flat laid out before it"));
    // Too long for its slot: it runs into the slot's far side.
    let mut spec = w.spec(1, &[0], None);
    spec.flanges[1].length = 8.0 * k;
    assert!(reason(&spec, &tol).contains("overlaps itself"));
    // A cut-out with a curved edge: a round hole has no straight edge.
    let mut spec = w.spec(1, &[0], None);
    let circle = SketchCurve {
        id: 3,
        curve: Curve2::Circle {
            center: [30.0, -20.0],
            radius: 3.0,
        },
    };
    spec.cutouts[0] = Cutout {
        profile: detect_profiles(&[circle], &tol).unwrap().remove(0),
        from: "a round hole".into(),
    };
    assert!(reason(&spec, &tol).contains("not straight"));
}

/// A louvre added in one wall renames nothing in another: the second
/// wall (closed to the first by a mitre) has a slot of its own, and its
/// hole keeps its names when the first wall's slot gets a louvre (a flat
/// that holds no cut-out a flange sits on takes its cut-outs after the
/// corners are closed, as without the louvre; review 2026-10-31).
#[test]
fn a_louvre_in_one_wall_renames_no_hole_of_another() {
    for k in SCALES {
        let tol = tolerance(k);
        let w = Wall::new(k, 0.5);
        let ba = w.ba(FRAC_PI_2) / k;
        let other = Cutout {
            profile: rect(60.0 + ba + 12.0, 10.0, 60.0 + ba + 20.0, 30.0, k),
            from: "a slot in the second wall".into(),
        };
        let mut without = w.spec(1, &[], Some(0.2 * k));
        without.cutouts.push(other.clone());
        let mut with = w.spec(1, &[0], Some(0.2 * k));
        with.cutouts.push(other);
        let (a, b) = (build(&without, &tol).unwrap(), build(&with, &tol).unwrap());
        healthy(&b, &tol);
        let (na, nb) = (names(&a), names(&b));
        let gone: Vec<_> = na.difference(&nb).collect();
        assert_eq!(gone.len(), 1, "scale {k}: {gone:?}");
        // The second wall's hole is a hole of flat 2, named as before.
        let second: Vec<_> = na.iter().filter(|n| n[0] == 13 && n[1] == 2).collect();
        assert_eq!(second.len(), 4, "scale {k}: {na:?}");
        assert!(second.iter().all(|n| nb.contains(*n)), "scale {k}");
    }
}
