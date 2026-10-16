//! A static tree of axis-aligned boxes in the plane (a bounding-volume
//! hierarchy), for finding which of many boxes pass a test without testing
//! all of them: the broad phase of profile detection.
//!
//! The tree is built once by splitting the boxes at the median of their
//! centres along the longer side of the spread, down to leaves of a few
//! boxes; a node's box is the hull of the boxes under it (minima and
//! maxima, so no rounding). A search hands every node's box to a test and
//! goes below only where it passes, so any test that passes for a box
//! whenever it passes for a box inside it finds exactly the boxes it would
//! find by testing every one. What it finds is returned in ascending index
//! order, whatever the shape of the tree.

/// How many boxes a leaf holds at most.
const LEAF: usize = 8;

/// `[min x, min y, max x, max y]`.
pub(crate) type Box2 = [f64; 4];

struct Node {
    bbox: Box2,
    /// The range of `items` under this node.
    first: usize,
    end: usize,
    /// The two nodes below, or none for a leaf.
    children: Option<[usize; 2]>,
}

pub(crate) struct BoxTree {
    boxes: Vec<Box2>,
    /// Box indices, grouped so that every node's boxes are one range.
    items: Vec<usize>,
    nodes: Vec<Node>,
}

fn hull(boxes: &[Box2], items: &[usize]) -> Box2 {
    let mut h = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for b in items.iter().filter_map(|&i| boxes.get(i)) {
        h = [
            h[0].min(b[0]),
            h[1].min(b[1]),
            h[2].max(b[2]),
            h[3].max(b[3]),
        ];
    }
    h
}

impl BoxTree {
    pub(crate) fn new(boxes: Vec<Box2>) -> BoxTree {
        let mut items: Vec<usize> = (0..boxes.len()).collect();
        let mut nodes = Vec::with_capacity(boxes.len() / LEAF * 2 + 1);
        let placeholder = || Node {
            bbox: [0.0; 4],
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
                // Split along the longer side of the spread of the centres.
                let centre = |i: usize, along_y: bool| {
                    boxes.get(i).map_or(0.0, |b| {
                        if along_y {
                            b[1] / 2.0 + b[3] / 2.0
                        } else {
                            b[0] / 2.0 + b[2] / 2.0
                        }
                    })
                };
                let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
                for &i in slice.iter() {
                    let c = [centre(i, false), centre(i, true)];
                    lo = [lo[0].min(c[0]), lo[1].min(c[1])];
                    hi = [hi[0].max(c[0]), hi[1].max(c[1])];
                }
                let axis = hi[1] - lo[1] > hi[0] - lo[0];
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
    /// ascending. `test` must pass for a box whenever it passes for a box
    /// inside it.
    pub(crate) fn search(&self, test: impl Fn(&Box2) -> bool, out: &mut Vec<usize>) {
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
                    out.extend(
                        items
                            .iter()
                            .copied()
                            .filter(|&i| self.boxes.get(i).is_some_and(&test)),
                    );
                }
            }
        }
        out.sort_unstable();
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
                tree.search(|b| overlap(q, b), &mut out);
                let all: Vec<usize> = (0..boxes.len())
                    .filter(|&i| overlap(q, &boxes[i]))
                    .collect();
                assert_eq!(out, all);
            }
        }
    }
}
