//! Rigid folding: bodies (the facets) joined by hinges (the fold creases),
//! each body moving rigidly, turned about its hinges by their angles.
//!
//! The solve knows nothing of crease patterns: it takes a [`Linkage`],
//! bodies and joints, each joint with its own axis in space (a point, a
//! direction, a side to sample) and the motion of its right body
//! relative to its left one at an angle ([`Joint::motion`]: a turn about
//! the axis by −ρ). A crease pattern is one linkage ([`fold`] builds it
//! from an [`Arrangement`]); a bend of sheet metal would be another, its
//! joint's motion a turn plus the offset of its radius
//! (docs/design/origami.md › *One fold mechanism*).
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

use super::{FoldError, MAX_FREE};
use craft3d_math::{Transform, Vec3};
use std::collections::{BTreeMap, VecDeque};
use std::f64::consts::PI;

/// The fold angle of one joint: signed, valley positive.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Angle {
    /// Given, in radians.
    Given(f64),
    /// Solved for; `sign` is +1 for a valley and −1 for a mountain.
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
}

impl Joint {
    /// The motion of the right body relative to the left one at angle
    /// `rho`: `T_right = T_left ∘ motion(rho)`. A turn about the axis by
    /// −ρ; a joint with an offset (a bend's radius) would add its
    /// translation here and its velocity in [`Joint::velocity`].
    fn motion(&self, rho: f64) -> Option<Transform> {
        Transform::rotation_about(self.point, self.direction, -rho)
    }

