#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use super::*;

/// A cylinder of radius 3 and height 40 as FreeCAD writes it (seam,
/// circles at two locations of one edge, a composite location).
const CYLINDER: &str = "
CASCADE Topology V1, (c) Matra-Datavision
Locations 3
1
1 0 0 0
0 1 0 0
0 0 1 0
1
1 0 0 0
0 1 0 0
0 0 1 40
2  2 -1 0
Curve2ds 1
1 0 0 0 -1
Curves 2
1 3 0 0 0 0 1
2 0 0 0 0 0 1 1 0 0 0 1 0 3
Polygon3D 0
PolygonOnTriangulations 0
Surfaces 2
2 0 0 0 0 0 1 1 0 0 0 1 0 3
1 0 0 0 0 0 1 1 0 0 0 1 0
Triangulations 0

TShapes 4
Ve
1e-07
3 0 0
0 0

0101101
*
Ed
 1e-07 1 1 0
1  1 0 0 40
3  1 2CN 1 0 0 40
0

0101000
+4 0 -4 2 *
Ed
 1e-07 1 1 0
1  2 0 0 6.283185307179586
2  1 2 0 0 6.283185307179586
0

0101000
+4 0 -4 0 *
Wi

0101100
+3 0 -3 0 -2 0 +2 2 *

+1 1";

#[test]
fn a_file_freecad_writes_reads() {
    let b = read(CYLINDER.as_bytes()).unwrap();
    assert_eq!(b.version, 1);
    assert_eq!(b.locations.len(), 3);
    assert_eq!(b.curves.len(), 2);
    assert_eq!(b.surfaces.len(), 2);
    assert_eq!(b.shapes.len(), 4);
    // Shape 1 is the last written: the wire, using shapes 3 and 2.
    let wire = b.shape(1).unwrap();
    assert_eq!(wire.kind, Kind::Wire);
    assert_eq!(wire.children.len(), 4);
    assert_eq!(wire.children[3].location, 2);
    let Kind::Edge {
        curve, degenerated, ..
    } = &b.shape(3).unwrap().kind
    else {
        panic!()
    };
    assert!(!degenerated);
    assert_eq!(curve.unwrap().last, 40.0);
    assert_eq!(b.root.unwrap().location, 1);
    // Location 3 is location 2 inverted.
    let m = b.matrix(3).unwrap();
    assert!((m[2][3] + 40.0).abs() < 1e-12);
    assert!(b.matrix(4).is_none());
    assert!(matches!(b.surface(1), Some(Surface::Cylinder { radius, .. }) if *radius == 3.0));
}

#[test]
fn orientations_compose() {
    use Orientation::*;
    assert_eq!(Forward.compose(Reversed), Reversed);
    assert_eq!(Reversed.compose(Reversed), Forward);
    assert_eq!(Internal.compose(Reversed), Internal);
    assert_eq!(Forward.compose(External), External);
}

#[test]
fn a_composite_multiplies_in_the_order_its_parts_apply() {
    let t = |x: f64| {
        [
            [1.0, 0.0, 0.0, x],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
        ]
    };
    // A quarter turn about z.
    let r = [
        [0.0, -1.0, 0.0, 0.0],
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
    ];
    let b = Brep {
        version: 1,
        locations: vec![
            Location::Elementary(t(5.0)),
            Location::Elementary(r),
            Location::Composite(vec![(1, 1), (2, 1)]),
            Location::Composite(vec![(1, -3)]),
        ],
        curves: Vec::new(),
        surfaces: Vec::new(),
        shapes: Vec::new(),
        root: None,
    };
    // Moved by 5 along x, then turned: the origin lands at (0, 5, 0).
    let m = b.matrix(3).unwrap();
    assert!((m[0][3]).abs() < 1e-12 && (m[1][3] - 5.0).abs() < 1e-12);
    assert!((b.matrix(4).unwrap()[0][3] + 15.0).abs() < 1e-12);
}

#[test]
fn every_cut_and_change_is_an_error_or_a_shape() {
    let bytes = CYLINDER.as_bytes();
    for at in 0..bytes.len() {
        let _ = read(&bytes[..at]);
        for delta in [1u8, 7, 40] {
            let mut b = bytes.to_vec();
            b[at] = b[at].wrapping_add(delta);
            let _ = read(&b);
        }
    }
}

#[test]
fn a_count_from_the_file_reserves_little() {
    // Review of 2026-11-02: "Curves 8000000" in a 16 MB file reserved
    // 960 MB (room for every record the count names before one was
    // read), and some 40 MB overflowed the capacity on wasm32.
    assert!(reserve::<Curve>(8_000_000).capacity() <= RESERVE);
    assert!(reserve::<TShape>(usize::MAX).capacity() <= RESERVE);
    for (count, section) in [
        (8_000_000usize, "Curves"),
        (8_000_000, "Surfaces"),
        (10_000_000, "TShapes"),
    ] {
        let mut text =
            format!("CASCADE Topology V1, (c) Matra-Datavision\nLocations 0\n{section} {count}\n");
        // 40 MB of a record that is never complete.
        text.push_str(&"0".repeat(40 << 20));
        let started = std::time::Instant::now();
        assert!(read(text.as_bytes()).is_err());
        assert!(started.elapsed().as_secs() < 20);
    }
    let poles = format!(
        "CASCADE Topology V1, (c) Matra-Datavision\nLocations 0\nCurves 1\n7 0 0 3 6000000 2\n{}",
        "1 ".repeat(18 << 20)
    );
    assert!(read(poles.as_bytes()).is_err());
}

#[test]
fn multiplicities_that_do_not_add_up_are_refused_before_anything_expands() {
    let head = "CASCADE Topology V1, (c) Matra-Datavision\nLocations 0\nCurves 1\n";
    // Four poles of degree 3 need multiplicities adding up to 8.
    let poles = "0 0 0 1 0 0 2 0 0 3 0 0 ";
    for (curve, ok) in [
        (format!("7 0 0 3 4 2 {poles} 0 4 1 4"), true),
        (format!("7 0 0 3 4 2 {poles} 0 26 1 26"), false),
        (format!("7 0 0 3 4 2 {poles} 0 4 1 3"), false),
        (format!("7 0 0 3 4 3 {poles} 0 4 1 1 2 4"), false),
        // Periodic: all but the last count the poles; first and last equal.
        (format!("7 0 1 3 4 5 {poles} 0 1 1 1 2 1 3 1 4 1"), true),
        (format!("7 0 1 3 4 4 {poles} 0 2 1 1 2 1 3 2"), true),
        (format!("7 0 1 3 4 4 {poles} 0 2 1 1 2 2 3 2"), false),
        (format!("7 0 1 3 4 4 {poles} 0 1 1 1 2 1 3 2"), false),
    ] {
        let text = format!("{head}{curve}\nTShapes 0\n");
        assert_eq!(read(text.as_bytes()).is_ok(), ok, "{curve}");
    }
}
