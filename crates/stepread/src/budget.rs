//! What reading a file will allocate, counted before anything is read.
//!
//! Each face of a body holds its own copy of its surface and each edge of
//! its curve, so a B-spline surface of a million control points that a
//! hostile file lets a thousand faces share would be a thousand copies.
//! The control points every face and edge of every item would hold are
//! counted from the references alone (list lengths, nothing allocated),
//! once per file, and a file above [`MAX_CONTROL_POINT_USES`] is refused.
//! A solid an assembly places N times counts N times: each placement is
//! a body of its own.

use crate::{ImportError, Parsed, err};
use craft3d_step::{Entity, Param};
use std::collections::{BTreeMap, BTreeSet};

/// Most control points (B-spline nets, polyline points) the faces and
/// edges of all a file's solids may hold together: some 64 bytes each
/// (point, weight, homogeneous copy), so about 256 MB.
pub const MAX_CONTROL_POINT_USES: usize = 4_000_000;

/// How deep a curve may be defined through others here (as in reading).
const DEPTH: usize = 4;

fn list_len(param: Option<&Param>) -> usize {
    param.and_then(Param::as_list).map_or(0, <[Param]>::len)
}

/// The control points one curve or surface holds.
fn points(parsed: &Parsed, id: u64, depth: usize, memo: &mut BTreeMap<u64, usize>) -> usize {
    if let Some(&n) = memo.get(&id) {
        return n;
    }
    let Ok(instance) = parsed.instance(id) else {
        return 1;
    };
    let entity = &instance.entity;
    let first_record = || entity.records().first();
    let any = |keywords: &[&str]| keywords.iter().any(|k| entity.is(k));
    let n = if any(&[
        "B_SPLINE_SURFACE_WITH_KNOTS",
        "UNIFORM_SURFACE",
        "QUASI_UNIFORM_SURFACE",
        "BEZIER_SURFACE",
    ]) {
        let net = match entity {
            Entity::Simple(r) => r.params.get(3),
            Entity::Complex(_) => entity
                .record("B_SPLINE_SURFACE")
                .and_then(|r| r.params.get(2)),
        };
        let rows = net.and_then(Param::as_list).unwrap_or(&[]);
        rows.len().saturating_mul(list_len(rows.first())).max(1)
    } else if any(&[
        "B_SPLINE_CURVE_WITH_KNOTS",
        "UNIFORM_CURVE",
        "QUASI_UNIFORM_CURVE",
        "BEZIER_CURVE",
    ]) {
        let list = match entity {
            Entity::Simple(r) => r.params.get(2),
            Entity::Complex(_) => entity
                .record("B_SPLINE_CURVE")
                .and_then(|r| r.params.get(1)),
        };
        list_len(list).max(1)
    } else if entity.is("POLYLINE") {
        list_len(first_record().and_then(|r| r.params.get(1))).max(1)
    } else if depth < DEPTH
        && any(&[
            "SURFACE_CURVE",
            "SEAM_CURVE",
            "INTERSECTION_CURVE",
            "SURFACE_OF_LINEAR_EXTRUSION",
            "SURFACE_OF_REVOLUTION",
            "OFFSET_SURFACE",
        ])
    {
        // A curve swept round an axis is nine rows of its points (four
        // rational quarters), along a direction two.
        let rows = if entity.is("SURFACE_OF_REVOLUTION") {
            9
        } else if entity.is("SURFACE_OF_LINEAR_EXTRUSION") {
            2
        } else {
            1
        };
        first_record()
            .and_then(|r| r.params.get(1))
            .and_then(Param::as_ref)
            .map_or(1, |inner| points(parsed, inner, depth + 1, memo))
            .saturating_mul(rows)
    } else if entity.is("CIRCLE") || entity.is("ELLIPSE") {
        // As a spline when swept: nine points.
        9
    } else {
        1
    };
    memo.insert(id, n);
    n
}

/// The control points the faces and edges of every item hold, refused
/// above [`MAX_CONTROL_POINT_USES`].
pub(crate) fn check(parsed: &Parsed) -> Result<usize, ImportError> {
    let mut memo = BTreeMap::new();
    let mut total: usize = 0;
    // An item placed N times by assemblies is built N times.
    let mut copies = vec![0usize; parsed.items().len()];
    for placed in parsed.placed() {
        if let Some(n) = copies.get_mut(placed.item) {
            *n = n.saturating_add(1);
        }
    }
    for (index, item) in parsed.items().iter().enumerate() {
        let times = copies.get(index).copied().unwrap_or(1).max(1);
        let mut seen: BTreeSet<u64> = BTreeSet::new();
        let mut stack = vec![item.id];
        while let Some(id) = stack.pop() {
            if !seen.insert(id) {
                continue;
            }
            let Ok(instance) = parsed.instance(id) else {
                continue;
            };
            let entity = &instance.entity;
            let face = ["ADVANCED_FACE", "FACE_SURFACE"]
                .iter()
                .find_map(|k| entity.record(k));
            if let Some(face) = face {
                if let Some(surface) = face.params.get(2).and_then(Param::as_ref) {
                    total = total.saturating_add(
                        points(parsed, surface, 0, &mut memo).saturating_mul(times),
                    );
                }
                if let Some(bounds) = face.params.get(1) {
                    bounds.for_each_ref(&mut |r| stack.push(r));
                }
            } else if let Some(edge) = entity.record("EDGE_CURVE") {
                if let Some(curve) = edge.params.get(3).and_then(Param::as_ref) {
                    total = total
                        .saturating_add(points(parsed, curve, 0, &mut memo).saturating_mul(times));
                }
            } else if entity.is("VERTEX_POINT") || entity.is("CARTESIAN_POINT") {
                // Nothing below a vertex holds control points.
            } else {
                entity.for_each_ref(&mut |r| stack.push(r));
            }
            if total > MAX_CONTROL_POINT_USES {
                return err(format!(
                    "the faces and edges of the file's solids would hold more than {MAX_CONTROL_POINT_USES} control points (a curve or surface shared by many of them is a copy in each); it is not read"
                ));
            }
        }
    }
    Ok(total)
}
