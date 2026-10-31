//! Rigid folding: bodies (the facets) joined by hinges (the fold creases),
//! each body moving rigidly, turned about its hinges by their angles.
//!
//! The solve knows nothing of crease patterns: it takes a [`Linkage`],
//! bodies and joints, each joint with its own axis in space (a point, a
//! direction, a side to sample) and the motion of its right body
//! relative to its left one at an angle ([`Joint::motion`]: a turn about
//! the axis by −ρ, after a shift along the neutral fibre for a joint with
//! a radius). A crease pattern is one linkage ([`fold`] builds it from an
//! [`Arrangement`]); a sheet-metal part is another, its joints bends with
//! a radius (`crate::sheetmetal`; docs/design/origami.md › *One fold
//! mechanism*, docs/design/sheetmetal.md).
//!
//! A breadth-first forest from the fixed body (and from the first body of
//! each other piece, which stays where it is) places every body by the
//! motions along its path, `T_child = T_parent ∘ M(±ρ)`. Every joint
//! outside the forest closes a loop and holds only when its two bodies
//! agree, `T_right = T_left ∘ M(ρ)`: the loop-closure condition of rigid
//! origami (Belcastro and Hull; Demaine and O'Rourke 2007, §13.1), for
//! every fundamental cycle, so any graph of bodies (holes, cuts, pieces)
//! is handled alike. It is written as three points of the right body that
//! must land in the same place both ways round (nine lengths per loop),
//! and the unknown angles are found by Levenberg–Marquardt on them with
//! the exact derivative: turning joint `k` moves a point `X` beyond it by
//! `s D × (X − P)` per radian, `P` and `D` its axis as its parent body
//! has it. The given angles are reached from the flat sheet by
//! continuation: the first step picks the branch through the flat sheet
//! by the kinds (and checks it is one, by halving), every later step is
//! predicted along the tangent (`J_free dx = −J_given dg`) and corrected;
//! a step whose corrector strays from the prediction is halved, and a
//! tangent that does not exist (a bifurcation) is refused by name. At the end the
//! solution must close within the tolerance, have full rank and turn
//! every solved joint its kind's way.

use super::sparse::{Chain, Pattern, chain_rows, cycle_basis};
use super::{FoldError, MAX_DENSE, MAX_FREE};
use craft3d_math::Portable;
use craft3d_math::{Transform, Vec3};
use std::collections::{BTreeMap, VecDeque};
use std::f64::consts::PI;

/// The fold angle of one joint: signed, valley positive.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Angle {
    /// Given, in radians.
    Given(f64),
    /// Solved for; `sign` is +1 for a valley, −1 for a mountain, 0 for a
    /// joint that may turn either way.
    Free { sign: f64 },
}

/// A hinge between two bodies.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Joint {
    pub left: usize,
    pub right: usize,
    /// A point on the axis, and its unit direction, with the left body on
    /// the left of the axis seen from the side `side × direction` points
    /// to.
    pub point: Vec3,
    pub direction: Vec3,
    /// A unit vector square to the axis: where the third point the loops
    /// are sampled at lies.
    pub side: Vec3,
    /// How far from `point` the loop's sample points lie (the hinge's
    /// length): the residuals are lengths at this reach.
    pub reach: f64,
    /// The curve the joint is, for messages.
    pub id: u64,
    /// The radius of the joint's neutral fibre: 0 for a crease (a turn
    /// about the axis), `R + K T` for a bend of sheet metal (inner radius
    /// `R`, thickness `T`, K-factor `K`), whose strip of the flat pattern
    /// `|ρ| r` wide is rolled onto the cylinder of that radius.
    pub radius: f64,
    /// A unit vector square to the axis in the left body's plane, from
    /// the left body towards the right one (across the bend's strip);
    /// only read when `radius` is not 0.
    pub across: Vec3,
}

impl Joint {
    /// A crease: a turn about the axis, no radius.
    #[allow(clippy::too_many_arguments)]
    pub fn crease(
        left: usize,
        right: usize,
        point: Vec3,
        direction: Vec3,
        side: Vec3,
        reach: f64,
        id: u64,
    ) -> Joint {
        Joint {
            left,
            right,
            point,
            direction,
            side,
            reach,
            id,
            radius: 0.0,
            across: -side,
        }
    }

    /// Where the right body goes as it turns at `ρ > 0` (a valley):
    /// `across × direction`, the side `direction × side` points to (for a
    /// crease, `across` is `−side`; in a sheet in the xy plane, +z).
    fn towards(&self) -> Vec3 {
        self.across.cross(self.direction)
    }

    /// The axis of the turn at angle `rho`: `point` for a crease; for a
    /// bend `point` is where the strip starts on the neutral fibre and
    /// the axis lies `radius` from it on the side the body turns to.
    fn centre(&self, rho: f64) -> Vec3 {
        if self.radius == 0.0 {
            self.point
        } else {
            self.point + self.towards() * (self.radius * rho.signum())
        }
    }

    /// The motion of the right body relative to the left one at angle
    /// `rho`: `T_right = T_left ∘ motion(rho)`. A turn about the axis by
    /// −ρ; for a bend first the shift back across the strip by the arc
    /// length the neutral fibre takes up, `|ρ| r` (the bend allowance at
    /// this angle), then the turn about the axis `r` from the strip's
    /// start: `Rot(C(ρ), −ρ) ∘ Shift(−|ρ| r û)`. The motion and its
    /// velocity are continuous through ρ = 0 (both one-sided velocities
    /// are the crease's there).
    pub(super) fn motion(&self, rho: f64) -> Option<Transform> {
        let turn = Transform::rotation_about(self.centre(rho), self.direction, -rho)?;
        if self.radius == 0.0 {
            return Some(turn);
        }
        let shift = Transform::translation(self.across * (-rho.abs() * self.radius));
        Some(shift.then(&turn))
    }

    /// d/dρ of a point `x` carried by `motion(rho)`, with the axis where
    /// the motion `t` before it puts it: the turn's `−D × (x − C)` and,
    /// for a bend, the shift's `−sign(ρ) r û` turned by the motion.
    pub(super) fn velocity(&self, t: &Transform, x: Vec3, rho: f64) -> Vec3 {
        let turn = -(t
            .vector(self.direction)
            .cross(x - t.point(self.centre(rho))));
        if self.radius == 0.0 {
            return turn;
        }
        let Some(rot) = Transform::rotation(self.direction, -rho) else {
            return turn;
        };
        turn - t.vector(rot.vector(self.across)) * (rho.signum() * self.radius)
    }
}

/// Bodies joined by hinges.
#[derive(Clone, Debug, PartialEq)]
pub struct Linkage {
    pub bodies: usize,
    pub joints: Vec<Joint>,
    /// The size of the whole (the diagonal of its box): the scale of the
    /// stopping rule and of the kinds' penalty.
    pub size: f64,
}

/// A folded linkage: where each body went, and the angle of every joint.
#[derive(Clone, Debug, PartialEq)]
pub struct Folded {
    /// Per body, from its flat place to its folded one.
    pub transforms: Vec<Transform>,
    /// Per joint, the fold angle (valley positive), in radians.
    pub angles: Vec<f64>,
    /// The largest distance by which a loop fails to close.
    pub gap: f64,
    /// The bodies that stay in place: the fixed one, then one per other
    /// piece.
    pub roots: Vec<usize>,
    /// Per body, which piece (index into `roots`) it belongs to.
    pub piece: Vec<usize>,
    /// The work the solve did, in the units of [`MAX_WORK`].
    pub work: u64,
    /// How many of the first step's guesses were solved (every guess of
    /// every first step tried, halved ones included).
    pub guesses: usize,
}

/// The largest step of the given angles (10 degrees), and the first one
/// from the flat sheet (1 degree).
const MAX_STEP: f64 = PI / 18.0;
const FIRST_STEP: f64 = PI / 180.0;
/// The first step's guesses, in units of its given step: every unknown
/// turned its kind's way by this much, at most [`FIRST_CAP`]. Which one is
/// taken is decided by the half-step test, not by the guess.
const FIRST_GUESSES: [f64; 5] = [1.0, 3.0, 10.0, 30.0, 100.0];
const FIRST_CAP: f64 = PI * 17.0 / 18.0;
/// A solution of the first step that passed the half-step test: solved at
/// the step, at half of it and (asked only when it is compared with
/// another) at a quarter.
struct Candidate {
    solved: Vec<f64>,
    half: Vec<f64>,
    /// `None` not asked yet; `Some(None)` asked and not solved.
    quarter: Option<Option<Vec<f64>>>,
    ran_out: bool,
    /// The largest gap of `solved`.
    gap: f64,
    /// Its error bound ([`error_bound`]), `None` not measured yet.
    bound: Option<Bound>,
}
/// Unknowns without a kind whose sign patterns are all tried at the first
/// step, at most (2^6 patterns); beyond, all one way and all the other.
const OPEN_PATTERN_BITS: usize = 6;
/// A step is halved at most this many times in a row.
const MAX_HALVINGS: usize = 24;
/// The first step is halved at most this many times.
const FIRST_HALVINGS: usize = 4;
const MAX_ITERATIONS: usize = 60;
/// From this many unknowns the sparse solve is taken below the dense
/// solve's limit too, where it is cheaper: below, the dense factor costs
/// at most about 11 000 multiplications, no concern to the budget.
const SPARSE_FROM: usize = 32;
/// The most work one fold may do, counted as the multiplications of the
/// dense factorisations and of the residual evaluations (about 20 s on
/// a laptop at the worst).
pub const MAX_WORK: u64 = 20_000_000_000;
/// A pivot of the scaled normal equations below this leaves its joint
/// undetermined.
const RANK_PIVOT: f64 = 1e-10;

struct Forest {
    parent: Vec<Option<(usize, usize)>>,
    order: Vec<usize>,
    depth: Vec<usize>,
    tree: Vec<bool>,
    /// −1 when the parent is the joint's left body, +1 when its right.
    sign: Vec<f64>,
    roots: Vec<usize>,
    piece: Vec<usize>,
}

