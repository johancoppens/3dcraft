//! Numbers in a `.3dcraft` file: what is read is what was written.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_doc::{Document, FeatureKind, Sketch, SketchPlane, Units};
use craft3d_format::{from_bytes, to_bytes};
use craft3d_math::Portable;

/// A reopened document must regenerate to the bodies it was saved from, so
/// a coordinate comes back as the very number that was written, not the
/// next one. The default JSON parser is allowed to be one unit in the last
/// place off: 14.582575694955839 (the centre of an arc of radius 5 over a
/// chord of 4) came back as 14.58257569495584.
#[test]
fn coordinates_come_back_bit_for_bit() {
    let mut sketch = Sketch::new(SketchPlane::default());
    sketch.add_point(0.1 + 0.2, 14.582575694955839).unwrap();
    // A thousand more, of every magnitude, from a fixed sequence.
    let mut state = 0x9E37_79B9_7F4A_7C15_u64;
    let mut next = || {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let unit = (state >> 11) as f64 / (1u64 << 53) as f64;
        (unit - 0.5) * 10f64.ppowi((state % 13) as i32 - 6)
    };
    for _ in 0..1000 {
        sketch.add_point(next(), next()).unwrap();
    }
    let mut doc = Document::new(Units::default());
    doc.add_feature(None, FeatureKind::Sketch(sketch)).unwrap();
    let back = from_bytes(&to_bytes(&doc).unwrap()).unwrap();
    let points = |doc: &Document| -> Vec<(u64, u64)> {
        let FeatureKind::Sketch(sketch) = &doc.features[0].kind else {
            panic!("not a sketch");
        };
        sketch
            .entities
            .iter()
            .map(|e| sketch.point(e.id).unwrap())
            .map(|(x, y)| (x.to_bits(), y.to_bits()))
            .collect()
    };
    assert_eq!(points(&back), points(&doc));
    assert_eq!(back, doc);
}
