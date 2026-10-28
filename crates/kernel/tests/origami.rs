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
    arrange, first_meeting, fold, fold_flat, fold_with, folded_sheet, layer_order, vertex_checks,
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
fn a_miura_ori_sheet_of_steep_parallelograms_folds_by_its_closed_form() {
    // The parallelograms' acute angle 88 degrees: the zigzag creases fold
    // 1 / cos 88° = 28.7 times as fast as the straight ones near flat.
    let gamma = 88f64.to_radians();
    let (l, h) = (1.0, 1.0);
    let d = h / gamma.tan();
    let (boundary, creases) = miura(1.0, 3, 3, l, d, h);
    let arr = arrange(&boundary, &creases, tol(1.0)).unwrap();
    let driver = creases
        .iter()
        .find(|c| c.start[1] > 0.0 && c.start[1] == c.end[1])
        .unwrap()
        .id;
    for degrees in [2.0f64, 10.0, 30.0] {
        let rho = degrees.to_radians();
        let folded = fold(&arr, &angles(&arr, &[(driver, rho)]), 0, tol(1.0), ANGULAR).unwrap();
        let zig = 2.0 * ((rho / 2.0).tan() / gamma.cos()).atan();
        for (hinge, a) in arr.hinges.iter().zip(&folded.angles) {
            let c = &arr.creases[hinge.crease];
            let want = if c.start[1] == c.end[1] { rho } else { zig };
            assert!(
                close(a.abs(), want, 1e-8),
                "{degrees}: crease {} folds {a} not {want}",
                c.id
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
    // A slit is a cut inside one facet (built since 2026-10-28:
    // `slits_and_slots_are_cuts_with_one_facet_on_both_sides`).
    let e = arrange(
        &square(s),
        &[crease(10, [0.5, 0.0], [0.5, 0.6], CreaseKind::Cut)],
        t,
    )
    .unwrap();
    assert_eq!((e.facets.len(), e.hinges.len()), (1, 0));
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
    // No angle at all: at the flat sheet the branches of the vertex meet,
    // and nothing says how the solved creases go. (A driving angle of 0
    // does: both branches leave the flat sheet only as it turns, so the
    // flat sheet is the fold; `a_driving_angle_of_zero_is_the_flat_sheet`.)
    let e = fold(&arr, &angles(&arr, &[]), 0, t, ANGULAR);
    assert!(matches!(e, Err(FoldError::Undetermined { .. })), "{e:?}");
    // Mode B of a vertex whose sectors nearly pair up (61 and 59 degrees):
    // near the flat sheet its solved creases fold 50 times as fast as the
    // driving one. It folds, by the closed form.
    let (a2, b2) = (61f64.to_radians(), 59f64.to_radians());
    let steep = single_vertex(1.0, &[0.0, a2, a2 + b2, PI + b2], &[V, V, M, V]);
    let q = ((a2 + b2) / 2.0).sin() / ((a2 - b2) / 2.0).sin();
    for degrees in [1.0f64, 3.0, 10.0] {
        let r1 = degrees.to_radians();
        let folded = fold(&steep, &angles(&steep, &[(10, r1)]), 0, t, ANGULAR).unwrap();
        let r2 = 2.0 * (q * (r1 / 2.0).tan()).atan();
        for (id, w) in (10..14).zip([r1, r2, -r1, r2]) {
            let got = solved(&steep, &folded, id)[0];
            assert!(close(got, w, 1e-8), "{degrees}: e{id} {got} vs {w}");
        }
    }
    // A facet that cannot be placed is an error, never "no contact".
    let mut folded = fold(&arr, &angles(&arr, &[(10, 0.3)]), 0, t, ANGULAR).unwrap();
    folded.transforms.pop();
    assert!(first_meeting(&arr, &folded, t, ANGULAR).is_err());
}

#[test]
fn two_branches_the_kinds_do_not_tell_apart_are_refused_as_ambiguous() {
    use CreaseKind::Valley as V;
    // A degree-4 vertex whose three solved creases have no kind (either
    // way): both its modes keep every kind, so the fold is ambiguous, and
    // the creases that differ between them are named (all three: mode A
    // and mode B differ in every solved crease). With kinds it is not.
    let t = tol(1.0);
    let (a, b) = (70f64.to_radians(), 50f64.to_radians());
    let arr = single_vertex(1.0, &[0.0, a, a + b, PI + b], &[V, V, V, V]);
    let open: Vec<Angle> = arr
        .hinges
        .iter()
        .map(|h| match arr.creases[h.crease].id {
            10 => Angle::Given(0.4),
            _ => Angle::Free { sign: 0.0 },
        })
        .collect();
    let e = fold(&arr, &open, 0, t, ANGULAR);
    assert!(
        matches!(e, Err(FoldError::Ambiguous { ref ids }) if ids == &[11, 12, 13]),
        "{e:?}"
    );
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

/// A driving angle of exactly 0 is the flat sheet, and a valid fold: a
/// degree-4 vertex driven by one crease (both modes) and a Miura-ori sheet
/// driven by one straight piece fold to it, every solved angle 0 and
/// every facet in place, and fold from it and back to it by the closed
/// form. At 0 the rank of the loops' derivative drops (every crease turns
/// about an axis in the sheet's plane), which is not freedom: the waterbomb
/// with only its midline given, at 0 as at any angle, is still
/// undetermined (one diagonal can fold alone), and so is a vertex with no
/// angle given at all.
#[test]
fn a_driving_angle_of_zero_is_the_flat_sheet() {
    use CreaseKind::{Mountain as M, Valley as V};
    let flat = |folded: &Folded, s: f64, what: &str| {
        for a in &folded.angles {
            assert!(a.abs() <= 1e-12, "{what}: an angle {a}");
        }
        for t in &folded.transforms {
            for p in [Vec3::ZERO, Vec3::new(s, -s, 0.0), Vec3::new(0.0, s, s)] {
                assert!(t.point(p).distance(p) <= 1e-12 * s, "{what}: moved");
            }
        }
    };
    for scale in SCALES {
        let (alpha, beta) = (70f64.to_radians(), 50f64.to_radians());
        let dirs = [0.0, alpha, alpha + beta, PI + beta];
        // Mode A, ρ3 = ρ1, ρ4 = −ρ2, and mode B, ρ3 = −ρ1, ρ4 = ρ2.
        let p = ((alpha + beta) / 2.0).cos() / ((alpha - beta) / 2.0).cos();
        let q = ((alpha + beta) / 2.0).sin() / ((alpha - beta) / 2.0).sin();
        let mode_a = |r1: f64| {
            let r2 = 2.0 * (-p * (r1 / 2.0).tan()).atan();
            [r1, r2, r1, -r2]
        };
        let mode_b = |r1: f64| {
            let r2 = 2.0 * (q * (r1 / 2.0).tan()).atan();
            [r1, r2, -r1, r2]
        };
        let kb = if mode_b(0.5)[1] > 0.0 { V } else { M };
        for (kinds, mode) in [
            ([V, M, V, V], &mode_a as &dyn Fn(f64) -> [f64; 4]),
            ([V, kb, M, kb], &mode_b),
        ] {
            let arr = single_vertex(scale, &dirs, &kinds);
            for zero in [0.0, -0.0] {
                let folded = fold(&arr, &angles(&arr, &[(10, zero)]), 0, tol(scale), ANGULAR)
                    .unwrap_or_else(|e| panic!("{kinds:?} at {zero}: {e:?}"));
                flat(&folded, scale, "the vertex at 0");
                let made = folded_sheet(&arr, &folded, &frame(), &tolerance(scale)).unwrap();
                let area = craft3d_topo::mass_properties(&made.body).unwrap().area;
                assert!(close(area, 4.0 * scale * scale, 1e-12 * scale * scale));
                assert!(
                    first_meeting(&arr, &folded, tol(scale), ANGULAR)
                        .unwrap()
                        .is_none()
                );
            }
            // From 0 and back to it.
            for degrees in [0.0f64, 0.5, 10.0, 60.0, 10.0, 0.5, 0.0] {
                let r1 = degrees.to_radians();
                let folded = fold(&arr, &angles(&arr, &[(10, r1)]), 0, tol(scale), ANGULAR)
                    .unwrap_or_else(|e| panic!("{kinds:?} at {degrees}: {e:?}"));
                let want = mode(r1);
                for (id, w) in (10..14).zip(want) {
                    let got = solved(&arr, &folded, id)[0];
                    assert!(close(got, w, 1e-9), "{degrees}: e{id} {got} vs {w}");
                }
            }
        }
        // No angle at all: still undetermined.
        let arr = single_vertex(scale, &dirs, &[V, M, V, V]);
        let e = fold(&arr, &angles(&arr, &[]), 0, tol(scale), ANGULAR);
        assert!(matches!(e, Err(FoldError::Undetermined { .. })), "{e:?}");

        // A Miura-ori sheet driven at 0.
        let (boundary, creases) = miura(scale, 4, 4, 1.0, 0.5, 1.0);
        let arr = arrange(&boundary, &creases, tol(scale)).unwrap();
        let driver = creases
            .iter()
            .find(|c| c.start[1] > 0.0 && c.start[1] == c.end[1])
            .unwrap()
            .id;
        let folded = fold(
            &arr,
            &angles(&arr, &[(driver, 0.0)]),
            0,
            tol(scale),
            ANGULAR,
        )
        .unwrap_or_else(|e| panic!("the Miura sheet at 0: {e:?}"));
        flat(&folded, scale, "the Miura sheet at 0");

        // The waterbomb with its midline at 0: a diagonal folds alone.
        let s = scale;
        let boundary = polygon(&[[-s, -s], [s, -s], [s, s], [-s, s]]);
        let creases = [
            crease(10, [-s, 0.0], [s, 0.0], M),
            crease(11, [-s, -s], [s, s], V),
            crease(12, [s, -s], [-s, s], V),
        ];
        let arr = arrange(&boundary, &creases, tol(s)).unwrap();
        let loose = fold(&arr, &angles(&arr, &[(10, 0.0)]), 0, tol(s), ANGULAR);
        assert!(
            matches!(loose, Err(FoldError::Undetermined { .. })),
            "{loose:?}"
        );
        // All three given at 0: the flat sheet.
        let all = fold(
            &arr,
            &angles(&arr, &[(10, 0.0), (11, 0.0), (12, 0.0)]),
            0,
            tol(s),
            ANGULAR,
        )
        .unwrap();
        flat(&all, s, "the waterbomb given flat");
    }
}

/// A joint folds through 0 either way, a crease (radius 0) and a bend
/// (radius r): two bodies joined by two joints on one axis (a loop), one
/// given and one solved with no kind. At every angle from −30 to 30
/// degrees and back, 0 and −0 among them, the solved joint takes the given
/// angle, and a point of the right body `w` from the axis lands where
/// rolling the strip `|ρ| r` onto the cylinder of radius `r` and turning by
/// ρ puts it (for a crease, the turn alone): at ±0 in place, at −ρ the
/// mirror image of ρ in the sheet's plane.
#[test]
fn a_crease_and_a_bend_fold_through_zero_both_ways() {
    use craft3d_kernel::origami::{Joint, Linkage, fold_linkage};
    for scale in SCALES {
        let t = tol(scale);
        for r in [0.0, 2.4 * scale] {
            let joint = |id: u64| Joint {
                left: 0,
                right: 1,
                point: Vec3::ZERO,
                direction: Vec3::X,
                side: Vec3::Y,
                reach: 10.0 * scale,
                id,
                radius: r,
                across: -Vec3::Y,
            };
            let link = Linkage {
                bodies: 2,
                joints: vec![joint(1), joint(2)],
                size: 20.0 * scale,
            };
            let (x, w) = (scale, 7.0 * scale);
            let expect = |rho: f64| {
                let sigma = if rho.is_sign_negative() { -1.0 } else { 1.0 };
                let y0 = -w + rho.abs() * r;
                Vec3::new(
                    x,
                    y0 * rho.cos() - sigma * r * rho.sin(),
                    sigma * r - y0 * rho.sin() - sigma * r * rho.cos(),
                )
            };
            let sweep = [
                -30.0f64, -1.0, -1e-4, -0.0, 0.0, 1e-4, 1.0, 30.0, 1.0, 1e-4, 0.0, -0.0, -1e-4,
                -1.0, -30.0,
            ];
            for degrees in sweep {
                let rho = degrees.to_radians();
                let folded = fold_linkage(
                    &link,
                    &[Angle::Given(rho), Angle::Free { sign: 0.0 }],
                    0,
                    t,
                    ANGULAR,
                )
                .unwrap_or_else(|e| panic!("r {r} at {degrees}: {e:?}"));
                assert!(
                    close(folded.angles[1], rho, 1e-9),
                    "r {r} at {degrees}: {:?}",
                    folded.angles
                );
                let got = folded.transforms[1].point(Vec3::new(x, -w, 0.0));
                let want = expect(rho);
                assert!(
                    got.distance(want) <= 1e-9 * scale,
                    "r {r} at {degrees}: {got:?} against {want:?}"
                );
                if degrees == 0.0 {
                    assert!(got.distance(Vec3::new(x, -w, 0.0)) <= 1e-12 * scale);
                }
            }
        }
    }
}

/// A degree-4 vertex at `centre` in a square of half-side `s`: its
/// boundary (curve ids from `first_boundary`) and its creases out to the
/// boundary (ids from `first_crease`).
fn vertex_piece(
    centre: [f64; 2],
    s: f64,
    directions: &[f64],
    kinds: &[CreaseKind],
    first_boundary: u64,
    first_crease: u64,
) -> (Vec<BoundarySegment>, Vec<Crease>) {
    let [cx, cy] = centre;
    let mut boundary = polygon(&[
        [cx - s, cy - s],
        [cx + s, cy - s],
        [cx + s, cy + s],
        [cx - s, cy + s],
    ]);
    for b in &mut boundary {
        b.id += first_boundary - 1;
    }
    let creases = directions
        .iter()
        .zip(kinds)
        .enumerate()
        .map(|(i, (&a, &kind))| {
            let (c, sn) = (a.cos(), a.sin());
            let t = s / c.abs().max(sn.abs());
            crease(
                first_crease + i as u64,
                centre,
                [cx + t * c, cy + t * sn],
                kind,
            )
        })
        .collect();
    (boundary, creases)
}

#[test]
fn two_branches_that_nearly_coincide_are_told_apart_by_how_they_part() {
    use CreaseKind::{Mountain as M, Valley as V};
    // Two pieces folded at once. Piece one: a degree-4 vertex of sectors
    // 100, 35, 80 and 145 degrees driven by e10 at a tenth of a degree, its
    // other three creases e11..e13 either without a kind or with the kinds
    // of one mode. Piece two: the 61/59-degree vertex (kinds set, mode B)
    // driven by e20 at 10 degrees, whose solved creases fold some fifty
    // times as fast. At the first step (1 degree of the largest given
    // angle) the two modes of piece one differ by less than a thousandth of
    // the largest solved angle: two real branches that a fixed relative
    // threshold took for one, so one was followed silently. They part in
    // proportion to the step (the solver's play does not), so they are told
    // apart: without kinds the fold is ambiguous, naming e11..e13; with the
    // kinds of either mode it follows that mode by its closed form.
    let t = tol(1.0);
    let (a, b) = (100f64.to_radians(), 35f64.to_radians());
    let dirs1 = [0.0, a, a + b, PI + b];
    let (a2, b2) = (61f64.to_radians(), 59f64.to_radians());
    let dirs2 = [0.0, a2, a2 + b2, PI + b2];
    let r_slow = 0.1f64.to_radians();
    let r_fast = 10f64.to_radians();
    let ka = -((a + b) / 2.0).cos() / ((a - b) / 2.0).cos();
    let kb = ((a + b) / 2.0).sin() / ((a - b) / 2.0).sin();
    let q = ((a2 + b2) / 2.0).sin() / ((a2 - b2) / 2.0).sin();
    let mode_a = |r: f64| {
        let r2 = 2.0 * (ka * (r / 2.0).tan()).atan();
        [r2, r, -r2]
    };
    let mode_b = |r: f64| {
        let r2 = 2.0 * (kb * (r / 2.0).tan()).atan();
        [r2, -r, r2]
    };
    // Near coincidence, by the closed forms at the first step.
    let first = 1f64.to_radians();
    let slow_first = r_slow * first / r_fast;
    let fast_first = 2.0 * (q * (first / 2.0).tan()).atan();
    let apart = mode_a(slow_first)
        .iter()
        .zip(mode_b(slow_first))
        .fold(0.0f64, |m, (x, y)| m.max((x - y).abs()));
    assert!(apart < 1e-3 * fast_first, "{apart} vs {fast_first}");
    assert!(apart > 1e3 * (ANGULAR + t), "{apart}");

    let build = |kinds1: [CreaseKind; 4]| {
        let (mut boundary, mut creases) = vertex_piece([0.0, 0.0], 1.0, &dirs1, &kinds1, 1, 10);
        let (b2, c2) = vertex_piece([4.0, 0.0], 1.0, &dirs2, &[V, V, M, V], 101, 20);
        boundary.extend(b2);
        creases.extend(c2);
        arrange(&boundary, &creases, t).unwrap()
    };
    let given = [(10, r_slow), (20, r_fast)];

    // Without kinds on e11..e13: ambiguous, naming them.
    let arr = build([V, V, V, V]);
    let open: Vec<Angle> = arr
        .hinges
        .iter()
        .map(|h| {
            let c = &arr.creases[h.crease];
            match c.id {
                10 => Angle::Given(r_slow),
                20 => Angle::Given(r_fast),
                11..=13 => Angle::Free { sign: 0.0 },
                _ => Angle::Free {
                    sign: c.kind.sign(),
                },
            }
        })
        .collect();
    let e = fold(&arr, &open, 0, t, ANGULAR);
    assert!(
        matches!(e, Err(FoldError::Ambiguous { ref ids }) if ids == &[11, 12, 13]),
        "{e:?}"
    );

    // With the kinds of one mode, that mode, by its closed form.
    let fast = [2.0 * (q * (r_fast / 2.0).tan()).atan(), -r_fast];
    for (kinds, want) in [
        ([V, M, V, V], mode_a(r_slow)),
        ([V, V, M, V], mode_b(r_slow)),
    ] {
        let arr = build(kinds);
        let folded = fold(&arr, &angles(&arr, &given), 0, t, ANGULAR).unwrap();
        for (id, w) in (11..14).zip(want) {
            let got = solved(&arr, &folded, id)[0];
            assert!(close(got, w, 1e-9), "{kinds:?}: e{id} {got} vs {w}");
        }
        for (id, w) in [(21, fast[0]), (22, fast[1]), (23, fast[0])] {
            let got = solved(&arr, &folded, id)[0];
            assert!(close(got, w, 1e-8), "{kinds:?}: e{id} {got} vs {w}");
        }
    }
}

/// The sparse solve (forced with `fold_with(.., 0)`) against the dense one
/// and the closed forms: a degree-4 vertex, the same vertex with a hole
/// where the vertex was (its loop goes round the hole, through no
/// vertex), the waterbomb (one diagonal solved; the midline alone
/// undetermined), a 4 × 4 Miura sheet, a cut sheet; and bit for bit the
/// same run twice.
#[test]
fn the_sparse_solve_agrees_with_the_dense_one_and_the_closed_forms() {
    use CreaseKind::{Mountain as M, Valley as V};
    for scale in SCALES {
        let t = tol(scale);
        // A degree-4 vertex, mode A.
        let (a, b) = (70f64.to_radians(), 50f64.to_radians());
        let dirs = [0.0, a, a + b, PI + b];
        let p = ((a + b) / 2.0).cos() / ((a - b) / 2.0).cos();
        for rho in [10f64, 60.0, 120.0] {
            let r1 = rho.to_radians();
            let r2 = 2.0 * (-p * (r1 / 2.0).tan()).atan();
            let arr = single_vertex(scale, &dirs, &[V, M, V, V]);
            let given = angles(&arr, &[(10, r1)]);
            let dense = fold_with(&arr, &given, 0, t, ANGULAR, usize::MAX).unwrap();
            let sparse = fold_with(&arr, &given, 0, t, ANGULAR, 0).unwrap();
            for (id, w) in (10..14).zip([r1, r2, r1, -r2]) {
                for f in [&dense, &sparse] {
                    let got = solved(&arr, f, id)[0];
                    assert!(close(got, w, 1e-9), "{rho}: e{id} {got} vs {w}");
                }
            }
            // The same vertex with a hole where the vertex was.
            let s = scale;
            let mut boundary = polygon(&[[-s, -s], [s, -s], [s, s], [-s, s]]);
            let h = 0.2 * s;
            let hole = [[-h, -h], [-h, h], [h, h], [h, -h]];
            for i in 0..4 {
                boundary.push(BoundarySegment {
                    id: 20 + i as u64,
                    start: hole[i],
                    end: hole[(i + 1) % 4],
                    reversed: false,
                });
            }
            let creases: Vec<Crease> = dirs
                .iter()
                .zip([V, M, V, V])
                .enumerate()
                .map(|(i, (&d, kind))| {
                    let (c, sn) = (d.cos(), d.sin());
                    let m = c.abs().max(sn.abs());
                    crease(
                        10 + i as u64,
                        [h / m * c, h / m * sn],
                        [s / m * c, s / m * sn],
                        kind,
                    )
                })
                .collect();
            let ring = arrange(&boundary, &creases, t).unwrap();
            assert_eq!(ring.facets.len(), 4);
            let given = angles(&ring, &[(10, r1)]);
            let sparse = fold_with(&ring, &given, 0, t, ANGULAR, 0).unwrap();
            for (id, w) in (10..14).zip([r1, r2, r1, -r2]) {
                let got = solved(&ring, &sparse, id)[0];
                assert!(close(got, w, 1e-9), "ring {rho}: e{id} {got} vs {w}");
            }
        }
        // The waterbomb.
        let s = scale;
        let boundary = polygon(&[[-s, -s], [s, -s], [s, s], [-s, s]]);
        let creases = [
            crease(10, [-s, 0.0], [s, 0.0], M),
            crease(11, [-s, -s], [s, s], V),
            crease(12, [s, -s], [-s, s], V),
        ];
        let arr = arrange(&boundary, &creases, t).unwrap();
        for w in [0.2, 1.0] {
            let (mu, delta) = waterbomb_angles(w);
            let given = angles(&arr, &[(10, mu), (11, delta)]);
            let sparse = fold_with(&arr, &given, 0, t, ANGULAR, 0).unwrap();
            for a in solved(&arr, &sparse, 12) {
                assert!(close(a, delta, 1e-9), "{a} vs {delta}");
            }
            let loose = fold_with(&arr, &angles(&arr, &[(10, mu)]), 0, t, ANGULAR, 0);
            assert!(
                matches!(loose, Err(FoldError::Undetermined { .. })),
                "{loose:?}"
            );
        }
        // A Miura sheet of 4 × 4, against the dense solve and the closed
        // form; twice, bit for bit.
        let (l, d, hh): (f64, f64, f64) = (1.0, 0.5, 1.0);
        let gamma = (hh / d).atan();
        let (boundary, creases) = miura(scale, 4, 4, l, d, hh);
        let arr = arrange(&boundary, &creases, t).unwrap();
        let driver = creases
            .iter()
            .find(|c| c.start[1] > 0.0 && c.start[1] == c.end[1])
            .unwrap()
            .id;
        for degrees in [20.0f64, 140.0] {
            let rho = degrees.to_radians();
            let given = angles(&arr, &[(driver, rho)]);
            let dense = fold_with(&arr, &given, 0, t, ANGULAR, usize::MAX).unwrap();
            let sparse = fold_with(&arr, &given, 0, t, ANGULAR, 0).unwrap();
            let again = fold_with(&arr, &given, 0, t, ANGULAR, 0).unwrap();
            assert_eq!(sparse, again);
            let zig = 2.0 * ((rho / 2.0).tan() / gamma.cos()).atan();
            for (i, hinge) in arr.hinges.iter().enumerate() {
                let c = &arr.creases[hinge.crease];
                let want = c.kind.sign() * if c.start[1] == c.end[1] { rho } else { zig };
                assert!(close(sparse.angles[i], want, 1e-8), "{degrees}: e{}", c.id);
                assert!(close(sparse.angles[i], dense.angles[i], 1e-8));
            }
        }
    }
}

/// A Miura sheet past the dense solve's 400 unknowns folds by its closed
/// form with the sparse solve, and how long it takes is printed (run with
/// `--release --nocapture` for the timings of docs/design/origami.md).
#[test]
fn a_large_miura_sheet_folds_by_the_sparse_solve() {
    let (l, d, h): (f64, f64, f64) = (1.0, 0.5, 1.0);
    let gamma = (h / d).atan();
    for (n, degrees) in [
        (16usize, &[20.0f64, 70.0][..]),
        (24, &[60.0]),
        (40, &[60.0]),
    ] {
        let (boundary, creases) = miura(1.0, n, n, l, d, h);
        let arr = arrange(&boundary, &creases, tol(1.0)).unwrap();
        assert!(arr.hinges.len() > 401, "{}", arr.hinges.len());
        let driver = creases
            .iter()
            .find(|c| c.start[1] > 0.0 && c.start[1] == c.end[1])
            .unwrap()
            .id;
        for &degrees in degrees {
            let rho = degrees.to_radians();
            if n == 16 {
                // The dense solve of the same sheet, for the timing.
                let clock = std::time::Instant::now();
                let dense = fold_with(
                    &arr,
                    &angles(&arr, &[(driver, rho)]),
                    0,
                    tol(1.0),
                    ANGULAR,
                    usize::MAX,
                )
                .unwrap();
                println!(
                    "Miura {n} x {n}, dense: {} angles at {degrees} degrees in {:.3} s",
                    arr.hinges.len() - 1,
                    clock.elapsed().as_secs_f64()
                );
                let zig = 2.0 * ((rho / 2.0).tan() / gamma.cos()).atan();
                for (hinge, a) in arr.hinges.iter().zip(&dense.angles) {
                    let c = &arr.creases[hinge.crease];
                    let want = c.kind.sign() * if c.start[1] == c.end[1] { rho } else { zig };
                    assert!(close(*a, want, 1e-8));
                }
            }
            let clock = std::time::Instant::now();
            let folded = fold(&arr, &angles(&arr, &[(driver, rho)]), 0, tol(1.0), ANGULAR).unwrap();
            println!(
                "Miura {n} x {n}: {} angles solved at {degrees} degrees in {:.3} s",
                arr.hinges.len() - 1,
                clock.elapsed().as_secs_f64()
            );
            let zig = 2.0 * ((rho / 2.0).tan() / gamma.cos()).atan();
            for (hinge, a) in arr.hinges.iter().zip(&folded.angles) {
                let c = &arr.creases[hinge.crease];
                let want = c.kind.sign() * if c.start[1] == c.end[1] { rho } else { zig };
                assert!(close(*a, want, 1e-8), "{degrees}: e{} {a} vs {want}", c.id);
            }
            let made = folded_sheet(&arr, &folded, &frame(), &tolerance(1.0)).unwrap();
            let area = craft3d_topo::mass_properties(&made.body).unwrap().area;
            let flat = (n * n) as f64 * l * h;
            assert!(close(area, flat, 1e-10 * flat));
        }
    }
}

/// A cut with one facet on both sides: a slit from the boundary, a slit
/// from a fold crease, and a slot (a cut that touches nothing) folded with
/// the facets they lie in. The cuts take no area, each is two edges of the
/// sheet (one per side), the mouth of the boundary slit two vertices, and
/// the slot moves with its facet; folded flat the two facets are layered.
#[test]
fn slits_and_slots_are_cuts_with_one_facet_on_both_sides() {
    use CreaseKind::{Cut, Valley as V};
    for scale in SCALES {
        let s = scale;
        let t = tol(scale);
        let creases = [
            crease(10, [0.5 * s, 0.0], [0.5 * s, s], V),
            // From the boundary into the left facet.
            crease(11, [0.25 * s, 0.0], [0.25 * s, 0.6 * s], Cut),
            // From the crease into the left facet.
            crease(12, [0.5 * s, 0.8 * s], [0.3 * s, 0.8 * s], Cut),
            // Inside the right facet, touching nothing.
            crease(13, [0.7 * s, 0.3 * s], [0.7 * s, 0.7 * s], Cut),
        ];
        let arr = arrange(&square(s), &creases, t).unwrap();
        assert_eq!(arr.facets.len(), 2);
        // The crease is split where the slit starts on it.
        assert_eq!(arr.hinges.len(), 2);
        for f in &arr.facets {
            assert!(close(f.area, 0.5 * s * s, 1e-12 * s * s), "{}", f.area);
        }
        let right = arr.facet_at([0.9 * s, 0.5 * s]).unwrap();
        assert_eq!(arr.facets[right].holes.len(), 1);
        let left = arr.facet_at([0.1 * s, 0.5 * s]).unwrap();
        let folded = fold(&arr, &angles(&arr, &[(10, PI / 2.0)]), left, t, ANGULAR).unwrap();
        let made = folded_sheet(&arr, &folded, &frame(), &tolerance(scale)).unwrap();
        let body = &made.body;
        assert_eq!(body.faces.len(), 2);
        // Boundary 7 pieces (the bottom split at the slit's mouth and the
        // crease, the top at the crease), the crease 2, each cut 2.
        assert_eq!(body.edges.len(), 7 + 2 + 2 * 3);
        // Corners 4, the crease's ends 2, the slit's mouth 2 and tip 1,
        // the crease slit's two ends 2, the slot's 2.
        assert_eq!(body.vertices.len(), 4 + 2 + 3 + 2 + 2);
        let area = craft3d_topo::mass_properties(body).unwrap().area;
        assert!(close(area, s * s, 1e-12 * s * s), "{area}");
        // The slot turned with the right facet: its ends at x = s/2,
        // z = 0.2 s.
        for y in [0.3, 0.7] {
            let want = Vec3::new(0.5 * s, y * s, 0.2 * s);
            assert!(
                body.vertices
                    .iter()
                    .any(|v| v.point.distance(want) <= 1e-12 * s),
                "{want:?}"
            );
        }
        assert!(first_meeting(&arr, &folded, t, ANGULAR).unwrap().is_none());
        // The vertex where the slit leaves the crease lies on a cut: not
        // asked of Kawasaki and Maekawa.
        let (checks, on_cuts) = vertex_checks(&arr, ANGULAR);
        assert!(checks.is_empty());
        assert!(!on_cuts.is_empty());
        // Folded flat: two layers.
        let flat = fold_flat(&arr, &angles(&arr, &[(10, PI)]), left, t).unwrap();
        let layers = layer_order(&arr, &flat, t).unwrap();
        assert_ne!(layers.layer[0], layers.layer[1]);
        let flat_body = folded_sheet(&arr, &flat, &frame(), &tolerance(scale)).unwrap();
        assert_eq!(flat_body.body.edges.len(), body.edges.len());
    }
    // A fold crease that ends inside a facet still dangles; a slot outside
    // the sheet is outside.
    let t = tol(1.0);
    let e = arrange(&square(1.0), &[crease(10, [0.5, 0.0], [0.5, 0.6], V)], t);
    assert!(matches!(e, Err(FoldError::Dangling { id: 10 })), "{e:?}");
    let e = arrange(&square(1.0), &[crease(13, [1.5, 0.3], [1.5, 0.7], Cut)], t);
    assert!(matches!(e, Err(FoldError::Outside { id: 13 })), "{e:?}");
}

/// A Miura sheet of 13 × 13 cells, 311 solved angles: the dense solve
/// (at most 400 unknowns) folds it by its closed form within the work
/// budget, at the sheet's own size and at a million times it, where the
/// first step's solves stop on stagnation, not on an absolute goal below
/// the rounding of long loops. The time is printed (`--nocapture`).
#[test]
fn a_dense_sheet_of_three_hundred_angles_folds_within_the_budget_at_any_scale() {
    let (l, d, h): (f64, f64, f64) = (1.0, 0.5, 1.0);
    let gamma = (h / d).atan();
    for scale in [1.0, 1e6] {
        let (boundary, creases) = miura(scale, 13, 13, l, d, h);
        let arr = arrange(&boundary, &creases, tol(scale)).unwrap();
        let solved = arr.hinges.len() - 1;
        assert!((300..=400).contains(&solved), "{solved}");
        let driver = creases
            .iter()
            .find(|c| c.start[1] > 0.0 && c.start[1] == c.end[1])
            .unwrap()
            .id;
        for degrees in [20.0f64, 60.0] {
            let rho = degrees.to_radians();
            let clock = std::time::Instant::now();
            let folded = fold(
                &arr,
                &angles(&arr, &[(driver, rho)]),
                0,
                tol(scale),
                ANGULAR,
            )
            .unwrap();
            println!(
                "Miura 13 x 13 at scale {scale:e}, dense: {solved} angles at {degrees} degrees in {:.3} s",
                clock.elapsed().as_secs_f64()
            );
            let zig = 2.0 * ((rho / 2.0).tan() / gamma.cos()).atan();
            for (hinge, a) in arr.hinges.iter().zip(&folded.angles) {
                let c = &arr.creases[hinge.crease];
                let want = c.kind.sign() * if c.start[1] == c.end[1] { rho } else { zig };
                assert!(
                    close(*a, want, 1e-8),
                    "{scale} {degrees}: e{} {a} vs {want}",
                    c.id
                );
            }
        }
    }
}

/// Folded flat, a slit's folded edge that lies on the line of another
/// folded edge and runs on past it: the stretch only the slit covers is a
/// cut (it may end inside a cell), the rest divides as before. A notched
/// square folded in half: the slit in the right half, from the right
/// edge, lands on the notch's top edge and runs on past it.
#[test]
fn a_slit_edge_running_on_past_a_collinear_edge_stays_a_cut_in_the_layers() {
    use CreaseKind::{Cut, Valley as V};
    for scale in SCALES {
        let s = scale;
        let t = tol(scale);
        let outline = polygon(&[
            [0.0, 0.4 * s],
            [0.2 * s, 0.4 * s],
            [0.2 * s, 0.0],
            [s, 0.0],
            [s, s],
            [0.0, s],
        ]);
        let creases = [
            crease(10, [0.5 * s, 0.0], [0.5 * s, s], V),
            crease(11, [s, 0.4 * s], [0.6 * s, 0.4 * s], Cut),
        ];
        let arr = arrange(&outline, &creases, t).unwrap();
        assert_eq!(arr.facets.len(), 2);
        let left = arr.facet_at([0.1 * s, 0.7 * s]).unwrap();
        let flat = fold_flat(&arr, &angles(&arr, &[(10, PI)]), left, t).unwrap();
        let layers = layer_order(&arr, &flat, t).unwrap();
        assert_ne!(layers.layer[0], layers.layer[1]);
    }
}

/// The first step's worst case on a dense sheet: a Miura sheet of 13 × 13
/// (311 solved angles) with six zigzag pieces without a kind (sign 0), so
/// every one of the 5 guesses is tried in all 2⁶ sign patterns, each with
/// its full and half solves. It folds by its closed form with at least
/// half of the work budget to spare (the work is printed): the sparse
/// solve is taken, its factor far cheaper than the dense one. Forced
/// dense, the same fold either folds by its closed form or is refused as
/// too much work, never anything else.
#[test]
fn the_first_steps_worst_case_on_a_dense_sheet_stays_within_half_the_budget() {
    use craft3d_kernel::origami::MAX_WORK;
    let (l, d, h): (f64, f64, f64) = (1.0, 0.5, 1.0);
    let gamma = (h / d).atan();
    let (boundary, creases) = miura(1.0, 13, 13, l, d, h);
    let arr = arrange(&boundary, &creases, tol(1.0)).unwrap();
    let driver = creases
        .iter()
        .find(|c| c.start[1] > 0.0 && c.start[1] == c.end[1])
        .unwrap()
        .id;
    let rho = 60f64.to_radians();
    let mut given = angles(&arr, &[(driver, rho)]);
    // Six zigzag pieces spread over the sheet, their kinds left open.
    let zigzag: Vec<usize> = arr
        .hinges
        .iter()
        .enumerate()
        .filter(|(_, hg)| {
            let c = &arr.creases[hg.crease];
            c.start[1] != c.end[1]
        })
        .map(|(k, _)| k)
        .collect();
    let open: Vec<usize> = (0..6)
        .map(|i| zigzag[(2 * i + 1) * zigzag.len() / 12])
        .collect();
    for &k in &open {
        given[k] = Angle::Free { sign: 0.0 };
    }
    let clock = std::time::Instant::now();
    let folded = fold(&arr, &given, 0, tol(1.0), ANGULAR).unwrap();
    println!(
        "Miura 13 x 13, six kinds open: {} of {MAX_WORK} work in {:.3} s",
        folded.work,
        clock.elapsed().as_secs_f64()
    );
    assert!(folded.work <= MAX_WORK / 2, "{}", folded.work);
    let zig = 2.0 * ((rho / 2.0).tan() / gamma.cos()).atan();
    let holds = |folded: &Folded| {
        for (hinge, a) in arr.hinges.iter().zip(&folded.angles) {
            let c = &arr.creases[hinge.crease];
            let want = c.kind.sign() * if c.start[1] == c.end[1] { rho } else { zig };
            assert!(close(*a, want, 1e-8), "e{} {a} vs {want}", c.id);
        }
    };
    holds(&folded);
    let clock = std::time::Instant::now();
    match fold_with(&arr, &given, 0, tol(1.0), ANGULAR, usize::MAX) {
        Ok(dense) => {
            println!(
                "forced dense: {} work in {:.3} s",
                dense.work,
                clock.elapsed().as_secs_f64()
            );
            holds(&dense);
        }
        Err(FoldError::TooMuchWork) => println!(
            "forced dense: too much work after {:.3} s",
            clock.elapsed().as_secs_f64()
        ),
        Err(e) => panic!("{e:?}"),
    }
}