fn forest(link: &Linkage, fixed: usize) -> Forest {
    let n = link.bodies;
    let mut adj: Vec<Vec<(usize, usize)>> = vec![Vec::new(); n];
    for (k, j) in link.joints.iter().enumerate() {
        if let Some(list) = adj.get_mut(j.left) {
            list.push((k, j.right));
        }
        if let Some(list) = adj.get_mut(j.right) {
            list.push((k, j.left));
        }
    }
    let mut parent = vec![None; n];
    let mut seen = vec![false; n];
    let mut depth = vec![0usize; n];
    let mut piece = vec![0usize; n];
    let mut tree = vec![false; link.joints.len()];
    let mut sign = vec![0.0; link.joints.len()];
    let mut order = Vec::with_capacity(n);
    let mut roots = Vec::new();
    for root in std::iter::once(fixed).chain(0..n) {
        if seen.get(root).copied() != Some(false) {
            continue;
        }
        let index = roots.len();
        roots.push(root);
        if let Some(s) = seen.get_mut(root) {
            *s = true;
        }
        let mut queue = VecDeque::from([root]);
        while let Some(f) = queue.pop_front() {
            order.push(f);
            if let Some(p) = piece.get_mut(f) {
                *p = index;
            }
            let d = depth.get(f).copied().unwrap_or(0);
            for &(k, g) in adj.get(f).into_iter().flatten() {
                if seen.get(g).copied() != Some(false) {
                    continue;
                }
                if let Some(s) = seen.get_mut(g) {
                    *s = true;
                }
                if let Some(slot) = parent.get_mut(g) {
                    *slot = Some((k, f));
                }
                if let Some(slot) = depth.get_mut(g) {
                    *slot = d + 1;
                }
                if let Some(t) = tree.get_mut(k) {
                    *t = true;
                }
                let left = link.joints.get(k).is_some_and(|j| j.left == f);
                if let Some(s) = sign.get_mut(k) {
                    *s = if left { -1.0 } else { 1.0 };
                }
                queue.push_back(g);
            }
        }
    }
    Forest {
        parent,
        order,
        depth,
        tree,
        sign,
        roots,
        piece,
    }
}

/// Where every body is at the angles `rho`, and per joint of the forest
/// the placement of its left body (which places its axis and the motion
/// its velocity is the derivative of).
struct State {
    transforms: Vec<Transform>,
    parent_motion: Vec<Transform>,
}

fn place(link: &Linkage, forest: &Forest, rho: &[f64]) -> Option<State> {
    let mut transforms = vec![Transform::IDENTITY; link.bodies];
    let mut parent_motion = vec![Transform::IDENTITY; link.joints.len()];
    for &f in &forest.order {
        let Some(Some((k, p))) = forest.parent.get(f).copied() else {
            continue;
        };
        let tp = *transforms.get(p)?;
        let joint = link.joints.get(k)?;
        let forward = joint.motion(*rho.get(k)?)?;
        // T_right = T_left ∘ motion(ρ): a child on the right takes the
        // motion, a child on the left its inverse (for a crease that is the
        // turn by −ρ; for a bend, whose motion shifts too, it is not).
        let left_parent = *forest.sign.get(k)? < 0.0;
        let motion = if left_parent {
            forward
        } else {
            forward.inverse()?
        };
        // T_child = T_parent ∘ motion: the motion first.
        let tc = motion.then(&tp);
        if let Some(slot) = parent_motion.get_mut(k) {
            *slot = if left_parent { tp } else { tc };
        }
        if let Some(slot) = transforms.get_mut(f) {
            *slot = tc;
        }
    }
    Some(State {
        transforms,
        parent_motion,
    })
}

fn path(forest: &Forest, mut f: usize) -> Vec<usize> {
    let mut out = Vec::new();
    while let Some(Some((k, p))) = forest.parent.get(f).copied() {
        out.push(k);
        f = p;
        if out.len() > forest.parent.len() {
            break;
        }
    }
    out
}

pub(super) struct Row {
    pub gap: Vec3,
    pub by: Vec<(usize, Vec3)>,
}

/// One loop: the joint that closes it and the paths of its two bodies.
type Loop = (usize, Vec<usize>, Vec<usize>);

/// The loops the solve closes: the forest's fundamental cycles, written
/// in the fixed body's frame (the dense solve), or a short cycle basis of
/// chains, each written in its own frame (the sparse solve, past
/// [`MAX_DENSE`] unknowns: `sparse.rs`).
enum Loops {
    Tree(Vec<Loop>),
    Chains(Vec<Chain>),
}

impl Loops {
    /// The joint that closes each loop, in order.
    fn closing(&self) -> Vec<usize> {
        match self {
            Loops::Tree(l) => l.iter().map(|(k, ..)| *k).collect(),
            Loops::Chains(c) => c.iter().map(|c| c.closing).collect(),
        }
    }
}

/// The chain of the fundamental cycle of joint `k`: from its left body up
/// the forest to the bodies' common ancestor and down to its right body.
fn fundamental_chain(link: &Linkage, forest: &Forest, k: usize) -> Option<Chain> {
    let j = link.joints.get(k)?;
    let (mut a, mut b) = (j.left, j.right);
    let (mut up_a, mut up_b) = (Vec::new(), Vec::new());
    let mut guard = 0usize;
    while a != b {
        guard += 1;
        if guard > 2 * link.bodies + 2 {
            return None;
        }
        if forest.depth.get(a) >= forest.depth.get(b) {
            let (kk, p) = forest.parent.get(a).copied().flatten()?;
            up_a.push((kk, a, p));
            a = p;
        } else {
            let (kk, p) = forest.parent.get(b).copied().flatten()?;
            up_b.push((kk, b, p));
            b = p;
        }
    }
    let mut steps = up_a;
    steps.extend(up_b.into_iter().rev().map(|(kk, body, p)| (kk, p, body)));
    Some(Chain { closing: k, steps })
}

/// Counts the work done, and refuses past [`MAX_WORK`].
pub(super) struct Budget(pub u64);

impl Budget {
    pub(super) fn spend(&mut self, work: u64) -> Result<(), FoldError> {
        self.0 = self.0.saturating_add(work);
        if self.0 > MAX_WORK {
            Err(FoldError::TooMuchWork)
        } else {
            Ok(())
        }
    }
}

fn rows(
    link: &Linkage,
    forest: &Forest,
    loops: &Loops,
    rho: &[f64],
    column: Option<&[Option<usize>]>,
    budget: &mut Budget,
) -> Result<Vec<Row>, FoldError> {
    let loops = match loops {
        Loops::Tree(l) => l,
        Loops::Chains(c) => return chain_rows(link, c, rho, column, budget),
    };
    let state = place(link, forest, rho).ok_or(BROKEN)?;
    let mut out = Vec::with_capacity(loops.len() * 3);
    // About 40 multiplications to place a body, 60 per sample point and
    // per joint on its paths.
    let mut work = (link.bodies as u64).saturating_mul(40);
    for (l, path_a, path_b) in loops {
        let joint = link.joints.get(*l).ok_or(BROKEN)?;
        let ta = *state.transforms.get(joint.left).ok_or(BROKEN)?;
        let tb = *state.transforms.get(joint.right).ok_or(BROKEN)?;
        let motion = joint.motion(*rho.get(*l).ok_or(BROKEN)?).ok_or(BROKEN)?;
        let (p, d, s, r) = (joint.point, joint.direction, joint.side, joint.reach);
        for q in [p, p + d * r, p + s * r] {
            work = work.saturating_add(60);
            let x1 = ta.point(motion.point(q));
            let x2 = tb.point(q);
            let mut by: BTreeMap<usize, Vec3> = BTreeMap::new();
            if let Some(column) = column {
                work = work.saturating_add(60 * (1 + path_a.len() + path_b.len()) as u64);
                let mut add = |k: usize, v: Vec3| {
                    if let Some(Some(c)) = column.get(k) {
                        *by.entry(*c).or_insert(Vec3::ZERO) += v;
                    }
                };
                for (path, x, s) in [(path_a, x1, 1.0), (path_b, x2, -1.0)] {
                    for &k in path {
                        if let (Some(j), Some(t), Some(&sign)) = (
                            link.joints.get(k),
                            state.parent_motion.get(k),
                            forest.sign.get(k),
                        ) {
                            // `t` places the joint's left body: through the
                            // motion when the child is on the right (+), its
                            // inverse when the child is on the left (−: the
                            // derivative of an inverse is minus the
                            // motion's velocity at the same point).
                            add(
                                k,
                                j.velocity(t, x, rho.get(k).copied().unwrap_or(0.0)) * (-sign * s),
                            );
                        }
                    }
                }
                add(*l, joint.velocity(&ta, x1, *rho.get(*l).unwrap_or(&0.0)));
            }
            out.push(Row {
                gap: x1 - x2,
                by: by.into_iter().collect(),
            });
        }
    }
    budget.spend(work)?;
    Ok(out)
}

pub(super) const BROKEN: FoldError = FoldError::Broken {
    reason: "a joint or a body is missing, or a motion is not finite",
};

/// The largest gap of the rows; NaN when any gap is not a number (a
/// plain `f64::max` would drop it and read a broken loop as closed).
fn largest_gap(rows: &[Row]) -> f64 {
    rows.iter().map(|r| r.gap.length()).fold(0.0, |m, g| {
        if m.is_nan() || g.is_nan() {
            f64::NAN
        } else {
            m.max(g)
        }
    })
}

fn cost(rows: &[Row]) -> f64 {
    rows.iter().map(|r| r.gap.length_squared()).sum()
}

/// Folds the linkage by `angles` (one per joint), keeping body `fixed` in
/// place. `tol` and `angular` are the document's tolerances.
pub fn fold_linkage(
    link: &Linkage,
    angles: &[Angle],
    fixed: usize,
    tol: f64,
    angular: f64,
) -> Result<Folded, FoldError> {
    fold_linkage_with(link, angles, fixed, tol, angular, MAX_DENSE)
}

