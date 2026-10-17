//! A static tree of axis-aligned boxes (a bounding-volume hierarchy), for
//! finding which of many boxes pass a test without testing all of them:
//! the broad phase of profile detection (boxes in the plane) and of the
//! booleans (the boxes of faces in space, `boolean/facetree.rs`).
//!
//! The tree is built once by splitting the boxes at the median of their
//! centres along the axis of the widest spread, down to leaves of a few
//! boxes; a node's box is the hull of the boxes under it (minima and
//! maxima, so no rounding). A search hands every node's box to a test and
//! goes below only where it passes, so any test that passes for a box
//! whenever it passes for a box inside it finds exactly the boxes it would
//! find by testing every one. A search draws on a budget, one per box
//! found, and stops when it runs out: the bound on the work of profile
//! detection.

use craft3d_math::Bbox3;

/// How many boxes a leaf holds at most.
const LEAF: usize = 8;

/// `[min x, min y, max x, max y]`.
pub(crate) type Box2 = [f64; 4];

/// A box the tree can hold: its hull with another, and its centre along
/// an axis (for the split; any number that orders boxes along the axis).
pub(crate) trait TreeBox: Clone {
    const AXES: usize;
    /// The box that holds nothing, the start of a hull.
    fn nothing() -> Self;
    /// Grows `self` to hold `other` too (minima and maxima, exact).
    fn take_in(&mut self, other: &Self);
    fn centre(&self, axis: usize) -> f64;
}

impl TreeBox for Box2 {
    const AXES: usize = 2;
    fn nothing() -> Self {
        [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ]
    }
    fn take_in(&mut self, b: &Self) {
        *self = [
            self[0].min(b[0]),
            self[1].min(b[1]),
            self[2].max(b[2]),
            self[3].max(b[3]),
        ];
    }
    fn centre(&self, axis: usize) -> f64 {
        if axis == 1 {
            self[1] / 2.0 + self[3] / 2.0
        } else {
            self[0] / 2.0 + self[2] / 2.0
        }
    }
}

impl TreeBox for Bbox3 {
    const AXES: usize = 3;
    fn nothing() -> Self {
        Bbox3::EMPTY
    }
    fn take_in(&mut self, b: &Self) {
        if let Some((lo, hi)) = b.corners() {
            self.add(lo);
            self.add(hi);
        }
    }
    fn centre(&self, axis: usize) -> f64 {
        self.corners().map_or(0.0, |(lo, hi)| match axis {
            0 => lo.x / 2.0 + hi.x / 2.0,
            1 => lo.y / 2.0 + hi.y / 2.0,
            _ => lo.z / 2.0 + hi.z / 2.0,
        })
    }
}

struct Node<B> {
    bbox: B,
    /// The range of `items` under this node.
    first: usize,
    end: usize,
    /// The two nodes below, or none for a leaf.
    children: Option<[usize; 2]>,
}

pub(crate) struct BoxTree<B: TreeBox = Box2> {
    boxes: Vec<B>,
    /// Box indices, grouped so that every node's boxes are one range.
    items: Vec<usize>,
    nodes: Vec<Node<B>>,
}

fn hull<B: TreeBox>(boxes: &[B], items: &[usize]) -> B {
    let mut h = B::nothing();
    for b in items.iter().filter_map(|&i| boxes.get(i)) {
        h.take_in(b);
    }
    h
}

impl<B: TreeBox> BoxTree<B> {
    pub(crate) fn new(boxes: Vec<B>) -> BoxTree<B> {
        let mut items: Vec<usize> = (0..boxes.len()).collect();
        let mut nodes = Vec::with_capacity(boxes.len() / LEAF * 2 + 1);
        let placeholder = || Node {
            bbox: B::nothing(),
            first: 0,
            end: 0,
            children: None,
        };
        nodes.push(placeholder());
        // (node, first, end): every node is built once, with a range half
        // its parent's, so the stack holds at most the depth plus one.
        let mut todo = vec![(0_usize, 0_usize, items.len())];
        while let Some((at, first, end)) = todo.pop() {
            let Some(slice) = items.get_mut(first..end) else {
                continue;
            };
            let bbox = hull(&boxes, slice);
            let mut children = None;
            if slice.len() > LEAF {
                // Split along the axis of the widest spread of the centres
                // (the first of equal ones).
                let centre = |i: usize, axis: usize| boxes.get(i).map_or(0.0, |b| b.centre(axis));
                let mut axis = 0;
                let mut widest = f64::NEG_INFINITY;
                for along in 0..B::AXES {
                    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
                    for &i in slice.iter() {
                        let c = centre(i, along);
                        lo = lo.min(c);
                        hi = hi.max(c);
                    }
                    if along == 0 || hi - lo > widest {
                        axis = along;
                        widest = hi - lo;
                    }
                }
                let mid = slice.len() / 2;
                slice.select_nth_unstable_by(mid, |&i, &j| {
                    centre(i, axis).total_cmp(&centre(j, axis)).then(i.cmp(&j))
                });
                let (left, right) = (nodes.len(), nodes.len() + 1);
                nodes.push(placeholder());
                nodes.push(placeholder());
                todo.push((left, first, first + mid));
                todo.push((right, first + mid, end));
                children = Some([left, right]);
            }
            if let Some(node) = nodes.get_mut(at) {
                *node = Node {
                    bbox,
                    first,
                    end,
                    children,
                };
            }
        }
        BoxTree {
            boxes,
            items,
            nodes,
        }
    }

