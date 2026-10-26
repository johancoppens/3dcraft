//! Origami (docs/design/origami.md): crease patterns divided into facets,
//! folded rigidly, built as sheets, and checked by Kawasaki and Maekawa,
//! held to closed forms at three scales.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_kernel::origami::{
    Angle, Arrangement, BoundarySegment, Crease, CreaseKind, FacetMeeting, FoldError, Folded,
    arrange, first_meeting, fold, folded_sheet, vertex_checks,
};
use craft3d_math::{Frame, Tolerance, Vec3};
use std::f64::consts::PI;

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];
const ANGULAR: f64 = 1e-9;

fn tol(scale: f64) -> f64 {
    1e-9 * scale
}

/// A counter-clockwise polygon as boundary segments, curve ids from 1.
fn polygon(points: &[[f64; 2]]) -> Vec<BoundarySegment> {
    let n = points.len();
    (0..n)
        .map(|i| BoundarySegment {
            id: i as u64 + 1,
            start: points[i],
            end: points[(i + 1) % n],
            reversed: false,
        })
        .collect()
}

fn square(s: f64) -> Vec<BoundarySegment> {
    polygon(&[[0.0, 0.0], [s, 0.0], [s, s], [0.0, s]])
}

fn crease(id: u64, start: [f64; 2], end: [f64; 2], kind: CreaseKind) -> Crease {
    Crease {
        id,
        start,
        end,
        kind,
    }
}

/// The angle of every hinge: given where `given` names the crease,
/// otherwise solved for, turning the way the crease's kind says.
fn angles(arr: &Arrangement, given: &[(u64, f64)]) -> Vec<Angle> {
    arr.hinges
        .iter()
        .map(|h| {
            let c = &arr.creases[h.crease];
            match given.iter().find(|(id, _)| *id == c.id) {
                Some((_, a)) => Angle::Given(c.kind.sign() * a),
                None => Angle::Free {
                    sign: c.kind.sign(),
                },
            }
        })
        .collect()
}

/// The solved angle of each piece of crease `id`.
fn solved(arr: &Arrangement, folded: &Folded, id: u64) -> Vec<f64> {
    arr.hinges
        .iter()
        .zip(&folded.angles)
        .filter(|(h, _)| arr.creases[h.crease].id == id)
        .map(|(_, a)| *a)
        .collect()
}

fn frame() -> Frame {
    Frame::new(Vec3::ZERO, Vec3::Z, Vec3::X).unwrap()
}

fn tolerance(scale: f64) -> Tolerance {
    Tolerance::new(tol(scale), ANGULAR).unwrap()
}

fn close(a: f64, b: f64, eps: f64) -> bool {
    (a - b).abs() <= eps
}

#[test]
fn a_single_fold_keeps_the_area_and_turns_by_its_angle() {
    for scale in SCALES {
        let s = 2.0 * scale;
        let arr = arrange(
            &square(s),
            &[crease(10, [s / 2.0, 0.0], [s / 2.0, s], CreaseKind::Valley)],
            tol(scale),
        )
        .unwrap();
        assert_eq!(arr.facets.len(), 2);
        assert_eq!(arr.hinges.len(), 1);
        // Congruent halves.
        assert!(close(arr.facets[0].area, arr.facets[1].area, 1e-12 * s * s));
        for degrees in [0.0, 30.0, 90.0, 150.0] {
            let theta = degrees * PI / 180.0;
            let fixed = arr.facet_at([s / 4.0, s / 2.0]).unwrap();
            let folded = fold(
                &arr,
                &angles(&arr, &[(10, theta)]),
                fixed,
                tol(scale),
                ANGULAR,
            )
            .unwrap();
            assert!(close(folded.angles[0], theta, 1e-12));
            let made = folded_sheet(&arr, &folded, &frame(), &tolerance(scale)).unwrap();
            let mass = craft3d_topo::mass_properties(&made.body).unwrap();
            assert!(
                close(mass.area, s * s, 1e-12 * s * s),
                "area {} at {degrees}",
                mass.area
            );
            assert_eq!(made.body.faces.len(), 2);
            assert_eq!(made.body.shells.len(), 1);
            // The far corner of the turned half: (s/2 + s/2 cos θ, y, s/2 sin θ).
            let far = made.body.vertices.iter().map(|v| v.point).fold(
                Vec3::new(f64::NEG_INFINITY, 0.0, 0.0),
                |a, p| {
                    if p.z > a.z || (p.z == a.z && p.x > a.x) {
                        p
                    } else {
                        a
                    }
                },
            );
            let expect = Vec3::new(s / 2.0 + s / 2.0 * theta.cos(), 0.0, s / 2.0 * theta.sin());
            assert!(
                close(far.x, expect.x, 1e-12 * s) && close(far.z, expect.z, 1e-12 * s),
                "{far:?} vs {expect:?}"
            );
            // The angle between the facets' normals is the fold angle.
            let normals: Vec<Vec3> = folded
                .transforms
                .iter()
                .map(|t| t.vector(Vec3::Z))
                .collect();
            let between = normals[0].dot(normals[1]).clamp(-1.0, 1.0).acos();
            assert!(close(between, theta, 1e-9), "{between} vs {theta}");
            // A valley lifts the turned half towards the normal.
            assert!(far.z >= -1e-12 * s);
            assert!(
                first_meeting(&arr, &folded, tol(scale), ANGULAR)
                    .unwrap()
                    .is_none()
            );
        }
    }
}