/// [`fold_linkage`], with the dense solve up to `dense_up_to` unknowns
/// and the sparse one past it (`fold_linkage` uses [`MAX_DENSE`]; a
/// test asks for either to compare them).
pub fn fold_linkage_with(
    link: &Linkage,
    angles: &[Angle],
    fixed: usize,
    tol: f64,
    angular: f64,
    dense_up_to: usize,
) -> Result<Folded, FoldError> {
    fold_linkage_within(
        link,
        angles,
        fixed,
        tol,
        angular,
        dense_up_to,
        &mut Budget(0),
    )
}

/// [`fold_linkage_with`], its work counted in `budget` with what the
/// caller already spent (a fold solved again shares one budget).
pub(super) fn fold_linkage_within(
    link: &Linkage,
    angles: &[Angle],
    fixed: usize,
    tol: f64,
    angular: f64,
    dense_up_to: usize,
    budget: &mut Budget,
) -> Result<Folded, FoldError> {
    if !(tol.is_finite() && tol > 0.0 && angular.is_finite() && angular > 0.0) {
        return Err(FoldError::Tolerance);
    }
    let nj = link.joints.len();
    if angles.len() != nj || fixed >= link.bodies {
        return Err(FoldError::Broken {
            reason: "the angles do not match the joints",
        });
    }
    let source = |k: usize| link.joints.get(k).map_or(0, |j| j.id);
    for (k, a) in angles.iter().enumerate() {
        if let Angle::Given(v) = a
            && !(v.is_finite() && v.abs() <= PI)
        {
            return Err(FoldError::AngleRange { id: source(k) });
        }
    }
    let forest = forest(link, fixed);

    // The loops, and the forest's joints that lie on one.
    let mut loops: Vec<Loop> = Vec::new();
    let mut on_loop = vec![false; nj];
    for (k, j) in link.joints.iter().enumerate() {
        if forest.tree.get(k).copied().unwrap_or(false) {
            continue;
        }
        let (mut a, mut b) = (j.left, j.right);
        let mut guard = 0usize;
        while a != b && guard <= 2 * link.bodies {
            guard += 1;
            let deeper_a = forest.depth.get(a) >= forest.depth.get(b);
            let at = if deeper_a { &mut a } else { &mut b };
            let Some((kk, p)) = forest.parent.get(*at).copied().flatten() else {
                break;
            };
            if let Some(m) = on_loop.get_mut(kk) {
                *m = true;
            }
            *at = p;
        }
        if let Some(m) = on_loop.get_mut(k) {
            *m = true;
        }
        loops.push((k, path(&forest, j.left), path(&forest, j.right)));
    }

    // The unknowns; one on no loop is determined by nothing.
    let mut column: Vec<Option<usize>> = vec![None; nj];
    let mut signs = Vec::new();
    let mut free = Vec::new();
    let mut lonely = Vec::new();
    for (k, a) in angles.iter().enumerate() {
        if let Angle::Free { sign } = a {
            if !on_loop.get(k).copied().unwrap_or(false) {
                lonely.push(source(k));
                continue;
            }
            if let Some(slot) = column.get_mut(k) {
                *slot = Some(free.len());
            }
            free.push(k);
            signs.push(*sign);
        }
    }
    if !lonely.is_empty() {
        lonely.sort_unstable();
        lonely.dedup();
        return Err(FoldError::Undetermined { ids: lonely });
    }
    if free.len() > MAX_FREE {
        return Err(FoldError::TooManyFree { count: free.len() });
    }
    // Past `dense_up_to` unknowns the sparse solve: a short cycle basis, its
    // loops in their own frames, the normal equations as a skyline. Below
    // it, from `SPARSE_FROM` unknowns, the sparse solve too where its
    // factor costs less than a quarter of the dense one: the dense solve
    // eliminates with pivoting, the skyline in a fixed order, so the
    // quarter is a margin in favour of the pivoted dense solve, taken
    // over only where the sparse one is clearly cheaper, not a number
    // fitted to a pattern. `dense_up_to = usize::MAX` is the dense solve
    // always.
    let sparse = |budget: &mut Budget| -> Result<(Vec<Chain>, Pattern), FoldError> {
        let fundamental: Vec<Chain> = loops
            .iter()
            .map(|(k, ..)| fundamental_chain(link, &forest, *k))
            .collect::<Option<_>>()
            .ok_or(BROKEN)?;
        let chains = cycle_basis(link, fundamental, budget)?;
        let cliques: Vec<Vec<usize>> = chains
            .iter()
            .map(|c| {
                std::iter::once(c.closing)
                    .chain(c.steps.iter().map(|s| s.0))
                    .filter_map(|k| column.get(k).copied().flatten())
                    .collect()
            })
            .collect();
        let pattern = Pattern::new(free.len(), &cliques, budget)?;
        Ok((chains, pattern))
    };
    let n = free.len();
    let chosen = if n > dense_up_to {
        Some(sparse(&mut *budget)?)
    } else if dense_up_to != usize::MAX && n >= SPARSE_FROM {
        let dense = (n as u64).saturating_pow(3) / 3;
        match sparse(&mut *budget) {
            Ok((chains, pattern)) if pattern.flops.saturating_mul(4) < dense => {
                Some((chains, pattern))
            }
            Err(FoldError::TooMuchWork) => return Err(FoldError::TooMuchWork),
            _ => None,
        }
    } else {
        None
    };
    let (loops, pattern) = match chosen {
        Some((chains, pattern)) => (Loops::Chains(chains), Some(pattern)),
        None => (Loops::Tree(loops), None),
    };
    let given = |scale: f64| -> Vec<f64> {
        angles
            .iter()
            .map(|a| match a {
                Angle::Given(v) => v * scale,
                Angle::Free { .. } => 0.0,
            })
            .collect()
    };
    let largest = angles
        .iter()
        .map(|a| match a {
            Angle::Given(v) => v.abs(),
            Angle::Free { .. } => 0.0,
        })
        .fold(0.0, f64::max);
    let size = if link.size.is_finite() && link.size > 0.0 {
        link.size
    } else {
        1.0
    };
    let goal = (tol * 1e-3).max(size * 1e-13);
    // A body's size plus the largest bend radius: what the derivative's
    // Lipschitz constant is measured in ([`error_bound`]).
    let reach = size
        + link
            .joints
            .iter()
            .map(|j| j.radius.abs())
            .filter(|r| r.is_finite())
            .fold(0.0, f64::max);
    let solver = Solver {
        link,
        forest: &forest,
        loops: &loops,
        free: &free,
        column: &column,
        signs: &signs,
        goal,
        weight: size,
        pattern: pattern.as_ref(),
        polish: false,
    };
    // The first step's solutions solved on past the stopping rule, as long
    // as the gap at least halves an iteration (`polish`): near the flat
    // sheet the loops' derivative is near singular, and two solves of one
    // branch that each stop at the rule can differ by more than the
    // tolerances' angle. Solved on, each lies within its own error bound
    // ([`error_bound`]) of a solution, and two solves of one branch are
    // one where uniqueness is proven too ([`merge_radius`]).
    let fine = Solver {
        polish: true,
        ..solver
    };

    // A guess solved at `base`: the solution, whether the iterations ran
    // out and its largest gap, when it closes within the tolerance with
    // every solved joint its kind's way.
    let attempt_with = |solver: &Solver,
                        base: &[f64],
                        guess: Vec<f64>,
                        budget: &mut Budget|
     -> Result<Option<(Vec<f64>, bool, f64)>, FoldError> {
        let (solved, ran_out, _) = match solver.levenberg(base, guess, budget) {
            Ok(found) => found,
            Err(FoldError::TooMuchWork) => return Err(FoldError::TooMuchWork),
            Err(_) => return Ok(None),
        };
        let here = match rows(
            link,
            &forest,
            &loops,
            &solver.with(base, &solved),
            None,
            budget,
        ) {
            Ok(here) => here,
            Err(FoldError::TooMuchWork) => return Err(FoldError::TooMuchWork),
            Err(_) => return Ok(None),
        };
        let kinds = solved.iter().zip(&signs).all(|(v, s)| s * v >= -angular);
        let gap = largest_gap(&here);
        Ok((gap <= tol && kinds).then_some((solved, ran_out, gap)))
    };
    let attempt = |base: &[f64], guess: Vec<f64>, budget: &mut Budget| {
        attempt_with(&solver, base, guess, budget)
    };
    // The first step's: solved on (`fine`).
    let first_attempt = |base: &[f64], guess: Vec<f64>, budget: &mut Budget| {
        attempt_with(&fine, base, guess, budget)
    };
    let size_of = |v: &[f64]| v.iter().fold(0.0f64, |m, a| m.max(a.abs()));
    // Continuation from the flat sheet: a first step by the kinds, then a
    // tangent predictor and a corrector.
    let mut x = vec![0.0; free.len()];
    let mut exhausted = false;
    let mut guesses = 0usize;
    if largest > 0.0 && !free.is_empty() {
        let mut done = 0.0f64;
        let mut h = (FIRST_STEP / largest).min(1.0);
        let mut halvings = 0usize;
        let mut first = true;
        while done < 1.0 {
            let target = (done + h).min(1.0);
            let base = given(target);
            let mut found = None;
            if first {
                // At the flat sheet the branches meet and there is no
                // tangent: the kinds pick the branch. A solution is taken
                // when it lies on a branch through the flat sheet: solved
                // again at half the step, it is half as far (within a
                // quarter), as every smooth branch through the flat sheet
                // is near it, whatever its slope.
                let step = (target - done) * largest;
                // Every guess is tried: each turns the unknowns their kinds'
                // way (an unknown without a kind, sign 0, both ways, every
                // pattern of them up to `MAX_OPEN_PATTERNS`), by 1 to 100
                // times the step. Two different solutions that both pass
                // are two branches the kinds do not tell apart: refused.
                let same = 10.0 * (angular + tol / size);
                // How far a candidate may lie from the solution it stands
                // for, in radians ([`error_bound`]), measured once.
                let bound_of =
                    |c: &mut Candidate, budget: &mut Budget| -> Result<Bound, FoldError> {
                        if let Some(b) = c.bound {
                            return Ok(b);
                        }
                        // Valid only near the solution: a solve that ran out
                        // or stopped above the goal explains no difference.
                        let solved = &c.solved;
                        let b = candidate_bound(c.ran_out, c.gap, goal, || {
                            let here = rows(
                                link,
                                &forest,
                                &loops,
                                &solver.with(&base, solved),
                                Some(&column),
                                budget,
                            )?;
                            error_bound(&here, free.len(), reach, pattern.as_ref(), budget)
                        })?;
                        c.bound = Some(b);
                        Ok(b)
                    };
                // A candidate solved again at a quarter of the step, from a
                // quarter of its solution (as its half is from half of
                // it): `None` when that solve does not close or does not
                // stay on the line through the flat sheet.
                let quarter_of = |c: &mut Candidate,
                                  budget: &mut Budget|
                 -> Result<Option<Vec<f64>>, FoldError> {
                    if c.quarter.is_none() {
                        let guess: Vec<f64> = c.solved.iter().map(|v| v / 4.0).collect();
                        let solved = first_attempt(&given(target / 4.0), guess.clone(), budget)?
                            .map(|(q, ..)| q)
                            .filter(|q| max_diff(q, &guess) <= 0.25 * size_of(&guess) + angular);
                        c.quarter = Some(solved);
                    }
                    Ok(c.quarter.clone().flatten())
                };
                // The solved angles in which `a` and `b` are two branches
                // (`parting_angles`): a difference past both error bounds,
                // solved again at half and a quarter of the step; any
                // difference that shrinks is two branches.
                let parting = |a: &mut Candidate,
                               b: &mut Candidate,
                               budget: &mut Budget|
                 -> Result<Vec<usize>, FoldError> {
                    let diff = |x: &[f64], y: &[f64]| -> Vec<f64> {
                        x.iter().zip(y).map(|(p, q)| p - q).collect()
                    };
                    let apart = same.max(merge_radius(bound_of(a, budget)?, bound_of(b, budget)?));
                    let d = diff(&a.solved, &b.solved);
                    let h = diff(&a.half, &b.half);
                    let mut quarter = || -> Result<Option<Vec<f64>>, FoldError> {
                        Ok(match (quarter_of(a, budget)?, quarter_of(b, budget)?) {
                            (Some(qa), Some(qb)) => Some(diff(&qa, &qb)),
                            _ => None,
                        })
                    };
                    parting_angles(&d, &h, &mut quarter, apart)
                };
                let mut passed: Vec<Candidate> = Vec::new();
                let mut parted: Option<Vec<usize>> = None;
                for guess in first_guesses(&signs, step) {
                    guesses += 1;
                    let Some((solved, ran_out, first_gap)) =
                        first_attempt(&base, guess, &mut *budget)?
                    else {
                        continue;
                    };
                    // The same solution again, within the tolerance's angle:
                    // nothing to tell apart.
                    if passed.iter().any(|p| max_diff(&p.solved, &solved) <= same) {
                        continue;
                    }
                    let mut candidate = Candidate {
                        solved,
                        half: Vec::new(),
                        quarter: None,
                        ran_out,
                        gap: first_gap,
                        bound: None,
                    };
                    // One solution found twice: near the flat sheet the
                    // loops' derivative is near singular, so two solves of
                    // one branch can differ by more than the tolerance's
                    // angle (a Miura sheet of 3 × 12 cells: 4.5e-8 rad at a
                    // 1-degree step, stopped at the goal). Two are one
                    // solution only where that is proven: each within its
                    // error bound of a solution, and the other's inside the
                    // ball where that solution is unique ([`merge_radius`]);
                    // otherwise two solutions.
                    let mut duplicate = false;
                    for p in &mut passed {
                        let apart = merge_radius(
                            bound_of(p, &mut *budget)?,
                            bound_of(&mut candidate, &mut *budget)?,
                        );
                        if max_diff(&p.solved, &candidate.solved) <= apart {
                            duplicate = true;
                            break;
                        }
                    }
                    if duplicate {
                        continue;
                    }
                    let half_guess: Vec<f64> = candidate.solved.iter().map(|v| v / 2.0).collect();
                    let Some((half, ..)) =
                        first_attempt(&given(target / 2.0), half_guess.clone(), &mut *budget)?
                    else {
                        continue;
                    };
                    if max_diff(&half, &half_guess) > 0.25 * size_of(&half_guess) + angular {
                        continue;
                    }
                    candidate.half = half;
                    // Two solutions, each on a branch through the flat
                    // sheet: two branches where their difference shrinks
                    // with the step, asked at half and at a quarter of it
                    // per solved angle.
                    let mut one = false;
                    for p in &mut passed {
                        let apart = parting(p, &mut candidate, &mut *budget)?;
                        if apart.is_empty() {
                            one = true;
                            break;
                        }
                        if parted.is_none() {
                            parted = Some(apart);
                        }
                    }
                    if !one {
                        passed.push(candidate);
                    }
                }
                match passed.as_slice() {
                    [] => {}
                    [one] => found = Some((one.solved.clone(), one.ran_out)),
                    [a, ..] => {
                        // A continuum of solutions (the given angles leave
                        // freedom) is undetermined, not two branches.
                        let at = rows(
                            link,
                            &forest,
                            &loops,
                            &solver.with(&base, &a.solved),
                            Some(&column),
                            &mut *budget,
                        )?;
                        let loose = undetermined(&at, free.len(), pattern.as_ref(), &mut *budget)?;
                        if !loose.is_empty() {
                            let mut ids: Vec<u64> = loose
                                .iter()
                                .filter_map(|&c| free.get(c))
                                .map(|&k| source(k))
                                .collect();
                            ids.sort_unstable();
                            ids.dedup();
                            return Err(FoldError::Undetermined { ids });
                        }
                        let mut ids: Vec<u64> = parted
                            .unwrap_or_default()
                            .iter()
                            .filter_map(|&c| free.get(c))
                            .map(|&k| source(k))
                            .collect();
                        ids.sort_unstable();
                        ids.dedup();
                        return Err(FoldError::Ambiguous { ids });
                    }
                }
            } else {
                // Predict along the tangent: J_free dx = −J_given dg at the
                // last solution; correct from there. Taken when the
                // corrector lands within a quarter of the predicted motion.
                let now = given(done);
                let dg: Vec<f64> = base.iter().zip(&now).map(|(a, b)| a - b).collect();
                let dx = solver.tangent(&solver.with(&now, &x), &dg, &mut *budget)?;
                let moved = size_of(&dx);
                let predicted: Vec<f64> = x.iter().zip(&dx).map(|(a, d)| a + d).collect();
                if let Some((solved, ran_out, _)) = attempt(&base, predicted.clone(), &mut *budget)?
                {
                    let off = solved
                        .iter()
                        .zip(&predicted)
                        .fold(0.0f64, |m, (a, b)| m.max((a - b).abs()));
                    if off <= 0.25 * moved + angular {
                        found = Some((solved, ran_out));
                    }
                }
            }
            match found {
                Some((solved, ran_out)) => {
                    x = solved;
                    exhausted = ran_out;
                    done = target;
                    first = false;
                    halvings = 0;
                    h = (h * 2.0).min(MAX_STEP / largest);
                }
                None => {
                    halvings += 1;
                    h /= 2.0;
                    // Near the flat sheet the branches are straight, so a
                    // first step that finds none does not find one smaller.
                    let most = if first { FIRST_HALVINGS } else { MAX_HALVINGS };
                    if halvings > most {
                        return Err(FoldError::NotFollowed { at: done * largest });
                    }
                }
            }
        }
    }
    let mut rho = given(1.0);
    for (c, &k) in free.iter().enumerate() {
        if let (Some(slot), Some(&v)) = (rho.get_mut(k), x.get(c)) {
            *slot = v;
        }
    }
    let final_rows = rows(link, &forest, &loops, &rho, Some(&column), &mut *budget)?;
    let gap = largest_gap(&final_rows);
    if gap.is_nan() || gap > tol {
        if exhausted {
            return Err(FoldError::NotConverged { gap });
        }
        let worst = final_rows
            .chunks(3)
            .zip(loops.closing())
            .map(|(r, k)| (largest_gap(r), k))
            // A loop whose gap is not a number is the worst.
            .fold((0.0f64, 0usize), |a, b| {
                if a.0.is_nan() {
                    a
                } else if b.0.is_nan() || b.0 > a.0 {
                    b
                } else {
                    a
                }
            });
        return Err(FoldError::NotClosing {
            id: source(worst.1),
            gap,
        });
    }
    // Determined: also at the flat sheet, where the branches meet and the
    // angles given do not say which way the others go. There (every given
    // angle 0, so the solve stayed at the flat sheet) the rank of the
    // loops' derivative says nothing: every crease through a vertex turns
    // about an axis in the sheet's plane, so the rank drops whether or not
    // a fold can leave the flat sheet with the given angles kept at 0.
    // Whether one can is asked as the first step asks it: a branch through
    // the flat sheet is a solution with the given angles at 0, turned by
    // the kinds by 1 to 100 times the first step, at least a quarter of
    // that step from the flat sheet, that solved again from half of it
    // stays at half (within a quarter). With none the flat sheet is the
    // only fold near it, and it is the answer.
    if !free.is_empty() {
        let mut loose = undetermined(&final_rows, free.len(), pattern.as_ref(), &mut *budget)?;
        if !loose.is_empty() && largest == 0.0 {
            let flat = given(0.0);
            let mut branch = false;
            for guess in first_guesses(&signs, FIRST_STEP) {
                let Some((solved, ..)) = attempt(&flat, guess, &mut *budget)? else {
                    continue;
                };
                if size_of(&solved) < 0.25 * FIRST_STEP {
                    continue;
                }
                let half_guess: Vec<f64> = solved.iter().map(|v| v / 2.0).collect();
                let Some((half, ..)) = attempt(&flat, half_guess.clone(), &mut *budget)? else {
                    continue;
                };
                if max_diff(&half, &half_guess) <= 0.25 * size_of(&half_guess) + angular {
                    branch = true;
                    break;
                }
            }
            if !branch {
                loose.clear();
            }
        }
        if !loose.is_empty() {
            let mut ids: Vec<u64> = loose
                .iter()
                .filter_map(|&c| free.get(c))
                .map(|&k| source(k))
                .collect();
            ids.sort_unstable();
            ids.dedup();
            return Err(FoldError::Undetermined { ids });
        }
    }
    // Every solved joint turns its kind's way, and no further than flat
    // on itself (the angle wrapped into (−π, π] first).
    for (c, &k) in free.iter().enumerate() {
        let (Some(&v), Some(&s)) = (rho.get(k), signs.get(c)) else {
            continue;
        };
        let wrapped = v.psin().patan2(v.pcos());
        if s * wrapped < -angular {
            return Err(FoldError::WrongWay {
                id: source(k),
                angle: wrapped,
            });
        }
        if let Some(slot) = rho.get_mut(k) {
            *slot = wrapped;
        }
    }
    let state = place(link, &forest, &rho).ok_or(BROKEN)?;
    if !state
        .transforms
        .iter()
        .all(|t| t.point(Vec3::ZERO).is_finite() && t.vector(Vec3::new(1.0, 1.0, 1.0)).is_finite())
    {
        return Err(FoldError::NotConverged { gap: f64::INFINITY });
    }
    Ok(Folded {
        transforms: state.transforms,
        angles: rho,
        gap,
        roots: forest.roots,
        piece: forest.piece,
        work: budget.0,
        guesses,
    })
}

