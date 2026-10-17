//! Oracles 1 and 2 (docs/testing.md): hand-built solids are valid and have
//! their closed-form mass properties.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_testkit::solids;
use craft3d_topo::{BodyKind, check, genus, mass_properties};

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
fn reference_solids_are_valid() {
    for scale in [1e-3, 1.0, 1e6] {
        let tol = Tolerance::new(1e-6 * scale, 1e-9).unwrap();
        for frame in frames(scale) {
            for solid in solids::all(&frame, scale).unwrap() {
                let defects = check(&solid.body, &tol);
                assert!(
                    defects.is_empty(),
                    "{} at scale {scale}: {defects:?}",
                    solid.name
                );
                let b = &solid.body;
                assert_eq!(
                    (b.vertices.len(), b.edges.len(), b.faces.len()),
                    solid.counts,
                    "{}",
                    solid.name
                );
                assert_eq!(genus(b), Some(solid.genus), "{}", solid.name);
            }
        }
    }
}

#[test]
fn mass_properties_match_the_closed_forms() {
    for scale in [1e-3, 1.0, 1e6] {
        for frame in frames(scale) {
            for solid in solids::all(&frame, scale).unwrap() {
                let m = mass_properties(&solid.body).unwrap();
                let rel = |got: f64, want: f64| ((got - want) / want).abs();
                assert!(
                    rel(m.area, solid.area) < 1e-12,
                    "{} area at scale {scale}: {} vs {}",
                    solid.name,
                    m.area,
                    solid.area
                );
                assert!(
                    rel(m.volume, solid.volume) < 1e-12,
                    "{} volume at scale {scale}: {} vs {}",
                    solid.name,
                    m.volume,
                    solid.volume
                );
                let centroid = m.centroid.unwrap();
                assert!(
                    centroid.distance(solid.centroid) < 1e-11 * scale,
                    "{} centroid at scale {scale}: {centroid:?} vs {:?}",
                    solid.name,
                    solid.centroid
                );
            }
        }
    }
}

#[test]
fn an_inside_out_solid_has_negative_volume_and_the_same_area() {
    for solid in solids::all(&Frame::WORLD, 1.0).unwrap() {
        let mut body = solid.body.clone();
        for face in &mut body.faces {
            face.reversed = !face.reversed;
        }
        for loop_ in &mut body.loops {
            loop_.coedges.reverse();
            for coedge in &mut loop_.coedges {
                coedge.reversed = !coedge.reversed;
            }
        }
        assert!(
            check(&body, &Tolerance::default()).is_empty(),
            "{}",
            solid.name
        );
        let m = mass_properties(&body).unwrap();
        assert!(
            (m.volume + solid.volume).abs() < 1e-12 * solid.volume,
            "{}: {}",
            solid.name,
            m.volume
        );
        assert!(
            (m.area - solid.area).abs() < 1e-12 * solid.area,
            "{}",
            solid.name
        );
    }
}

#[test]
fn a_sheet_has_area_but_no_volume() {
    let mut block = solids::block(&Frame::WORLD, 2.0, 3.0, 4.0).unwrap().body;
    block.kind = BodyKind::Sheet;
    // Drop the top face: an open box.
    block.shells[0].faces.retain(|f| f.0 != 1);
    let top = block.faces.remove(1);
    block.loops[top.loops[0].0 as usize].coedges.clear();
    for shell in &mut block.shells {
        for face in &mut shell.faces {
            if face.0 > 1 {
                face.0 -= 1;
            }
        }
    }
    let m = mass_properties(&block).unwrap();
    assert!((m.area - (52.0 - 6.0)).abs() < 1e-12);
    assert_eq!(m.volume, 0.0);
    assert!(m.centroid.is_none());
}

#[test]
fn unbounded_and_broken_faces_are_errors_not_panics() {
    use craft3d_geom::{Cylinder, Plane, Surface};
    use craft3d_topo::{Body, Coedge, EdgeId, LoopId, MassError};
    // A whole plane or cylinder is not a face.
    for surface in [
        Surface::Plane(Plane::new(Frame::WORLD)),
        Surface::Cylinder(Cylinder::new(Frame::WORLD, 1.0).unwrap()),
    ] {
        let mut body = Body::new(BodyKind::Solid);
        let face = body.add_face(surface, false, Vec::new());
        body.add_shell(vec![face]);
        assert_eq!(
            mass_properties(&body),
            Err(MassError::UnboundedFace { face: 0 })
        );
    }
    // A cylinder wall with only one end loop runs off to infinity.
    let mut open = solids::cylinder(&Frame::WORLD, 1.0, 2.0).unwrap().body;
    open.faces[0].loops.pop();
    assert_eq!(
        mass_properties(&open),
        Err(MassError::UnboundedFace { face: 0 })
    );
    // Dangling references.
    let mut dangling = solids::cylinder(&Frame::WORLD, 1.0, 2.0).unwrap().body;
    dangling.faces[1].loops[0] = LoopId(99);
    assert_eq!(
        mass_properties(&dangling),
        Err(MassError::Invalid { face: 1 })
    );
    let mut dangling = solids::cylinder(&Frame::WORLD, 1.0, 2.0).unwrap().body;
    dangling.loops[0].coedges[0] = Coedge {
        edge: EdgeId(99),
        reversed: false,
    };
    assert_eq!(
        mass_properties(&dangling),
        Err(MassError::Invalid { face: 0 })
    );
    // An empty body has nothing.
    let empty = mass_properties(&Body::new(BodyKind::Solid)).unwrap();
    assert_eq!((empty.area, empty.volume, empty.centroid), (0.0, 0.0, None));
}

/// `mass_bound` gives `mass_properties`' very value, and its bound covers
/// how far that is from the closed form, while staying a small share of
/// what is measured (it is the integrator's allowance, not a guess).
#[test]
fn the_mass_bound_covers_the_error_and_is_small() {
    use craft3d_testkit::trimmed;
    for scale in [1e-3, 1.0, 1e6] {
        for frame in frames(scale) {
            let mut all = solids::all(&frame, scale).unwrap();
            all.extend(trimmed::all(&frame, scale).unwrap());
            for solid in all {
                let (m, bound) = craft3d_topo::mass_bound(&solid.body).unwrap();
                assert_eq!(m, mass_properties(&solid.body).unwrap(), "{}", solid.name);
                assert!(
                    (m.volume - solid.volume).abs() <= bound.volume,
                    "{} at {scale}: volume {} off by {}, bound {}",
                    solid.name,
                    m.volume,
                    (m.volume - solid.volume).abs(),
                    bound.volume
                );
                assert!(
                    (m.area - solid.area).abs() <= bound.area,
                    "{} at {scale}: area off by {}, bound {}",
                    solid.name,
                    (m.area - solid.area).abs(),
                    bound.area
                );
                assert!(
                    bound.volume > 0.0 && bound.volume < 1e-9 * solid.volume.abs(),
                    "{} at {scale}: bound {} of {}",
                    solid.name,
                    bound.volume,
                    solid.volume
                );
            }
        }
    }
}
