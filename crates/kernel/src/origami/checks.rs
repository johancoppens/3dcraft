//! Kawasaki's and Maekawa's conditions at the interior vertices of a
//! crease pattern.
//!
//! Both are *local* and *necessary* conditions for a vertex to fold flat
//! (Demaine and O'Rourke 2007, §12.2; Hull, *Origametry*, ch. 1): at a
//! vertex that folds flat, the alternating sum of the sector angles is 0,
//! so the sectors taken alternately add up to 180 degrees each (Kawasaki,
//! also Justin), and the numbers of mountains and valleys differ by two
//! (Maekawa). Neither is sufficient, not even for one vertex (the
//! mountain–valley assignment must also satisfy the big-little-big
//! condition), and a pattern whose every vertex passes both may still not
//! fold flat: whether a crease pattern folds flat is NP-hard (Bern and
//! Hayes 1996). These checks say which vertex fails and by how much; they
//! never say that a pattern folds flat.

use super::{Arrangement, CreaseKind, P2, sub};
use craft3d_math::Portable;
use std::f64::consts::{PI, TAU};

/// The two conditions at one interior vertex.
#[derive(Clone, Debug, PartialEq)]
pub struct VertexCheck {
    pub vertex: usize,
    pub at: P2,
    /// The creases that meet here, counter-clockwise from the first one
    /// after the direction −x, by curve id.
    pub creases: Vec<u64>,
    /// The angles between each crease and the next, counter-clockwise, in
    /// radians.
    pub sectors: Vec<f64>,
    pub mountains: usize,
    pub valleys: usize,
    /// The sectors taken alternately: the sums of the first, third, … and
    /// of the second, fourth, … (radians); for an even degree they add up
    /// to 2π.
    pub sums: [f64; 2],
    /// How far the first sum is from π (radians, signed); `None` for an
    /// odd number of creases, which never folds flat.
    pub kawasaki: Option<f64>,
    pub kawasaki_holds: bool,
    /// `|M − V|`; Maekawa holds when it is 2.
    pub maekawa: usize,
    pub maekawa_holds: bool,
}

/// The two conditions at every interior vertex of the pattern (not on
/// the boundary), in the order of the vertices; vertices where a cut ends
/// or passes are left out and returned apart: the conditions are about a
/// sheet joined all round the vertex. `angular` is the document's angular
/// tolerance: Kawasaki holds when the sums are within it of π, plus the
/// angle the linear tolerance makes at the length of the shortest crease
/// there (a point may lie anywhere within the linear tolerance).
pub fn vertex_checks(arr: &Arrangement, angular: f64) -> (Vec<VertexCheck>, Vec<(usize, P2)>) {
    let mut out = Vec::new();
    let mut on_cuts = Vec::new();
    for (v, around) in arr.around.iter().enumerate() {
        let Some(at) = arr.point(v) else { continue };
        let kinds: Vec<Option<CreaseKind>> =
            around.iter().map(|&h| arr.crease_kind(h / 2)).collect();
        if kinds.iter().any(Option::is_none) {
            continue;
        }
        if kinds.contains(&Some(CreaseKind::Cut)) {
            on_cuts.push((v, at));
            continue;
        }
        let angles: Vec<f64> = around
            .iter()
            .filter_map(|&h| {
                let head = arr.point(arr.head(h)?)?;
                let d = sub(head, at);
                Some(d[1].patan2(d[0]))
            })
            .collect();
        let n = angles.len();
        if n == 0 {
            continue;
        }
        let sectors: Vec<f64> = (0..n)
            .filter_map(|k| {
                let a = *angles.get(k)?;
                let b = *angles.get((k + 1) % n)?;
                let mut s = b - a;
                if s <= 0.0 {
                    s += TAU;
                }
                Some(s)
            })
            .collect();
        let mut sums = [0.0, 0.0];
        for (k, s) in sectors.iter().enumerate() {
            if let Some(slot) = sums.get_mut(k % 2) {
                *slot += s;
            }
        }
        let mountains = kinds
            .iter()
            .filter(|k| **k == Some(CreaseKind::Mountain))
            .count();
        let valleys = kinds
            .iter()
            .filter(|k| **k == Some(CreaseKind::Valley))
            .count();
        let shortest = around
            .iter()
            .filter_map(|&h| {
                let head = arr.point(arr.head(h)?)?;
                Some(super::dist(head, at))
            })
            .fold(f64::INFINITY, f64::min);
        let slack = angular + arr.tolerance / shortest;
        let kawasaki = n.is_multiple_of(2).then(|| sums[0] - PI);
        let maekawa = mountains.abs_diff(valleys);
        out.push(VertexCheck {
            vertex: v,
            at,
            creases: around
                .iter()
                .filter_map(|&h| arr.edges.get(h / 2).map(|e| e.source))
                .collect(),
            sectors,
            mountains,
            valleys,
            sums,
            kawasaki,
            kawasaki_holds: kawasaki.is_some_and(|d| d.abs() <= slack),
            maekawa,
            maekawa_holds: maekawa == 2,
        });
    }
    (out, on_cuts)
}