struct Solver<'a> {
    link: &'a Linkage,
    forest: &'a Forest,
    loops: &'a Loops,
    free: &'a [usize],
    column: &'a [Option<usize>],
    signs: &'a [f64],
    goal: f64,
    weight: f64,
    pattern: Option<&'a Pattern>,
    /// Past the goal, solved on while the gap at least halves an
    /// iteration: it stops where the rounding of the loops' transforms
    /// sets the gap, whatever that is for the sheet's size and the loops'
    /// lengths.
    polish: bool,
}

impl Solver<'_> {
    fn with(&self, base: &[f64], x: &[f64]) -> Vec<f64> {
        let mut rho = base.to_vec();
        for (c, &k) in self.free.iter().enumerate() {
            if let (Some(slot), Some(&v)) = (rho.get_mut(k), x.get(c)) {
                *slot = v;
            }
        }
        rho
    }

    /// The tangent of the branch: the motion `dx` of the solved angles
    /// for a change `dg` of the given ones (per joint) at the angles `rho`,
    /// from `J_free dx = −J_given dg` in least squares. Fails as
    /// undetermined, naming the joints, where `J_free` is singular: more
    /// freedom than the given angles fix, or a bifurcation where the branch
    /// does not go on one way.
    fn tangent(&self, rho: &[f64], dg: &[f64], budget: &mut Budget) -> Result<Vec<f64>, FoldError> {
        let n = self.free.len();
        let free_rows = rows(
            self.link,
            self.forest,
            self.loops,
            rho,
            Some(self.column),
            budget,
        )?;
        let loose = undetermined(&free_rows, n, self.pattern, budget)?;
        if !loose.is_empty() {
            let mut ids: Vec<u64> = loose
                .iter()
                .filter_map(|&c| self.free.get(c))
                .filter_map(|&k| self.link.joints.get(k).map(|j| j.id))
                .collect();
            ids.sort_unstable();
            ids.dedup();
            return Err(FoldError::Undetermined { ids });
        }
        // Every joint as a column: the free ones first, then the given.
        let mut all: Vec<Option<usize>> = self.column.to_vec();
        let mut given_change = Vec::new();
        for (k, slot) in all.iter_mut().enumerate() {
            if slot.is_none() {
                *slot = Some(n + given_change.len());
                given_change.push(dg.get(k).copied().unwrap_or(0.0));
            }
        }
        let m = n + given_change.len();
        let all_rows = rows(self.link, self.forest, self.loops, rho, Some(&all), budget)?;
        let everything = || FoldError::Undetermined {
            ids: self
                .free
                .iter()
                .filter_map(|&k| self.link.joints.get(k).map(|j| j.id))
                .collect(),
        };
        if let Some(p) = self.pattern {
            // The free block as the skyline; the right-hand side row by
            // row, −J_freeᵀ (J_given dg).
            budget.spend(p.flops.saturating_add(all_rows.len() as u64 * 64))?;
            let (mut a, _) = p.normal(&all_rows, n);
            let mut b = vec![0.0; n];
            for row in &all_rows {
                let mut moved = Vec3::ZERO;
                for &(c, v) in &row.by {
                    if c >= n {
                        moved += v * given_change.get(c - n).copied().unwrap_or(0.0);
                    }
                }
                for &(c, v) in &row.by {
                    if let Some(slot) = b.get_mut(c) {
                        *slot -= v.dot(moved);
                    }
                }
            }
            p.factor(&mut a, None).ok_or_else(everything)?;
            return p.solve(&a, &b).ok_or_else(everything);
        }
        budget.spend(
            (m as u64)
                .saturating_pow(2)
                .saturating_add((n as u64).saturating_pow(3) / 3),
        )?;
        let (gram, _) = normal(&all_rows, m);
        let mut a = vec![0.0; n * n];
        let mut b = vec![0.0; n];
        for i in 0..n {
            for j in 0..n {
                if let (Some(slot), Some(&v)) = (a.get_mut(i * n + j), gram.get(i * m + j)) {
                    *slot = v;
                }
            }
            let mut sum = 0.0;
            for (g, d) in given_change.iter().enumerate() {
                sum -= gram.get(i * m + n + g).copied().unwrap_or(0.0) * d;
            }
            if let Some(slot) = b.get_mut(i) {
                *slot = sum;
            }
        }
        solve_dense(&mut a, &mut b, n).ok_or_else(everything)?;
        Ok(b)
    }

    /// The diagonal entry of unknown `i` in normal equations of `n`
    /// unknowns, dense or as the pattern's skyline.
    fn diagonal<'v>(&self, gram: &'v mut [f64], i: usize) -> Option<&'v mut f64> {
        match self.pattern {
            Some(p) => p.diagonal_mut(gram, i),
            None => gram.get_mut(i * self.free.len() + i),
        }
    }

    /// The kinds' penalty: an unknown turned against its kind costs the
    /// sheet's size per radian, as a gap would; zero at every solution
    /// that keeps the kinds.
    fn penalty(&self, x: &[f64]) -> f64 {
        x.iter()
            .zip(self.signs)
            .map(|(v, s)| {
                let wrong = (s * v).min(0.0) * self.weight;
                wrong * wrong
            })
            .sum()
    }

    /// Levenberg–Marquardt from `x`; the solution, whether the iterations
    /// ran out first, and its largest gap.
    fn levenberg(
        &self,
        base: &[f64],
        mut x: Vec<f64>,
        budget: &mut Budget,
    ) -> Result<(Vec<f64>, bool, f64), FoldError> {
        let n = self.free.len();
        let cube = (n as u64).saturating_pow(3);
        let mut lambda = 1e-3;
        // Polishing: the last point within the goal and its gap.
        let mut within: Option<(Vec<f64>, f64)> = None;
        let mut gap = f64::INFINITY;
        for _ in 0..MAX_ITERATIONS {
            let now = rows(
                self.link,
                self.forest,
                self.loops,
                &self.with(base, &x),
                Some(self.column),
                budget,
            )?;
            gap = largest_gap(&now);
            if gap <= self.goal {
                if !self.polish || gap == 0.0 {
                    return Ok((x, false, gap));
                }
                // Stagnation: the gap no longer halves, so rounding, not
                // the solve, sets it now. The better of the two is kept.
                if let Some((last, last_gap)) = &within
                    && gap > 0.5 * last_gap
                {
                    return Ok(if gap < *last_gap {
                        (x, false, gap)
                    } else {
                        (last.clone(), false, *last_gap)
                    });
                }
                within = Some((x.clone(), gap));
            }
            let c0 = cost(&now) + self.penalty(&x);
            let (mut gram, mut grad) = match self.pattern {
                Some(p) => p.normal(&now, n),
                None => normal(&now, n),
            };
            let w2 = self.weight * self.weight;
            for (i, (v, s)) in x.iter().zip(self.signs).enumerate() {
                if s * v < 0.0 {
                    if let Some(d) = self.diagonal(&mut gram, i) {
                        *d += w2;
                    }
                    if let Some(g) = grad.get_mut(i) {
                        *g += w2 * v;
                    }
                }
            }
            let mut moved = false;
            for _ in 0..12 {
                budget.spend(self.pattern.map_or(cube / 3, |p| p.flops) + 1)?;
                let mut a = gram.clone();
                for i in 0..n {
                    if let Some(d) = self.diagonal(&mut a, i) {
                        *d += lambda * (*d + 1e-12);
                    }
                }
                let mut b: Vec<f64> = grad.iter().map(|g| -g).collect();
                let solved = match self.pattern {
                    Some(p) => p
                        .factor(&mut a, None)
                        .and_then(|_| p.solve(&a, &b))
                        .map(|x| b = x),
                    None => solve_dense(&mut a, &mut b, n),
                };
                if solved.is_some() {
                    let trial: Vec<f64> = x.iter().zip(&b).map(|(x, d)| x + d).collect();
                    // A trial whose motions are not finite is no better.
                    let tried = match rows(
                        self.link,
                        self.forest,
                        self.loops,
                        &self.with(base, &trial),
                        None,
                        budget,
                    ) {
                        Ok(r) => cost(&r) + self.penalty(&trial),
                        Err(FoldError::TooMuchWork) => return Err(FoldError::TooMuchWork),
                        Err(_) => f64::INFINITY,
                    };
                    if tried < c0 {
                        x = trial;
                        lambda = (lambda / 3.0).max(1e-15);
                        moved = true;
                        break;
                    }
                }
                lambda *= 4.0;
            }
            if !moved {
                return Ok((x, false, gap));
            }
        }
        // The last step's gap, not yet measured.
        let now = rows(
            self.link,
            self.forest,
            self.loops,
            &self.with(base, &x),
            None,
            budget,
        )?;
        let last = largest_gap(&now);
        if let Some((best, best_gap)) = within
            && (last.is_nan() || last >= best_gap)
        {
            return Ok((best, false, best_gap));
        }
        Ok((x, gap > self.goal && last > self.goal, last))
    }
}

