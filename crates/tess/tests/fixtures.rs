//! The trimmed reference solids of `craft3d_testkit::trimmed` are proven
//! right here before anything uses them as an oracle, and without the
//! tessellator: the validity checker finds no defect, the entity counts
//! are the ones stated, and the mass properties computed from the exact
//! B-rep match the closed forms.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_testkit::trimmed;
use craft3d_topo::{check, genus, mass_properties};

fn frames(scale: f64) -> [Frame; 2] {
    [
        Frame::WORLD,
        Frame::new(
            Vec3::new(1.0, -2.0, 0.5) * scale,
            Vec3::new(0.3, -0.4, 1.0),
            Vec3::new(1.0, 0.2, 0.0),
        )
        .unwrap(),
    ]
}

#[test]
fn trimmed_fixtures_are_valid_and_match_their_closed_forms() {
    for scale in [1e-3, 1.0, 1e6] {
        // The modelling tolerance scales with the model (docs/decisions.md).
        let tolerance = Tolerance::new(1e-6 * scale, 1e-9).unwrap();
        for frame in frames(scale) {
            for solid in trimmed::all(&frame, scale).unwrap() {
                let what = format!("{} at scale {scale}", solid.name);
                assert_eq!(check(&solid.body, &tolerance), vec![], "{what}");
                let body = &solid.body;
                assert_eq!(
                    (body.vertices.len(), body.edges.len(), body.faces.len()),
                    solid.counts,
                    "{what}"
                );
                assert_eq!(genus(body), Some(solid.genus), "{what}");
                let mass = mass_properties(body).unwrap();
                // 1e-9 relative: the integrals are Gauss–Legendre sums of
                // smooth integrands, exact to rounding for these shapes.
                assert!(
                    (mass.volume - solid.volume).abs() <= 1e-9 * solid.volume,
                    "{what}: volume {} vs {}",
                    mass.volume,
                    solid.volume
                );
                assert!(
                    (mass.area - solid.area).abs() <= 1e-9 * solid.area,
                    "{what}: area {} vs {}",
                    mass.area,
                    solid.area
                );
                let centroid = mass.centroid.unwrap();
                assert!(
                    centroid.distance(solid.centroid) <= 1e-9 * scale,
                    "{what}: centroid {centroid:?} vs {:?}",
                    solid.centroid
                );
            }
        }
    }
}

/// The domed block's area and centroid are taken from `mass_properties`,
/// so the test above proves nothing about them. Pin them by a third
/// route that shares no code with it: a fine midpoint sum over the roof
/// patch's own parametrisation, plus the walls as polygons under the
/// roof's boundary curves.
#[test]
fn the_domed_block_matches_a_direct_sum_over_its_roof() {
    use craft3d_geom::Surface;
    let (dx, dy, height) = (3.0, 2.0, 1.5);
    for nurbs in [false, true] {
        let solid = trimmed::domed_block(&Frame::WORLD, dx, dy, height, nurbs).unwrap();
        let roof = solid
            .body
            .faces
            .iter()
            .map(|f| &f.surface)
            .find(|s| matches!(s, Surface::Bezier(_) | Surface::Nurbs(_)))
            .unwrap();
        let n = 400;
        let h = 1.0 / f64::from(n);
        let (mut roof_area, mut volume, mut moment) = (0.0, 0.0, Vec3::ZERO);
        // Walls: the area under z(x) along each of the four sides.
        let mut walls = 0.0;
        for i in 0..n {
            let s = (f64::from(i) + 0.5) * h;
            for j in 0..n {
                let t = (f64::from(j) + 0.5) * h;
                let (p, du, dv) = roof.derivatives(s, t);
                roof_area += du.cross(dv).length() * h * h;
                // x and y are linear in (u, v): a column of height z.
                volume += p.z * dx * dy * h * h;
                moment += Vec3::new(p.x * p.z, p.y * p.z, p.z * p.z / 2.0) * (dx * dy * h * h);
            }
            walls += (roof.eval(s, 0.0).z + roof.eval(s, 1.0).z) * dx * h;
            walls += (roof.eval(0.0, s).z + roof.eval(1.0, s).z) * dy * h;
        }
        let area = roof_area + walls + dx * dy;
        // The midpoint rule on 400 × 400 cells of a cubic patch is good
        // to about h² ≈ 6e-6 relative.
        assert!(
            (area - solid.area).abs() <= 2e-5 * solid.area,
            "area {area} vs {}",
            solid.area
        );
        assert!(
            (volume - solid.volume).abs() <= 2e-5 * solid.volume,
            "volume {volume} vs {}",
            solid.volume
        );
        let centroid = moment * (1.0 / volume);
        assert!(
            centroid.distance(solid.centroid) <= 2e-5,
            "centroid {centroid:?} vs {:?}",
            solid.centroid
        );
    }
}

