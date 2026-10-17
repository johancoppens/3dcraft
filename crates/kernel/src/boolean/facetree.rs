//! A static tree of the boxes of an operand's faces, or of a face's edges
//! (a bounding-volume hierarchy), so that the faces a box, a point or a ray
//! can meet are found without testing every face: the broad phase of the
//! section (which faces of B a face of A can meet, and which edges of a
//! face are near the other face) and of the ray casting that classifies
//! regions (which faces a ray or a point near the boundary can meet).
//!
//! The tree is the crate's [`BoxTree`] of the faces' boxes. A search
//! returns the faces whose boxes pass a test **in ascending order**, and
//! the caller then runs its own test on each exactly as it would have on
//! every face. So any test that
//! passes for a box whenever it passes for a box inside it (every test of
//! a box against a box, a point or a line grown by a margin is such a test:
//! the arithmetic of floating point is monotone) finds exactly the faces
//! testing every face would find, in the same order: the results are the
//! same bit for bit.
//!
//! A face whose box is not finite (empty, or with an infinite or NaN
//! corner) is in no node: it is always a candidate, so nothing depends on
//! how a test treats such numbers.

use crate::boxtree::BoxTree;
use craft3d_math::Bbox3;

/// Fewer faces than this are not worth a tree: every face is a candidate.
const SMALL: usize = 16;

pub(super) struct FaceTree {
    /// The tree of the finite boxes, and the face of each box in it.
    tree: Option<(BoxTree<Bbox3>, Vec<usize>)>,
    /// The faces in no tree (boxes that are not finite), ascending.
    always: Vec<usize>,
    /// Without a tree: every face is a candidate.
    all: usize,
}

fn finite(b: &Bbox3) -> bool {
    b.corners().is_some_and(|(lo, hi)| {
        [lo.x, lo.y, lo.z, hi.x, hi.y, hi.z]
            .iter()
            .all(|c| c.is_finite())
    })
}

impl FaceTree {
    pub(super) fn new(boxes: &[Bbox3]) -> FaceTree {
        if boxes.len() < SMALL {
            return FaceTree {
                tree: None,
                always: Vec::new(),
                all: boxes.len(),
            };
        }
        let (faces, always): (Vec<usize>, Vec<usize>) =
            (0..boxes.len()).partition(|&i| boxes.get(i).is_some_and(finite));
        let held = faces
            .iter()
            .filter_map(|&i| boxes.get(i).copied())
            .collect();
        FaceTree {
            tree: Some((BoxTree::new(held), faces)),
            always,
            all: 0,
        }
    }

    /// Every face whose box passes `test`, and those whose boxes are not
    /// finite (for the caller to test as it tests every face), ascending.
    /// `test` must pass for a box whenever it passes for a box inside it.
    pub(super) fn candidates(&self, test: impl Fn(&Bbox3) -> bool) -> Vec<usize> {
        let Some((tree, faces)) = &self.tree else {
            return (0..self.all).collect();
        };
        let mut found = Vec::new();
        let mut budget = usize::MAX;
        // (A budget of every face there is cannot run out.)
        tree.search(test, &mut found, &mut budget);
        let mut out = self.always.clone();
        out.extend(found.iter().filter_map(|&i| faces.get(i).copied()));
        out.sort_unstable();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::super::outline::{boxes_meet, line_in_box};
    use super::*;
    use craft3d_math::Vec3;

    /// SplitMix64.
    fn next(state: &mut u64) -> u64 {
        *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = *state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn boxes(state: &mut u64, n: usize) -> Vec<Bbox3> {
        let mut out = Vec::new();
        for k in 0..n {
            let mut u = || (next(state) >> 11) as f64 / (1u64 << 53) as f64;
            let p = Vec3::new(u() * 100.0, u() * 100.0, u() * 100.0);
            // Some boxes repeat, some are points, some long, some empty or
            // not finite.
            let size = match k % 6 {
                0 => Vec3::ZERO,
                1 => Vec3::new(u() * 100.0, u(), u()),
                _ => Vec3::new(u() * 5.0, u() * 5.0, u() * 5.0),
            };
            let b = match k % 29 {
                3 => Bbox3::EMPTY,
                5 => Bbox3::from_points([p, Vec3::new(f64::NAN, p.y, p.z)]),
                7 => Bbox3::from_points([p, Vec3::new(p.x, f64::INFINITY, p.z)]),
                _ => Bbox3::from_points([p, p + size]),
            };
            out.push(b);
            if k % 7 == 0 {
                out.push(b);
            }
        }
        out
    }

    /// What testing every face finds, the tree finds among its candidates.
    fn same(tree: &FaceTree, all: &[Bbox3], test: impl Fn(&Bbox3) -> bool) {
        let found: Vec<usize> = tree
            .candidates(&test)
            .into_iter()
            .filter(|&i| test(&all[i]))
            .collect();
        let want: Vec<usize> = (0..all.len()).filter(|&i| test(&all[i])).collect();
        assert_eq!(found, want);
    }

    #[test]
    fn finds_what_testing_every_face_finds_in_the_same_order() {
        let mut state = 11;
        for n in [0, 1, 2, 15, 16, 17, 100, 1000] {
            let all = boxes(&mut state, n);
            let tree = FaceTree::new(&all);
            for q in all.iter().take(60) {
                same(&tree, &all, |b| boxes_meet(q, b, 0.25));
                if let Some((lo, hi)) = q.corners() {
                    same(&tree, &all, |b| b.contains(lo, 0.5));
                    let direction = Vec3::new(0.577, 0.211, 0.789);
                    same(&tree, &all, |b| {
                        line_in_box(hi, direction, b, 0.5).is_some_and(|(_, leave)| leave > -1.0)
                    });
                }
            }
        }
    }
}