/// `JᵀJ` (dense, row-major) and `Jᵀr`, from each row's few entries.
fn normal(rows: &[Row], n: usize) -> (Vec<f64>, Vec<f64>) {
    let mut gram = vec![0.0; n * n];
    let mut grad = vec![0.0; n];
    for row in rows {
        for &(a, va) in &row.by {
            if let Some(g) = grad.get_mut(a) {
                *g += va.dot(row.gap);
            }
            for &(b, vb) in &row.by {
                if let Some(slot) = gram.get_mut(a * n + b) {
                    *slot += va.dot(vb);
                }
            }
        }
    }
    (gram, grad)
}

/// Solves `a x = b` in place (`b` becomes `x`) by Gaussian elimination
/// with partial pivoting; `None` when `a` is singular.
fn solve_dense(a: &mut [f64], b: &mut [f64], n: usize) -> Option<()> {
    for col in 0..n {
        let mut best = col;
        let mut size = 0.0;
        for row in col..n {
            let v = a.get(row * n + col)?.abs();
            if v > size {
                size = v;
                best = row;
            }
        }
        if !(size > 0.0 && size.is_finite()) {
            return None;
        }
        if best != col {
            for k in 0..n {
                a.swap(best * n + k, col * n + k);
            }
            b.swap(best, col);
        }
        let pivot = *a.get(col * n + col)?;
        for row in col + 1..n {
            let factor = a.get(row * n + col)? / pivot;
            if factor == 0.0 {
                continue;
            }
            for k in col..n {
                let v = *a.get(col * n + k)?;
                *a.get_mut(row * n + k)? -= factor * v;
            }
            let v = *b.get(col)?;
            *b.get_mut(row)? -= factor * v;
        }
    }
    for col in (0..n).rev() {
        let mut sum = *b.get(col)?;
        for k in col + 1..n {
            sum -= a.get(col * n + k)? * b.get(k)?;
        }
        *b.get_mut(col)? = sum / a.get(col * n + col)?;
    }
    b.iter().all(|v| v.is_finite()).then_some(())
}