    /// d/dρ of a point `x` carried by `motion`, with the axis where the
    /// motion `t` before it puts it.
    fn velocity(&self, t: &Transform, x: Vec3) -> Vec3 {
        -(t.vector(self.direction).cross(x - t.point(self.point)))
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
/// A step is halved at most this many times in a row.
const MAX_HALVINGS: usize = 24;
const MAX_ITERATIONS: usize = 60;
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
/// the motion of its parent (which places its axis).
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
        let motion = joint.motion(-forest.sign.get(k)? * rho.get(k)?)?;
        if let Some(slot) = parent_motion.get_mut(k) {
            *slot = tp;
        }
        // T_child = T_parent ∘ motion: the motion first.
        if let Some(slot) = transforms.get_mut(f) {
            *slot = motion.then(&tp);
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

struct Row {
    gap: Vec3,
    by: Vec<(usize, Vec3)>,
}

/// One loop: the joint that closes it and the paths of its two bodies.
type Loop = (usize, Vec<usize>, Vec<usize>);

/// Counts the work done, and refuses past [`MAX_WORK`].
struct Budget(u64);

impl Budget {
    fn spend(&mut self, work: u64) -> Result<(), FoldError> {
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
    loops: &[Loop],
    rho: &[f64],
    column: Option<&[Option<usize>]>,
    budget: &mut Budget,
) -> Result<Vec<Row>, FoldError> {
    let state = place(link, forest, rho).ok_or(BROKEN)?;
    let mut out = Vec::with_capacity(loops.len() * 3);
    let mut work = link.bodies as u64;
    for (l, path_a, path_b) in loops {
        let joint = link.joints.get(*l).ok_or(BROKEN)?;
        let ta = *state.transforms.get(joint.left).ok_or(BROKEN)?;
        let tb = *state.transforms.get(joint.right).ok_or(BROKEN)?;
        let motion = joint.motion(*rho.get(*l).ok_or(BROKEN)?).ok_or(BROKEN)?;
        let (p, d, s, r) = (joint.point, joint.direction, joint.side, joint.reach);
        for q in [p, p + d * r, p + s * r] {
            let x1 = ta.point(motion.point(q));
            let x2 = tb.point(q);
            let mut by: BTreeMap<usize, Vec3> = BTreeMap::new();
            if let Some(column) = column {
                work = work.saturating_add((path_a.len() + path_b.len()) as u64);
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
                            // The child turns by −sign ρ through the motion.
                            add(k, j.velocity(t, x) * (-sign * s));
                        }
                    }
                }
                add(*l, joint.velocity(&ta, x1));
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

const BROKEN: FoldError = FoldError::Broken {
    reason: "a joint or a body is missing, or a motion is not finite",
};

fn largest_gap(rows: &[Row]) -> f64 {
    rows.iter().map(|r| r.gap.length()).fold(0.0, f64::max)
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
    let mut budget = Budget(0);
    let solver = Solver {
        link,
        forest: &forest,
        loops: &loops,
        free: &free,
        column: &column,
        signs: &signs,
        goal,
        weight: size,
    };

    // Continuation from the flat sheet: a first step by the kinds, then a
    // tangent predictor and a corrector.
    let mut x = vec![0.0; free.len()];
    let mut exhausted = false;
    if largest > 0.0 && !free.is_empty() {
        // A guess solved at `base`: the solution when it closes within the
        // tolerance with every solved joint its kind's way.
        let attempt = |base: &[f64],
                       guess: Vec<f64>,
                       budget: &mut Budget|
         -> Result<Option<(Vec<f64>, bool)>, FoldError> {
            let (solved, ran_out) = match solver.levenberg(base, guess, budget) {
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
            Ok((largest_gap(&here) <= tol && kinds).then_some((solved, ran_out)))
        };
        let size_of = |v: &[f64]| v.iter().fold(0.0f64, |m, a| m.max(a.abs()));
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
                for f in FIRST_GUESSES {
                    let guess: Vec<f64> = signs
                        .iter()
                        .map(|s| s * (step * f).min(FIRST_CAP))
                        .collect();
                    let Some((solved, ran_out)) = attempt(&base, guess, &mut budget)? else {
                        continue;
                    };
                    let half_guess: Vec<f64> = solved.iter().map(|v| v / 2.0).collect();
                    let Some((half, _)) =
                        attempt(&given(target / 2.0), half_guess.clone(), &mut budget)?
                    else {
                        continue;
                    };
                    let off = half
                        .iter()
                        .zip(&half_guess)
                        .fold(0.0f64, |m, (a, b)| m.max((a - b).abs()));
                    if off <= 0.25 * size_of(&half_guess) + angular {
                        found = Some((solved, ran_out));
                        break;
                    }
                }
            } else {
                // Predict along the tangent: J_free dx = −J_given dg at the
                // last solution; correct from there. Taken when the
                // corrector lands within a quarter of the predicted motion.
                let now = given(done);
                let dg: Vec<f64> = base.iter().zip(&now).map(|(a, b)| a - b).collect();
                let dx = solver.tangent(&solver.with(&now, &x), &dg, &mut budget)?;
                let moved = size_of(&dx);
                let predicted: Vec<f64> = x.iter().zip(&dx).map(|(a, d)| a + d).collect();
                if let Some((solved, ran_out)) = attempt(&base, predicted.clone(), &mut budget)? {
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
                    if halvings > MAX_HALVINGS {
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
    let final_rows = rows(link, &forest, &loops, &rho, Some(&column), &mut budget)?;
    let gap = largest_gap(&final_rows);
    if gap > tol {
        if exhausted {
            return Err(FoldError::NotConverged { gap });
        }
        let worst = final_rows
            .chunks(3)
            .zip(&loops)
            .map(|(r, (k, ..))| (largest_gap(r), *k))
            .fold((0.0, 0usize), |a, b| if b.0 > a.0 { b } else { a });
        return Err(FoldError::NotClosing {
            id: source(worst.1),
            gap,
        });
    }
    // Determined: also at the flat sheet, where the branches meet and the
    // angles given do not say which way the others go.
    if !free.is_empty() {
        let loose = undetermined(&final_rows, free.len(), &mut budget)?;
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
        let wrapped = v.sin().atan2(v.cos());
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
    })
}

struct Solver<'a> {
    link: &'a Linkage,
    forest: &'a Forest,
    loops: &'a [Loop],
    free: &'a [usize],
    column: &'a [Option<usize>],
    signs: &'a [f64],
    goal: f64,
    weight: f64,
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
    /// from `J_free dx = −J_given dg` in least squares. Fails, naming the
    /// joints, where `J_free` is singular: a bifurcation, where the branch
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
        let loose = undetermined(&free_rows, n, budget)?;
        if !loose.is_empty() {
            let mut ids: Vec<u64> = loose
                .iter()
                .filter_map(|&c| self.free.get(c))
                .filter_map(|&k| self.link.joints.get(k).map(|j| j.id))
                .collect();
            ids.sort_unstable();
            ids.dedup();
            return Err(FoldError::Bifurcation { ids });
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
        solve_dense(&mut a, &mut b, n).ok_or(FoldError::Bifurcation {
            ids: self
                .free
                .iter()
                .filter_map(|&k| self.link.joints.get(k).map(|j| j.id))
                .collect(),
        })?;
        Ok(b)
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

    /// Levenberg–Marquardt from `x`; the solution and whether the
    /// iterations ran out first.
    fn levenberg(
        &self,
        base: &[f64],
        mut x: Vec<f64>,
        budget: &mut Budget,
    ) -> Result<(Vec<f64>, bool), FoldError> {
        let n = self.free.len();
        let cube = (n as u64).saturating_pow(3);
        let mut lambda = 1e-3;
        for _ in 0..MAX_ITERATIONS {
            let now = rows(
                self.link,
                self.forest,
                self.loops,
                &self.with(base, &x),
                Some(self.column),
                budget,
            )?;
            if largest_gap(&now) <= self.goal {
                return Ok((x, false));
            }
            let c0 = cost(&now) + self.penalty(&x);
            let (mut gram, mut grad) = normal(&now, n);
            let w2 = self.weight * self.weight;
            for (i, (v, s)) in x.iter().zip(self.signs).enumerate() {
                if s * v < 0.0 {
                    if let Some(d) = gram.get_mut(i * n + i) {
                        *d += w2;
                    }
                    if let Some(g) = grad.get_mut(i) {
                        *g += w2 * v;
                    }
                }
            }
            let mut moved = false;
            for _ in 0..12 {
                budget.spend(cube / 3 + 1)?;
                let mut a = gram.clone();
                for i in 0..n {
                    if let Some(d) = a.get_mut(i * n + i) {
                        *d += lambda * (*d + 1e-12);
                    }
                }
                let mut b: Vec<f64> = grad.iter().map(|g| -g).collect();
                if solve_dense(&mut a, &mut b, n).is_some() {
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
                return Ok((x, false));
            }
        }
        Ok((x, true))
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
fn undetermined(rows: &[Row], n: usize, budget: &mut Budget) -> Result<Vec<usize>, FoldError> {
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
    Ok(loose)
}