#[test]
fn a_diagonal_fold_has_congruent_triangles_and_a_mountain_goes_down() {
    for scale in SCALES {
        let s = scale;
        let arr = arrange(
            &square(s),
            &[crease(10, [0.0, 0.0], [s, s], CreaseKind::Mountain)],
            tol(scale),
        )
        .unwrap();
        assert_eq!(arr.facets.len(), 2);
        let fixed = arr.facet_at([0.75 * s, 0.25 * s]).unwrap();
        let folded = fold(
            &arr,
            &angles(&arr, &[(10, PI / 3.0)]),
            fixed,
            tol(scale),
            ANGULAR,
        )
        .unwrap();
        let made = folded_sheet(&arr, &folded, &frame(), &tolerance(scale)).unwrap();
        let areas: Vec<f64> = craft3d_topo::face_areas(&made.body)
            .into_iter()
            .map(Result::unwrap)
            .collect();
        assert!(
            close(areas[0], s * s / 2.0, 1e-12 * s * s)
                && close(areas[1], s * s / 2.0, 1e-12 * s * s)
        );
        // The corner (0, s) goes down for a mountain.
        let lowest = made
            .body
            .vertices
            .iter()
            .map(|v| v.point.z)
            .fold(0.0, f64::min);
        let expect = -(s / 2.0f64.sqrt()) * (PI / 3.0).sin();
        assert!(close(lowest, expect, 1e-12 * s), "{lowest} vs {expect}");
    }
}

/// A sheet with one interior vertex at the middle and creases out to the
/// boundary at the given directions.
fn single_vertex(scale: f64, directions: &[f64], kinds: &[CreaseKind]) -> Arrangement {
    let s = scale;
    let boundary = polygon(&[[-s, -s], [s, -s], [s, s], [-s, s]]);
    let creases: Vec<Crease> = directions
        .iter()
        .zip(kinds)
        .enumerate()
        .map(|(i, (&a, &kind))| {
            // Out to the square's boundary.
            let (c, sn) = (a.cos(), a.sin());
            let t = s / c.abs().max(sn.abs());
            crease(10 + i as u64, [0.0, 0.0], [t * c, t * sn], kind)
        })
        .collect();
    arrange(&boundary, &creases, tol(scale)).unwrap()
}