/// The unknowns the derivative leaves free: a symmetric pivoted
/// elimination of the normal equations with their columns scaled to unit
/// length, the columns never pivoted.
fn undetermined(
    rows: &[Row],
    n: usize,
    pattern: Option<&Pattern>,
    budget: &mut Budget,
) -> Result<Vec<usize>, FoldError> {
    if let Some(p) = pattern {
        // The skyline in its fixed order: for a positive semi-definite
        // matrix a zero pivot in any symmetric order is a dependent
        // column (sparse.rs).
        budget.spend(p.flops.saturating_add(n as u64))?;
        let (mut vals, _) = p.normal(rows, n);
        let diag: Vec<f64> = (0..n)
            .map(|i| p.diagonal_mut(&mut vals, i).map_or(0.0, |d| *d))
            .collect();
        p.scale(&mut vals, &diag);
        let scaled = vals.clone();
        let mut loose = p.factor(&mut vals, Some(RANK_PIVOT)).ok_or(BROKEN)?;
        let smallest = p.smallest_pivot(&vals, &loose);
        name_free(p, &scaled, &mut loose, smallest, budget)?;
        return Ok(loose);
    }
    budget.spend((n as u64).saturating_pow(3))?;
    let (mut gram, _) = normal(rows, n);
    let diag: Vec<f64> = (0..n)
        .map(|i| gram.get(i * n + i).copied().unwrap_or(0.0))
        .collect();
    for i in 0..n {
        for j in 0..n {
            let (di, dj) = (
                diag.get(i).copied().unwrap_or(0.0),
                diag.get(j).copied().unwrap_or(0.0),
            );
            if let Some(v) = gram.get_mut(i * n + j) {
                *v = if di > 0.0 && dj > 0.0 {
                    *v / (di * dj).sqrt()
                } else {
                    0.0
                };
            }
        }
    }
    let scaled = gram.clone();
    let mut smallest = f64::INFINITY;
    let mut loose = Vec::new();
    let mut left: Vec<usize> = (0..n).collect();
    while !left.is_empty() {
        let Some((at, &p)) = left.iter().enumerate().max_by(|a, b| {
            let va = gram.get(a.1 * n + a.1).copied().unwrap_or(0.0);
            let vb = gram.get(b.1 * n + b.1).copied().unwrap_or(0.0);
            va.total_cmp(&vb).then(b.1.cmp(a.1))
        }) else {
            break;
        };
        let pivot = gram.get(p * n + p).copied().unwrap_or(0.0);
        if pivot.is_nan() || pivot <= RANK_PIVOT {
            loose.extend(left.iter().copied());
            break;
        }
        smallest = smallest.min(pivot);
        left.remove(at);
        for &i in &left {
            let f = gram.get(i * n + p).copied().unwrap_or(0.0) / pivot;
            for &j in &left {
                let v = gram.get(p * n + j).copied().unwrap_or(0.0);
                if let Some(slot) = gram.get_mut(i * n + j) {
                    *slot -= f * v;
                }
            }
        }
    }
    loose.sort_unstable();
    if (smallest <= CONFIRM_PIVOT || !loose.is_empty()) && n > 0 {
        // Confirmed on the same matrix as the sparse test: one clique of
        // every unknown, its skyline the whole lower triangle.
        let all: Vec<usize> = (0..n).collect();
        let p = Pattern::new(n, &[all], budget)?;
        let vals = p.skyline_of(&scaled, n);
        name_free(&p, &vals, &mut loose, smallest, budget)?;
    }
    Ok(loose)
}

/// A smallest pivot at most this (a million times the rank pivot) is
/// confirmed by an estimate of the smallest eigenvalue: an elimination,
/// pivoted or not, can show no small pivot where the matrix is near
/// singular (Kahan's matrix is the textbook case).
const CONFIRM_PIVOT: f64 = RANK_PIVOT * 1e6;

/// The most near-null directions [`name_free`] looks for.
const MAX_NULL_VECTORS: usize = 16;

/// Which unknowns are free, in words that do not depend on an
/// elimination's order. An elimination finds how many directions the
/// normal equations leave free (its pivots at most the rank pivot), and
/// can miss a near dependence (Kahan's matrix shows no small pivot,
/// pivoted or not); so when any pivot was dropped, or the smallest kept
/// is near the rank pivot, the near-null directions of the unit-scaled
/// equations `scaled` (skyline of `p`) are found by inverse iteration
/// on their factor shifted by the rank pivot, each orthogonal to those
/// found, as long as its eigenvalue is at most the rank pivot (one more
/// than the elimination found, at most [`MAX_NULL_VECTORS`]). The
/// unknowns that carry a tenth of a direction's largest component are the
/// free ones: dense and sparse ask this of the same matrix and name the
/// same unknowns. Where it finds fewer directions than the elimination
/// (or hits the cap) the elimination's unknowns are named too.
fn name_free(
    p: &Pattern,
    scaled: &[f64],
    loose: &mut Vec<usize>,
    smallest: f64,
    budget: &mut Budget,
) -> Result<(), FoldError> {
    if loose.is_empty() && smallest > CONFIRM_PIVOT {
        return Ok(());
    }
    let n = p.unknowns();
    let shift = RANK_PIVOT;
    let mut b = scaled.to_vec();
    for i in 0..n {
        if let Some(d) = p.diagonal_mut(&mut b, i) {
            *d += shift;
        }
    }
    let wanted = (loose.len() + 1).min(MAX_NULL_VECTORS);
    // Per vector at most 80 iterations, each a solve with the factor and
    // the orthogonalisation against the vectors found (2 n each).
    budget.spend(
        p.flops.saturating_add(
            (wanted as u64).saturating_mul(80).saturating_mul(
                (p.entries() as u64)
                    .saturating_mul(2)
                    .saturating_add((n as u64).saturating_mul(2 * wanted as u64 + 1)),
            ),
        ),
    )?;
    if p.factor(&mut b, None).is_none() {
        return Ok(());
    }
    let dot = |x: &[f64], y: &[f64]| x.iter().zip(y).map(|(a, b)| a * b).sum::<f64>();
    let mut found: Vec<Vec<f64>> = Vec::new();
    for r in 0..wanted {
        let away = |x: &mut Vec<f64>, found: &[Vec<f64>]| {
            for v in found {
                let k = dot(x, v);
                x.iter_mut().zip(v).for_each(|(a, b)| *a -= k * b);
            }
        };
        // A start of every unknown, the same in every run.
        let mut x: Vec<f64> = (0..n)
            .map(|i| 0.5 + ((i * 7919 + r * 104_729) % 1000) as f64 / 1000.0)
            .collect();
        away(&mut x, &found);
        let mut lambda = f64::INFINITY;
        let mut good = false;
        for _ in 0..80 {
            let norm = dot(&x, &x).sqrt();
            if !(norm.is_finite() && norm > 0.0) {
                break;
            }
            x.iter_mut().for_each(|v| *v /= norm);
            let Some(mut y) = p.solve(&b, &x) else {
                break;
            };
            away(&mut y, &found);
            let grow = dot(&y, &y).sqrt();
            if !(grow.is_finite() && grow > 0.0) {
                break;
            }
            let next = 1.0 / grow - shift;
            let settled = (next - lambda).abs() <= 1e-3 * shift.max(next.abs());
            lambda = next;
            x = y.iter().map(|v| v / grow).collect();
            good = true;
            if settled {
                break;
            }
        }
        if !(good && lambda <= RANK_PIVOT) {
            break;
        }
        found.push(x);
    }
    if found.is_empty() {
        return Ok(());
    }
    let mut names: std::collections::BTreeSet<usize> =
        if found.len() < loose.len() || loose.len() >= MAX_NULL_VECTORS {
            loose.iter().copied().collect()
        } else {
            std::collections::BTreeSet::new()
        };
    for v in &found {
        let top = v.iter().fold(0.0f64, |m, a| m.max(a.abs()));
        for (i, a) in v.iter().enumerate() {
            if a.abs() >= 0.1 * top {
                names.insert(i);
            }
        }
    }
    *loose = names.into_iter().collect();
    Ok(())
}