/// The pipe elbow and the sphere octant are valid bodies with the stated
/// counts. `mass_properties` confirms their values in the two tests
/// below; here the closed forms are checked against each other: the
/// elbow over a whole turn is the torus, and eight octants are the
/// sphere.
#[test]
fn the_elbow_and_the_octant_are_valid_bodies() {
    use craft3d_testkit::solids;
    for scale in [1e-3, 1.0, 1e6] {
        let tolerance = Tolerance::new(1e-6 * scale, 1e-9).unwrap();
        for frame in frames(scale) {
            let elbow = trimmed::pipe_elbow(&frame, 3.0 * scale, 1.0 * scale, 1.9).unwrap();
            let octant = trimmed::sphere_octant(&frame, 2.0 * scale).unwrap();
            for solid in [&elbow, &octant] {
                let what = format!("{} at scale {scale}", solid.name);
                assert_eq!(check(&solid.body, &tolerance), vec![], "{what}");
                let body = &solid.body;
                assert_eq!(
                    (body.vertices.len(), body.edges.len(), body.faces.len()),
                    solid.counts,
                    "{what}"
                );
                assert_eq!(genus(body), Some(solid.genus), "{what}");
            }
            let torus = solids::torus(&frame, 3.0 * scale, 1.0 * scale).unwrap();
            let disc = std::f64::consts::PI * scale * scale;
            let turn = std::f64::consts::TAU / 1.9;
            assert!((elbow.volume * turn - torus.volume).abs() < 1e-12 * torus.volume);
            assert!(((elbow.area - 2.0 * disc) * turn - torus.area).abs() < 1e-12 * torus.area);
            let sphere = solids::sphere(&frame, 2.0 * scale).unwrap();
            let quarter_disc = std::f64::consts::PI * 4.0 * scale * scale / 4.0;
            assert!((octant.volume * 8.0 - sphere.volume).abs() < 1e-12 * sphere.volume);
            assert!(
                ((octant.area - 3.0 * quarter_disc) * 8.0 - sphere.area).abs()
                    < 1e-12 * sphere.area
            );
        }
    }
}

/// Once a gap in `craft3d_topo::mass_properties`: a face bounded by loops
/// that wind around the *tube* of a torus. Along them du = 0, so the
/// integral over u alone came out as zero for the face.
#[test]
fn mass_properties_of_a_pipe_elbow() {
    let solid = trimmed::pipe_elbow(&Frame::WORLD, 3.0, 1.0, 1.2).unwrap();
    let mass = mass_properties(&solid.body).unwrap();
    assert!(
        (mass.area - solid.area).abs() <= 1e-9 * solid.area,
        "area {} vs {}",
        mass.area,
        solid.area
    );
    assert!(
        (mass.volume - solid.volume).abs() <= 1e-9 * solid.volume,
        "volume {} vs {}",
        mass.volume,
        solid.volume
    );
}

/// Once a second gap: a face whose boundary passes through a pole of a
/// sphere. In parameter space the pole is a whole edge of the region (u
/// runs from one meridian to the other at v = π/2), which no edge of the
/// body stands for.
#[test]
fn mass_properties_of_a_sphere_octant() {
    let solid = trimmed::sphere_octant(&Frame::WORLD, 2.0).unwrap();
    let mass = mass_properties(&solid.body).unwrap();
    assert!(
        (mass.area - solid.area).abs() <= 1e-9 * solid.area,
        "area {} vs {}",
        mass.area,
        solid.area
    );
    assert!(
        (mass.volume - solid.volume).abs() <= 1e-9 * solid.volume,
        "volume {} vs {}",
        mass.volume,
        solid.volume
    );
}