#[test]
fn a_degree_four_vertex_folds_by_the_closed_form() {
    use CreaseKind::{Mountain as M, Valley as V};
    for scale in SCALES {
        for (alpha, beta) in [(70.0f64, 50.0f64), (100.0, 35.0), (60.0, 60.0)] {
            let (a, b) = (alpha.to_radians(), beta.to_radians());
            let dirs = [0.0, a, a + b, PI + b];
            for rho1 in [10.0f64, 60.0, 120.0] {
                let r1 = rho1.to_radians();
                // Mode A: ρ3 = ρ1, ρ4 = −ρ2,
                // tan(ρ2/2) = −cos((α+β)/2)/cos((α−β)/2) tan(ρ1/2).
                let p = ((a + b) / 2.0).cos() / ((a - b) / 2.0).cos();
                if p.abs() > 1e-6 {
                    let r2 = 2.0 * (-p * (r1 / 2.0).tan()).atan();
                    let kinds = if r2 < 0.0 { [V, M, V, V] } else { [V, V, V, M] };
                    let arr = single_vertex(scale, &dirs, &kinds);
                    let folded =
                        fold(&arr, &angles(&arr, &[(10, r1)]), 0, tol(scale), ANGULAR).unwrap();
                    let got = [
                        solved(&arr, &folded, 10)[0],
                        solved(&arr, &folded, 11)[0],
                        solved(&arr, &folded, 12)[0],
                        solved(&arr, &folded, 13)[0],
                    ];
                    let want = [r1, r2, r1, -r2];
                    for (g, w) in got.iter().zip(want) {
                        assert!(
                            close(*g, w, 1e-9),
                            "mode A α {alpha} β {beta} ρ1 {rho1}: {got:?} vs {want:?}"
                        );
                    }
                }
                // Mode B: ρ3 = −ρ1, ρ4 = ρ2,
                // tan(ρ2/2) = sin((α+β)/2)/sin((α−β)/2) tan(ρ1/2).
                let q = ((a + b) / 2.0).sin() / ((a - b) / 2.0).sin();
                if q.is_finite() && (alpha - beta).abs() > 1.0 {
                    let r2 = 2.0 * (q * (r1 / 2.0).tan()).atan();
                    if r2.abs() >= PI * 0.95 {
                        continue;
                    }
                    let k2 = if r2 > 0.0 { V } else { M };
                    let kinds = [V, k2, M, k2];
                    let arr = single_vertex(scale, &dirs, &kinds);
                    let folded =
                        fold(&arr, &angles(&arr, &[(10, r1)]), 0, tol(scale), ANGULAR).unwrap();
                    let got = [
                        solved(&arr, &folded, 10)[0],
                        solved(&arr, &folded, 11)[0],
                        solved(&arr, &folded, 12)[0],
                        solved(&arr, &folded, 13)[0],
                    ];
                    let want = [r1, r2, -r1, r2];
                    for (g, w) in got.iter().zip(want) {
                        assert!(
                            close(*g, w, 1e-9),
                            "mode B α {alpha} β {beta} ρ1 {rho1}: {got:?} vs {want:?}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn kawasaki_and_maekawa_hold_on_flat_foldable_vertices_and_fail_on_a_perturbed_one() {
    use CreaseKind::{Mountain as M, Valley as V};
    for scale in SCALES {
        let (a, b) = (70f64.to_radians(), 50f64.to_radians());
        let arr = single_vertex(scale, &[0.0, a, a + b, PI + b], &[V, M, V, V]);
        let (checks, cuts) = vertex_checks(&arr, ANGULAR);
        assert!(cuts.is_empty());
        assert_eq!(checks.len(), 1);
        let c = &checks[0];
        assert_eq!((c.mountains, c.valleys), (1, 3));
        assert!(c.kawasaki_holds && c.maekawa_holds, "{c:?}");
        assert!(close(c.sums[0], PI, 1e-12) && close(c.sums[1], PI, 1e-12));
        // Turn one crease by 2 degrees: Kawasaki fails by exactly that much
        // in one of the sums.
        let arr = single_vertex(
            scale,
            &[0.0, a + 2f64.to_radians(), a + b, PI + b],
            &[V, M, V, V],
        );
        let c = &vertex_checks(&arr, ANGULAR).0[0];
        assert!(!c.kawasaki_holds && c.maekawa_holds);
        assert!(
            close(c.kawasaki.unwrap().abs(), 2f64.to_radians(), 1e-12),
            "{c:?}"
        );
        // All valleys: Maekawa fails, |M − V| = 4.
        let arr = single_vertex(scale, &[0.0, a, a + b, PI + b], &[V, V, V, V]);
        let c = &vertex_checks(&arr, ANGULAR).0[0];
        assert!(c.kawasaki_holds && !c.maekawa_holds && c.maekawa == 4);
        // Odd degree never folds flat.
        let arr = single_vertex(scale, &[0.0, 2.0 * PI / 3.0, 4.0 * PI / 3.0], &[V, V, M]);
        let c = &vertex_checks(&arr, ANGULAR).0[0];
        assert!(c.kawasaki.is_none() && !c.kawasaki_holds && !c.maekawa_holds);
    }
}

/// The waterbomb base: both diagonals valleys, one midline a mountain.
/// Its symmetric folding, by the spherical linkage: rays m = (cos t, 0,
/// sin t), d = (±√2/2, ±q, √2/2 tan(t/2)) with q² = 1/2 − d_z², which
/// keep the sector angles (45, 90, 45, 45, 90, 45 degrees); the fold
/// angles are the angles between the normals of the facets either side.
fn waterbomb_angles(t: f64) -> (f64, f64) {
    let r2 = 0.5f64.sqrt();
    let zd = r2 * (t / 2.0).tan();
    let q = (0.5 - zd * zd).sqrt();
    let m1 = Vec3::new(t.cos(), 0.0, t.sin());
    let d1 = Vec3::new(r2, q, zd);
    let d2 = Vec3::new(-r2, q, zd);
    let d4 = Vec3::new(r2, -q, zd);
    let n0 = d4.cross(m1);
    let n1 = m1.cross(d1);
    let n2 = d1.cross(d2);
    let angle = |a: Vec3, b: Vec3| {
        (a.dot(b) / (a.length() * b.length()))
            .clamp(-1.0, 1.0)
            .acos()
    };
    (angle(n0, n1), angle(n1, n2))
}

#[test]
fn the_waterbomb_base_folds_by_its_spherical_linkage_and_needs_three_angles() {
    use CreaseKind::{Mountain as M, Valley as V};
    for scale in SCALES {
        let s = scale;
        let boundary = polygon(&[[-s, -s], [s, -s], [s, s], [-s, s]]);
        let creases = [
            crease(10, [-s, 0.0], [s, 0.0], M),
            crease(11, [-s, -s], [s, s], V),
            crease(12, [s, -s], [-s, s], V),
        ];
        let arr = arrange(&boundary, &creases, tol(s)).unwrap();
        assert_eq!(arr.facets.len(), 6);
        assert_eq!(arr.hinges.len(), 6);
        let (checks, _) = vertex_checks(&arr, ANGULAR);
        assert_eq!(checks.len(), 1);
        assert!(
            checks[0].kawasaki_holds && checks[0].maekawa_holds,
            "{:?}",
            checks[0]
        );
        for t in [0.2, 0.6, 1.0, 1.4] {
            let (mu, delta) = waterbomb_angles(t);
            // Midline and one diagonal given: the other diagonal is solved,
            // and comes out as the linkage says.
            let folded = fold(
                &arr,
                &angles(&arr, &[(10, mu), (11, delta)]),
                0,
                tol(s),
                ANGULAR,
            )
            .unwrap();
            for a in solved(&arr, &folded, 12) {
                assert!(close(a, delta, 1e-9), "t {t}: {a} vs {delta}");
            }
            // All given by the linkage: the loop closes.
            let all = fold(
                &arr,
                &angles(&arr, &[(10, mu), (11, delta), (12, delta)]),
                0,
                tol(s),
                ANGULAR,
            )
            .unwrap();
            assert!(all.gap <= tol(s));
            let made = folded_sheet(&arr, &all, &frame(), &tolerance(s)).unwrap();
            let area = craft3d_topo::mass_properties(&made.body).unwrap().area;
            assert!(close(area, 4.0 * s * s, 1e-11 * s * s));
            assert!(
                first_meeting(&arr, &all, tol(s), ANGULAR)
                    .unwrap()
                    .is_none()
            );
            // The midline alone leaves two angles free.
            let loose = fold(&arr, &angles(&arr, &[(10, mu)]), 0, tol(s), ANGULAR);
            assert!(
                matches!(loose, Err(FoldError::Undetermined { .. })),
                "{loose:?}"
            );
            // A diagonal angle that does not go with the midline's.
            let wrong = fold(
                &arr,
                &angles(&arr, &[(10, mu), (11, delta), (12, delta * 0.8)]),
                0,
                tol(s),
                ANGULAR,
            );
            assert!(
                matches!(wrong, Err(FoldError::NotClosing { .. })),
                "{wrong:?}"
            );
        }
    }
}

/// A Miura-ori sheet of `cols` × `rows` parallelograms: straight creases
/// along x, zigzag creases between the rows. Vertex (i, j) at
/// (i L + (j mod 2) d, j H). Zigzag column i is a valley for even i;
/// along the straight creases mountains and valleys alternate.
fn miura(
    scale: f64,
    cols: usize,
    rows: usize,
    l: f64,
    d: f64,
    h: f64,
) -> (Vec<BoundarySegment>, Vec<Crease>) {
    let at = |i: usize, j: usize| {
        [
            (i as f64 * l + (j % 2) as f64 * d) * scale,
            j as f64 * h * scale,
        ]
    };
    let mut outline = Vec::new();
    for i in 0..cols {
        outline.push(at(i, 0));
    }
    for j in 0..rows {
        outline.push(at(cols, j));
    }
    for i in (1..=cols).rev() {
        outline.push(at(i, rows));
    }
    for j in (1..=rows).rev() {
        outline.push(at(0, j));
    }
    let boundary = polygon(&outline);
    let mut creases = Vec::new();
    let mut id = 100;
    let zig = |i: usize| if i.is_multiple_of(2) { 1.0 } else { -1.0 };
    let kind = |sign: f64| {
        if sign > 0.0 {
            CreaseKind::Valley
        } else {
            CreaseKind::Mountain
        }
    };
    for i in 1..cols {
        for j in 0..rows {
            creases.push(crease(id, at(i, j), at(i, j + 1), kind(zig(i))));
            id += 1;
        }
    }
    for j in 1..rows {
        for i in 0..cols {
            let sign = if j.is_multiple_of(2) { -zig(i) } else { zig(i) };
            creases.push(crease(id, at(i, j), at(i + 1, j), kind(sign)));
            id += 1;
        }
    }
    (boundary, creases)
}

#[test]
fn a_miura_ori_sheet_folds_with_one_degree_of_freedom_by_its_closed_form() {
    let (l, d, h): (f64, f64, f64) = (1.0, 0.5, 1.0);
    let gamma = (h / d).atan(); // the parallelograms' acute angle
    for scale in SCALES {
        let (boundary, creases) = miura(scale, 4, 4, l, d, h);
        let arr = arrange(&boundary, &creases, tol(scale)).unwrap();
        assert_eq!(arr.facets.len(), 16);
        let (checks, _) = vertex_checks(&arr, ANGULAR);
        assert_eq!(checks.len(), 9);
        assert!(
            checks.iter().all(|c| c.kawasaki_holds && c.maekawa_holds),
            "{checks:?}"
        );
        // One straight crease piece drives the whole sheet.
        let driver = creases
            .iter()
            .find(|c| c.start[1] > 0.0 && c.start[1] == c.end[1])
            .unwrap()
            .id;
        for degrees in [20.0f64, 70.0, 140.0] {
            let rho = degrees.to_radians();
            let folded = fold(
                &arr,
                &angles(&arr, &[(driver, rho)]),
                0,
                tol(scale),
                ANGULAR,
            )
            .unwrap();
            // Every straight piece folds by ρ (alternately up and down), every
            // zigzag piece by 2 atan(tan(ρ/2) / cos γ).
            let zig = 2.0 * ((rho / 2.0).tan() / gamma.cos()).atan();
            for (hinge, a) in arr.hinges.iter().zip(&folded.angles) {
                let c = &arr.creases[hinge.crease];
                let want = if c.start[1] == c.end[1] { rho } else { zig };
                assert!(
                    close(a.abs(), want, 1e-8),
                    "{degrees}: crease {} folds {} not {}",
                    c.id,
                    a,
                    want
                );
                assert!(a.signum() == c.kind.sign());
            }
            let made = folded_sheet(&arr, &folded, &frame(), &tolerance(scale)).unwrap();
            let area = craft3d_topo::mass_properties(&made.body).unwrap().area;
            let flat: f64 = arr.facets.iter().map(|f| f.area).sum();
            assert!(close(area, flat, 1e-10 * flat));
            assert!(close(flat, 16.0 * l * h * scale * scale, 1e-10 * flat));
            assert!(
                first_meeting(&arr, &folded, tol(scale), ANGULAR)
                    .unwrap()
                    .is_none(),
                "{degrees}"
            );
        }
    }
}

#[test]
fn a_strip_folded_back_on_itself_touches_then_passes_through() {
    for scale in SCALES {
        let boundary = polygon(&[
            [0.0, 0.0],
            [3.0 * scale, 0.0],
            [3.0 * scale, scale],
            [0.0, scale],
        ]);
        let creases = [
            crease(10, [scale, 0.0], [scale, scale], CreaseKind::Valley),
            crease(
                11,
                [2.0 * scale, 0.0],
                [2.0 * scale, scale],
                CreaseKind::Valley,
            ),
        ];
        let arr = arrange(&boundary, &creases, tol(scale)).unwrap();
        let fixed = arr.facet_at([0.5 * scale, 0.5 * scale]).unwrap();
        let at = |deg: f64| {
            let r = deg.to_radians();
            let folded = fold(
                &arr,
                &angles(&arr, &[(10, r), (11, r)]),
                fixed,
                tol(scale),
                ANGULAR,
            )
            .unwrap();
            first_meeting(&arr, &folded, tol(scale), ANGULAR).unwrap()
        };
        assert!(at(100.0).is_none());
        // An equilateral triangle: the far edge lands on the first one's.
        let touch = at(120.0).unwrap();
        assert_eq!(touch.kind, FacetMeeting::Touch);
        let through = at(130.0).unwrap();
        assert_eq!(through.kind, FacetMeeting::Through);
        // Folded flat on itself: the halves lie on each other.
        let folded = fold(
            &arr,
            &angles(&arr, &[(10, PI), (11, 0.0)]),
            fixed,
            tol(scale),
            ANGULAR,
        )
        .unwrap();
        assert_eq!(
            first_meeting(&arr, &folded, tol(scale), ANGULAR)
                .unwrap()
                .unwrap()
                .kind,
            FacetMeeting::Touch
        );
    }
}

#[test]
fn a_cut_separates_the_sheet_and_each_piece_folds_on_its_own() {
    for scale in SCALES {
        let s = scale;
        // A cut along x = s/2, a valley along y = s/2 on each side.
        let arr = arrange(
            &square(s),
            &[
                crease(10, [s / 2.0, 0.0], [s / 2.0, s], CreaseKind::Cut),
                crease(11, [0.0, s / 2.0], [s / 2.0, s / 2.0], CreaseKind::Valley),
                crease(12, [s / 2.0, s / 2.0], [s, s / 2.0], CreaseKind::Valley),
            ],
            tol(s),
        )
        .unwrap();
        assert_eq!(arr.facets.len(), 4);
        assert_eq!(arr.hinges.len(), 2);
        let folded = fold(
            &arr,
            &angles(&arr, &[(11, PI / 2.0), (12, PI / 4.0)]),
            0,
            tol(s),
            ANGULAR,
        )
        .unwrap();
        assert_eq!(folded.roots.len(), 2);
        let made = folded_sheet(&arr, &folded, &frame(), &tolerance(s)).unwrap();
        assert_eq!(made.body.shells.len(), 2);
        let area = craft3d_topo::mass_properties(&made.body).unwrap().area;
        assert!(close(area, s * s, 1e-12 * s * s));
        // The cut's two sides are two edges, its boundary ends two
        // vertices each.
        let cut_edges = made
            .origins
            .edges
            .iter()
            .filter(|o| o.sources == [10])
            .count();
        assert_eq!(cut_edges, 4);
        assert!(
            first_meeting(&arr, &folded, tol(s), ANGULAR)
                .unwrap()
                .is_none()
        );
        // Kawasaki and Maekawa are about vertices joined all round.
        let (checks, cuts) = vertex_checks(&arr, ANGULAR);
        assert!(checks.is_empty());
        assert_eq!(cuts.len(), 1);
    }
}

#[test]
fn a_sheet_with_a_hole_folds_and_keeps_the_hole() {
    for scale in SCALES {
        let s = scale;
        let mut boundary = square(4.0 * s);
        // A hole, clockwise, in the left half.
        let hole = [
            [0.5 * s, 1.0 * s],
            [0.5 * s, 3.0 * s],
            [1.5 * s, 3.0 * s],
            [1.5 * s, 1.0 * s],
        ];
        for i in 0..4 {
            boundary.push(BoundarySegment {
                id: 20 + i as u64,
                start: hole[i],
                end: hole[(i + 1) % 4],
                reversed: false,
            });
        }
        let arr = arrange(
            &boundary,
            &[crease(
                10,
                [2.0 * s, 0.0],
                [2.0 * s, 4.0 * s],
                CreaseKind::Mountain,
            )],
            tol(s),
        )
        .unwrap();
        assert_eq!(arr.facets.len(), 2);
        assert_eq!(arr.facets.iter().filter(|f| f.holes.len() == 1).count(), 1);
        let folded = fold(&arr, &angles(&arr, &[(10, PI / 2.0)]), 0, tol(s), ANGULAR).unwrap();
        let made = folded_sheet(&arr, &folded, &frame(), &tolerance(s)).unwrap();
        let area = craft3d_topo::mass_properties(&made.body).unwrap().area;
        assert!(close(area, 16.0 * s * s - 2.0 * s * s, 1e-11 * s * s));
    }
}

#[test]
fn what_a_crease_pattern_must_be() {
    let s = 1.0;
    let t = tol(1.0);
    let v = CreaseKind::Valley;
    // Reaching outside.
    let e = arrange(&square(s), &[crease(10, [0.5, 0.0], [0.5, 1.5], v)], t);
    assert!(
        matches!(
            e,
            Err(FoldError::Outside { id: 10 }) | Err(FoldError::Dangling { id: 10 })
        ),
        "{e:?}"
    );
    // Ending inside a facet.
    let e = arrange(&square(s), &[crease(10, [0.5, 0.0], [0.5, 0.6], v)], t);
    assert!(matches!(e, Err(FoldError::Dangling { id: 10 })), "{e:?}");
    // Floating inside.
    let e = arrange(&square(s), &[crease(10, [0.2, 0.2], [0.6, 0.6], v)], t);
    assert!(matches!(e, Err(FoldError::Dangling { id: 10 })), "{e:?}");
    // Along each other, and along the boundary.
    let e = arrange(
        &square(s),
        &[
            crease(10, [0.5, 0.0], [0.5, 1.0], v),
            crease(11, [0.5, 0.2], [0.5, 1.0], v),
        ],
        t,
    );
    assert!(matches!(e, Err(FoldError::Overlap { .. })), "{e:?}");
    let e = arrange(&square(s), &[crease(10, [0.2, 0.0], [0.8, 0.0], v)], t);
    assert!(matches!(e, Err(FoldError::Overlap { .. })), "{e:?}");
    // A slit.
    let e = arrange(
        &square(s),
        &[crease(10, [0.5, 0.0], [0.5, 0.6], CreaseKind::Cut)],
        t,
    );
    assert!(matches!(e, Err(FoldError::Slit { id: 10 })), "{e:?}");
    // Too short, not finite.
    let e = arrange(&square(s), &[crease(10, [0.5, 0.0], [0.5, 1e-12], v)], t);
    assert!(matches!(e, Err(FoldError::TooShort { id: 10 })), "{e:?}");
    let e = arrange(&square(s), &[crease(10, [0.5, 0.0], [f64::NAN, 1.0], v)], t);
    assert!(matches!(e, Err(FoldError::NotFinite { id: 10 })), "{e:?}");
    assert!(matches!(
        arrange(&square(s), &[], 0.0),
        Err(FoldError::Tolerance)
    ));
    // A crease on no loop, solved for: nothing determines it.
    let arr = arrange(&square(s), &[crease(10, [0.5, 0.0], [0.5, 1.0], v)], t).unwrap();
    let e = fold(&arr, &angles(&arr, &[]), 0, t, ANGULAR);
    assert!(
        matches!(e, Err(FoldError::Undetermined { ref ids }) if ids == &[10]),
        "{e:?}"
    );
    // An angle past 180 degrees.
    let e = fold(&arr, &[Angle::Given(4.0)], 0, t, ANGULAR);
    assert!(matches!(e, Err(FoldError::AngleRange { id: 10 })), "{e:?}");
    // A degree-4 vertex whose solved crease is marked the wrong way.
    let (a, b) = (70f64.to_radians(), 50f64.to_radians());
    let arr = single_vertex(1.0, &[0.0, a, a + b, PI + b], &[v, v, v, v]);
    let e = fold(&arr, &angles(&arr, &[(10, 0.5)]), 0, t, ANGULAR);
    assert!(
        matches!(
            e,
            Err(FoldError::WrongWay { .. })
                | Err(FoldError::NotClosing { .. })
                | Err(FoldError::NotFollowed { .. })
        ),
        "{e:?}"
    );
}

#[test]
fn solved_creases_need_an_angle_that_folds_and_contact_fails_closed() {
    use CreaseKind::{Mountain as M, Valley as V};
    let t = tol(1.0);
    let (a, b) = (70f64.to_radians(), 50f64.to_radians());
    let arr = single_vertex(1.0, &[0.0, a, a + b, PI + b], &[V, M, V, V]);
    // No angle at all, or only zeros: at the flat sheet the branches of
    // the vertex meet, and nothing says how the solved creases go.
    for given in [vec![], vec![(10, 0.0)]] {
        let e = fold(&arr, &angles(&arr, &given), 0, t, ANGULAR);
        assert!(
            matches!(e, Err(FoldError::Undetermined { .. })),
            "{given:?}: {e:?}"
        );
    }
    // A facet that cannot be placed is an error, never "no contact".
    let mut folded = fold(&arr, &angles(&arr, &[(10, 0.3)]), 0, t, ANGULAR).unwrap();
    folded.transforms.pop();
    assert!(first_meeting(&arr, &folded, t, ANGULAR).is_err());
}

#[test]
fn a_small_sheet_far_from_the_origin_folds_as_one_at_the_origin() {
    use CreaseKind::{Mountain as M, Valley as V};
    let (a, b) = (70f64.to_radians(), 50f64.to_radians());
    let dirs = [0.0, a, a + b, PI + b];
    let r1 = 30f64.to_radians();
    let p = ((a + b) / 2.0).cos() / ((a - b) / 2.0).cos();
    let r2 = 2.0 * (-p * (r1 / 2.0).tan()).atan();
    let want = [r1, r2, r1, -r2];
    for far in [0.0, 1e7] {
        // A sheet of side 2 about (far, far), the tolerance 1e-7.
        let at = |q: [f64; 2]| [q[0] + far, q[1] + far];
        let boundary = polygon(&[
            at([-1.0, -1.0]),
            at([1.0, -1.0]),
            at([1.0, 1.0]),
            at([-1.0, 1.0]),
        ]);
        let creases: Vec<Crease> = dirs
            .iter()
            .zip([V, M, V, V])
            .enumerate()
            .map(|(i, (&d, kind))| {
                let (c, sn) = (d.cos(), d.sin());
                let r = 1.0 / c.abs().max(sn.abs());
                crease(10 + i as u64, at([0.0, 0.0]), at([r * c, r * sn]), kind)
            })
            .collect();
        let arr = arrange(&boundary, &creases, 1e-7).unwrap();
        let folded = fold(&arr, &angles(&arr, &[(10, r1)]), 0, 1e-7, ANGULAR).unwrap();
        for (id, w) in (10..14).zip(want) {
            let got = solved(&arr, &folded, id)[0];
            assert!(close(got, w, 1e-7), "far {far}: e{id} {got} vs {w}");
        }
        assert!(folded.gap <= 1e-7, "far {far}: gap {}", folded.gap);
    }
}

#[test]
fn folding_is_deterministic() {
    let (boundary, creases) = miura(1.0, 3, 3, 1.0, 0.4, 1.0);
    let build = || {
        let arr = arrange(&boundary, &creases, tol(1.0)).unwrap();
        let driver = creases
            .iter()
            .find(|c| c.start[1] > 0.0 && c.start[1] == c.end[1])
            .unwrap()
            .id;
        let folded = fold(&arr, &angles(&arr, &[(driver, 1.0)]), 0, tol(1.0), ANGULAR).unwrap();
        folded_sheet(&arr, &folded, &frame(), &tolerance(1.0)).unwrap()
    };
    let (a, b) = (build(), build());
    let bits = |r: &craft3d_kernel::extrude::OpResult| -> Vec<u64> {
        r.body
            .vertices
            .iter()
            .flat_map(|v| {
                [
                    v.point.x.to_bits(),
                    v.point.y.to_bits(),
                    v.point.z.to_bits(),
                ]
            })
            .collect()
    };
    assert_eq!(bits(&a), bits(&b));
    assert_eq!(a.origins, b.origins);
}