/// A solve's error bound and what proves it: `radius` (radians; 0 when
/// nothing is proven), `sigma` the derivative's smallest singular value
/// from below and `lipschitz` its Lipschitz constant from above.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Bound {
    radius: f64,
    sigma: f64,
    lipschitz: f64,
}

impl Bound {
    /// Nothing proven: explains no difference.
    const NONE: Bound = Bound {
        radius: 0.0,
        sigma: 0.0,
        lipschitz: f64::INFINITY,
    };
}

/// How far, in radians, a solve of the loops may lie from a solution,
/// valid only near it: twice the Gauss–Newton step it would still take,
/// `2 ‖r‖ / σ_min(J)`, with `r` the gaps of `rows` and
/// `σ_min(J) ≥ √λ_min · min_i ‖J e_i‖` (`λ_min` the smallest eigenvalue of
/// the normal equations scaled to unit diagonal, as the rank test scales
/// them, divided by 1.01 since inverse iteration estimates it from above),
/// and only where Kantorovich's condition `h = L ‖r‖ / σ² ≤ ½` holds for a
/// Lipschitz constant `L` of `J` taken from above. The loops' rows are
/// redundant but consistent (every loop closes at a solution), so the
/// zero-residual Gauss–Newton form of the theorem applies: a solution lies
/// within the radius, whatever way the solve stopped, and it is the only
/// one within `σ / L` ([`merge_radius`]). Nothing is proven
/// ([`Bound::NONE`]) where `λ_min` is at most the rank pivot, where
/// inverse iteration did not settle, where `h` is above a half and where
/// anything is not finite; the caller also takes none for a solve that ran
/// out or stopped above its goal ([`candidate_bound`]). `reach` bounds a
/// body's size plus a bend's radius.
fn error_bound(
    rows: &[Row],
    n: usize,
    reach: f64,
    pattern: Option<&Pattern>,
    budget: &mut Budget,
) -> Result<Bound, FoldError> {
    if n == 0 {
        return Ok(Bound::NONE);
    }
    let owned;
    let p = match pattern {
        Some(p) => p,
        None => {
            // Dense: one clique of every unknown, the whole lower triangle.
            let all: Vec<usize> = (0..n).collect();
            owned = Pattern::new(n, &[all], budget)?;
            &owned
        }
    };
    budget.spend(p.entries() as u64)?;
    let (mut vals, _) = p.normal(rows, n);
    let diag: Vec<f64> = (0..n)
        .map(|i| p.diagonal_mut(&mut vals, i).map_or(0.0, |d| *d))
        .collect();
    let column = diag.iter().copied().fold(f64::INFINITY, f64::min);
    if !(column.is_finite() && column > 0.0) {
        return Ok(Bound::NONE);
    }
    p.scale(&mut vals, &diag);
    let Some(lambda) = smallest_eigenvalue(p, &vals, budget)? else {
        return Ok(Bound::NONE);
    };
    if lambda.is_nan() || lambda <= RANK_PIVOT {
        return Ok(Bound::NONE);
    }
    // The derivative's Lipschitz constant, from above: an entry of row
    // `r` is a point's velocity about a joint on its loop, `D × (X − C)`
    // and, for a bend, `−sign(ρ) r rot(û)`; turning another joint of the
    // loop moves `D`, `C`, `X` and `rot(û)`, each by at most the distance
    // along the loop plus the bend's radius, and the loop crosses at most
    // `k + 1` bodies of diameter at most the sheet's size (`k` the row's
    // unknowns). So each entry changes by at most `3 (k + 1) reach` per
    // radian of each of the `k` (one-sided at a bend's ρ = 0, where the
    // velocity is continuous), and in the Frobenius norm
    // `L ≤ 3 reach √Σ_r k_r² (k_r + 1)²`.
    let lipschitz = 3.0
        * reach
        * rows
            .iter()
            .map(|r| {
                let k = r.by.len() as f64;
                k * k * (k + 1.0) * (k + 1.0)
            })
            .sum::<f64>()
            .sqrt();
    Ok(kantorovich(
        cost(rows).sqrt(),
        lambda / 1.01 * column,
        lipschitz,
    ))
}

/// Whether a solve's error bound may explain a difference: only a solve
/// that did not run out of iterations and closed within its goal is near
/// its solution.
fn bound_usable(ran_out: bool, gap: f64, goal: f64) -> bool {
    !ran_out && gap <= goal
}

/// A candidate's bound: [`Bound::NONE`] without measuring when the solve
/// ran out or stopped above its goal, else what `measure` proves.
fn candidate_bound(
    ran_out: bool,
    gap: f64,
    goal: f64,
    measure: impl FnOnce() -> Result<Bound, FoldError>,
) -> Result<Bound, FoldError> {
    if bound_usable(ran_out, gap, goal) {
        measure()
    } else {
        Ok(Bound::NONE)
    }
}

/// Twice the Gauss–Newton step `‖r‖ / σ` (`sigma2 = σ²`, from below)
/// when Kantorovich's condition `h = L ‖r‖ / σ² ≤ ½` holds, so that a
/// solution lies within it; [`Bound::NONE`] otherwise (and for anything
/// not finite).
fn kantorovich(residual: f64, sigma2: f64, lipschitz: f64) -> Bound {
    if !(residual.is_finite()
        && sigma2 > 0.0
        && sigma2.is_finite()
        && lipschitz.is_finite()
        && lipschitz > 0.0)
    {
        return Bound::NONE;
    }
    let h = lipschitz * residual / sigma2;
    let sigma = sigma2.sqrt();
    let radius = 2.0 * residual / sigma;
    if h <= 0.5 && radius.is_finite() {
        Bound {
            radius,
            sigma,
            lipschitz,
        }
    } else {
        Bound::NONE
    }
}

/// How far apart two solves may be and still be one solution, proven: the
/// sum of their radii when that is shown, else 0. Each lies within its
/// radius of a solution (existence); for those two to be one, the other's
/// solution must lie in the ball where the solution near `a` is unique,
/// which Kantorovich's theorem makes at least `σ_a / L` round `a`. With
/// `a` the solve of the larger `σ`, the other's solution lies within
/// `d + r_b ≤ r_a + 2 r_b` of `a` when `d ≤ r_a + r_b`, so the sum is
/// proven only where `r_a + 2 r_b < σ_a / L` (`L` the larger of the two).
/// Both radii from `h ≤ ½` alone can add up to `2 σ / L`, the least
/// distance two solutions may lie apart: existence alone merges nothing.
fn merge_radius(a: Bound, b: Bound) -> f64 {
    if !(a.radius > 0.0 && b.radius > 0.0) {
        return 0.0;
    }
    let (big, small) = if a.sigma >= b.sigma { (a, b) } else { (b, a) };
    let l = a.lipschitz.max(b.lipschitz);
    if big.radius + 2.0 * small.radius < big.sigma / l {
        a.radius + b.radius
    } else {
        0.0
    }
}

/// The smallest eigenvalue of the unit-scaled normal equations `scaled`
/// (skyline of `p`), by inverse iteration on their factor shifted by the
/// rank pivot; `None` when the factor fails or the estimate does not
/// settle (to a thousandth) within its iterations.
fn smallest_eigenvalue(
    p: &Pattern,
    scaled: &[f64],
    budget: &mut Budget,
) -> Result<Option<f64>, FoldError> {
    let n = p.unknowns();
    let shift = RANK_PIVOT;
    budget.spend(
        p.flops.saturating_add(
            80u64.saturating_mul(
                (p.entries() as u64)
                    .saturating_mul(2)
                    .saturating_add(n as u64),
            ),
        ),
    )?;
    let mut b = scaled.to_vec();
    for i in 0..n {
        if let Some(d) = p.diagonal_mut(&mut b, i) {
            *d += shift;
        }
    }
    if p.factor(&mut b, None).is_none() {
        return Ok(None);
    }
    let dot = |x: &[f64], y: &[f64]| x.iter().zip(y).map(|(a, b)| a * b).sum::<f64>();
    // A start of every unknown, the same in every run.
    let mut x: Vec<f64> = (0..n)
        .map(|i| 0.5 + ((i * 7919) % 1000) as f64 / 1000.0)
        .collect();
    let mut lambda = None;
    for _ in 0..80 {
        let norm = dot(&x, &x).sqrt();
        if !(norm.is_finite() && norm > 0.0) {
            break;
        }
        x.iter_mut().for_each(|v| *v /= norm);
        let Some(y) = p.solve(&b, &x) else {
            break;
        };
        let grow = dot(&y, &y).sqrt();
        if !(grow.is_finite() && grow > 0.0) {
            break;
        }
        let next = (1.0 / grow - shift).max(0.0);
        let settled = lambda.is_some_and(|l: f64| (next - l).abs() <= 1e-3 * shift.max(next));
        lambda = Some(next);
        x = y;
        if settled {
            return Ok(lambda);
        }
    }
    Ok(None)
}