    /// Puts into `out` (cleared first) every item whose box passes `test`,
    /// in no particular order (the callers order what they keep). `test`
    /// must pass for a box whenever it passes for a box inside it.
    ///
    /// Every item found is taken from `budget`; when there is none left
    /// the search stops at once and returns false (`out` then holds the
    /// items found until then).
    pub(crate) fn search(
        &self,
        test: impl Fn(&B) -> bool,
        out: &mut Vec<usize>,
        budget: &mut usize,
    ) -> bool {
        out.clear();
        let mut stack = vec![0_usize];
        while let Some(at) = stack.pop() {
            let Some(node) = self.nodes.get(at) else {
                continue;
            };
            if !test(&node.bbox) {
                continue;
            }
            match node.children {
                Some(below) => stack.extend(below),
                None => {
                    let items = self.items.get(node.first..node.end).unwrap_or(&[]);
                    for &i in items {
                        if self.boxes.get(i).is_some_and(&test) {
                            let Some(left) = budget.checked_sub(1) else {
                                return false;
                            };
                            *budget = left;
                            out.push(i);
                        }
                    }
                }
            }
        }
        true
    }

    /// True when an item whose box passes `test` also passes `pick`; the
    /// search stops at the first. `test` must pass for a box whenever it
    /// passes for a box inside it, so this is `any` over the items `search`
    /// finds.
    ///
    /// Every item whose box passes is taken from `budget`; when there is
    /// none left the search stops and returns `None`, with the last item
    /// taken in `last`.
    pub(crate) fn any(
        &self,
        test: impl Fn(&B) -> bool,
        mut pick: impl FnMut(usize) -> bool,
        budget: &mut usize,
        last: &mut Option<usize>,
    ) -> Option<bool> {
        let mut stack = vec![0_usize];
        while let Some(at) = stack.pop() {
            let Some(node) = self.nodes.get(at) else {
                continue;
            };
            if !test(&node.bbox) {
                continue;
            }
            match node.children {
                Some(below) => stack.extend(below),
                None => {
                    let items = self.items.get(node.first..node.end).unwrap_or(&[]);
                    for &i in items {
                        if self.boxes.get(i).is_some_and(&test) {
                            let left = budget.checked_sub(1)?;
                            *budget = left;
                            *last = Some(i);
                            if pick(i) {
                                return Some(true);
                            }
                        }
                    }
                }
            }
        }
        Some(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SplitMix64.
    fn next(state: &mut u64) -> u64 {
        *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = *state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn overlap(a: &Box2, b: &Box2) -> bool {
        !(b[0] > a[2] || a[0] > b[2] || b[1] > a[3] || a[1] > b[3])
    }

    #[test]
    fn finds_what_testing_every_box_finds() {
        let mut state = 7;
        for n in [0, 1, 2, 8, 9, 17, 100, 1000] {
            let mut boxes = Vec::new();
            for k in 0..n {
                let mut u = || (next(&mut state) >> 11) as f64 / (1u64 << 53) as f64;
                // Some boxes repeat exactly, some are points, some are long.
                let (x, y) = (u() * 100.0, u() * 100.0);
                let (w, h) = match k % 4 {
                    0 => (0.0, 0.0),
                    1 => (u() * 100.0, u()),
                    _ => (u() * 5.0, u() * 5.0),
                };
                boxes.push([x, y, x + w, y + h]);
                if k % 7 == 0 {
                    boxes.push([x, y, x + w, y + h]);
                }
            }
            let tree = BoxTree::new(boxes.clone());
            let mut out = Vec::new();
            for q in &boxes {
                let mut budget = usize::MAX;
                assert!(tree.search(|b| overlap(q, b), &mut out, &mut budget));
                out.sort_unstable();
                assert_eq!(usize::MAX - budget, out.len());
                let all: Vec<usize> = (0..boxes.len())
                    .filter(|&i| overlap(q, &boxes[i]))
                    .collect();
                assert_eq!(out, all);
                // `any` agrees with `any` over every box, for a pick that
                // passes for some of them.
                let pick = |i: usize| i % 5 == 3;
                let mut budget = usize::MAX;
                let mut last = None;
                let found = tree.any(|b| overlap(q, b), pick, &mut budget, &mut last);
                assert_eq!(found, Some(all.iter().any(|&i| pick(i))));
            }
        }
    }

    #[test]
    fn a_search_stops_when_its_budget_runs_out() {
        let boxes = vec![[0.0, 0.0, 1.0, 1.0]; 100];
        let tree = BoxTree::new(boxes);
        let mut out = Vec::new();
        let mut budget = 30;
        assert!(!tree.search(|_| true, &mut out, &mut budget));
        assert_eq!((out.len(), budget), (30, 0));
        let mut budget = 100;
        assert!(tree.search(|_| true, &mut out, &mut budget));
        assert_eq!((out.len(), budget), (100, 0));
        let (mut budget, mut last) = (30, None);
        assert_eq!(tree.any(|_| true, |_| false, &mut budget, &mut last), None);
        assert_eq!(budget, 0);
        let (mut budget, mut last) = (30, None);
        assert_eq!(
            tree.any(|_| true, |_| true, &mut budget, &mut last),
            Some(true)
        );
        assert_eq!(budget, 29);
    }
}