/// The solved angles in which two solutions of the first step are two
/// branches, from their differences at the step (`d`), at half of it
/// (`h`) and, asked only when needed, at a quarter (`quarter`, `None`
/// when that step could not be solved). A difference of more than `same`
/// (the tolerances' angle) is the solver's play, one branch solved twice,
/// only when it does not shrink with the step: at half and at a quarter
/// it is still at least three quarters as large (the play does not
/// shrink, it grows as the derivative near the flat sheet gets more
/// singular). Every difference that shrinks is two branches, whatever
/// the rate: linear (two slopes), quadratic (two branches tangent at the
/// flat sheet), as the square root of the step (a pitchfork). A quarter
/// step that cannot be solved explains nothing away: two branches
/// (refused, never a silent choice).
fn parting_angles(
    d: &[f64],
    h: &[f64],
    quarter: &mut dyn FnMut() -> Result<Option<Vec<f64>>, FoldError>,
    same: f64,
) -> Result<Vec<usize>, FoldError> {
    let mut parts = Vec::new();
    let mut steady = Vec::new();
    for (i, &di) in d.iter().enumerate() {
        if di.is_nan() || di.abs() <= same {
            continue;
        }
        let hi = h.get(i).copied().unwrap_or(0.0);
        if hi.abs() < 0.75 * di.abs() {
            parts.push(i);
        } else {
            steady.push(i);
        }
    }
    if steady.is_empty() {
        return Ok(parts);
    }
    match quarter()? {
        None => parts.extend(steady),
        Some(q) => parts.extend(steady.into_iter().filter(|&i| {
            let (di, qi) = (
                d.get(i).copied().unwrap_or(0.0),
                q.get(i).copied().unwrap_or(0.0),
            );
            qi.abs() < 0.75 * di.abs()
        })),
    }
    parts.sort_unstable();
    Ok(parts)
}

/// The first step's guesses at `step`: every unknown turned its kind's way
/// (an unknown without a kind, sign 0, both ways, every pattern of them up
/// to [`OPEN_PATTERN_BITS`]) by each of [`FIRST_GUESSES`] times the step,
/// at most [`FIRST_CAP`].
fn first_guesses(signs: &[f64], step: f64) -> Vec<Vec<f64>> {
    let open = signs.iter().filter(|s| **s == 0.0).count();
    let patterns: Vec<Vec<f64>> = if open == 0 {
        vec![Vec::new()]
    } else if open <= OPEN_PATTERN_BITS {
        (0..1usize << open)
            .map(|m| {
                (0..open)
                    .map(|b| if m >> b & 1 == 1 { -1.0 } else { 1.0 })
                    .collect()
            })
            .collect()
    } else {
        vec![vec![1.0; open], vec![-1.0; open]]
    };
    let mut out = Vec::with_capacity(FIRST_GUESSES.len() * patterns.len());
    for f in FIRST_GUESSES {
        for pattern in &patterns {
            let mut way = pattern.iter();
            out.push(
                signs
                    .iter()
                    .map(|s| {
                        let sign = if *s == 0.0 {
                            way.next().copied().unwrap_or(1.0)
                        } else {
                            *s
                        };
                        sign * (step * f).min(FIRST_CAP)
                    })
                    .collect(),
            );
        }
    }
    out
}

fn max_diff(a: &[f64], b: &[f64]) -> f64 {
    a.iter()
        .zip(b)
        .fold(0.0f64, |m, (x, y)| m.max((x - y).abs()))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    /// Two branches 1e-6 rad apart: a solve that ran out of iterations (or
    /// stopped above its goal) gets no bound, whatever it would measure,
    /// so the two are never one solution; Kantorovich's condition refuses
    /// a bound far from the solution and keeps one near it.
    #[test]
    fn a_solve_that_ran_out_never_merges_two_branches_and_existence_alone_merges_nothing() {
        let (a, b) = ([0.1, 0.2], [0.1, 0.2 + 1e-6]);
        let goal = 1e-13;
        for (ran_out, gap) in [(true, 1e-14), (false, 1e-9), (false, f64::NAN)] {
            assert!(!bound_usable(ran_out, gap, goal));
        }
        assert!(bound_usable(false, 1e-14, goal));
        // The seam: a ran-out solve is not measured, whatever it would show.
        let wide = Bound {
            radius: 1.0,
            sigma: 1.0,
            lipschitz: 1e-6,
        };
        for (ran_out, gap) in [(true, 1e-14), (false, 1e-9)] {
            let got = candidate_bound(ran_out, gap, goal, || Ok(wide)).unwrap();
            assert_eq!(got, Bound::NONE);
            assert_eq!(merge_radius(got, wide), 0.0);
            assert!(max_diff(&a, &b) > merge_radius(got, wide));
        }
        assert_eq!(
            candidate_bound(false, 1e-14, goal, || Ok(wide)).unwrap(),
            wide
        );
        // σ² of 1e-12 (σ_min 1e-6), L of 1e3: a residual of 1e-9 has
        // h = 1e3 · 1e-9 / 1e-12 = 1e6, nothing proven (it would be 2e-3,
        // past the branches' 1e-6); a residual of 1e-17 has h = 1e-2,
        // radius 2e-11.
        assert_eq!(kantorovich(1e-9, 1e-12, 1e3), Bound::NONE);
        let near = kantorovich(1e-17, 1e-12, 1e3);
        assert!((near.radius - 2e-11).abs() <= 1e-24, "{near:?}");
        // Unique within σ / L = 1e-9 of each: 2e-11 + 4e-11 < 1e-9, so the
        // sum of the radii is proven one solution, and 1e-6 is past it.
        assert!((merge_radius(near, near) - 4e-11).abs() <= 1e-24);
        assert!(max_diff(&a, &b) > merge_radius(near, near));
        for bad in [f64::NAN, f64::INFINITY] {
            assert_eq!(kantorovich(bad, 1e-12, 1e3), Bound::NONE);
            assert_eq!(kantorovich(1e-17, bad, 1e3), Bound::NONE);
            assert_eq!(kantorovich(1e-17, 1e-12, bad), Bound::NONE);
        }
        assert_eq!(kantorovich(1e-17, 0.0, 1e3), Bound::NONE);
        // h = 0.4 on both (σ = L = 1, residual 0.4): each radius 0.8 is
        // proven, but two solutions may lie 2σ/L = 2 apart and the radii
        // add to 1.6: not proven one (0.8 + 1.6 is past σ/L = 1).
        let h4 = kantorovich(0.4, 1.0, 1.0);
        assert!((h4.radius - 0.8).abs() <= 1e-15, "{h4:?}");
        assert_eq!(merge_radius(h4, h4), 0.0);
        // And with σ apart: the larger σ's ball is the one asked.
        let small = kantorovich(1e-4, 1.0, 1.0);
        let large = kantorovich(1e-4, 4.0, 1.0);
        assert!(merge_radius(small, large) > 0.0);
        assert_eq!(merge_radius(small, large), merge_radius(large, small));
    }

    /// One pair of solutions per way of parting: linear, quadratic
    /// (tangent at the flat sheet), as the square root (a pitchfork), the
    /// solver's play (not shrinking), a quarter step that cannot be
    /// solved, and a difference within the tolerances.
    #[test]
    fn every_difference_that_shrinks_with_the_step_is_two_branches() {
        let same = 1e-8;
        let d = 1e-4;
        let cases: [(f64, Option<f64>, bool); 6] = [
            (d / 2.0, Some(d / 4.0), true),
            (d / 4.0, Some(d / 16.0), true),
            (d / 2f64.sqrt(), Some(d / 2.0), true),
            (1.1 * d, Some(1.3 * d), false),
            (0.9 * d, None, true),
            (0.9 * d, Some(0.2 * d), true),
        ];
        for (h, q, two) in cases {
            let mut asked = false;
            let mut quarter = || {
                asked = true;
                Ok(q.map(|q| vec![q]))
            };
            let parts = parting_angles(&[d], &[h], &mut quarter, same).unwrap();
            assert_eq!(!parts.is_empty(), two, "h {h} q {q:?}");
            // The quarter is asked only when the half did not shrink.
            assert_eq!(asked, h.abs() >= 0.75 * d, "h {h}");
        }
        // Within the tolerances: nothing to tell apart (below).
        let mut never = || -> Result<Option<Vec<f64>>, FoldError> { Ok(None) };
        assert!(
            parting_angles(&[1e-9], &[1e-9], &mut never, same)
                .unwrap()
                .is_empty()
        );
    }

    /// Kahan's matrix as the loops' derivative: no elimination shows a
    /// small pivot (unpivoted the smallest is about 2e-6), yet its
    /// smallest eigenvalue is about 4e-15. The dense and the sparse rank
    /// tests both confirm by the smallest eigenvalue and name the same
    /// unknowns.
    #[test]
    fn a_near_dependence_no_pivot_shows_is_undetermined_dense_and_sparse_alike() {
        let n = 100usize;
        let (c, s) = (1.2f64.pcos(), 1.2f64.psin());
        let rows: Vec<Row> = (0..n)
            .map(|i| {
                let scale = s.ppowi(i as i32);
                let by = (i..n)
                    .map(|j| {
                        let k = if j == i { scale } else { -c * scale };
                        (j, Vec3::new(k, 0.0, 0.0))
                    })
                    .collect();
                Row {
                    gap: Vec3::ZERO,
                    by,
                }
            })
            .collect();
        let mut budget = Budget(0);
        let dense = undetermined(&rows, n, None, &mut budget).unwrap();
        let cliques: Vec<Vec<usize>> = rows
            .iter()
            .map(|r| r.by.iter().map(|(j, _)| *j).collect())
            .collect();
        let pattern = Pattern::new(n, &cliques, &mut budget).unwrap();
        let sparse = undetermined(&rows, n, Some(&pattern), &mut budget).unwrap();
        assert!(
            !dense.is_empty(),
            "the dense test misses the near dependence"
        );
        assert_eq!(dense, sparse);
        // A healthy matrix (the identity) has nothing undetermined.
        let plain: Vec<Row> = (0..n)
            .map(|i| Row {
                gap: Vec3::ZERO,
                by: vec![(i, Vec3::new(1.0, 0.0, 0.0))],
            })
            .collect();
        assert!(
            undetermined(&plain, n, None, &mut budget)
                .unwrap()
                .is_empty()
        );
    }
}
