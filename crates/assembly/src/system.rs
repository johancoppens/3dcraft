//! The equations: unknowns, residuals and their exact Jacobian.
//!
//! Unknowns are dimensionless. A free instance has six: a shift `u` of its
//! reference point in units of the problem's scale `L` and a rotation
//! vector `ω` (radians), applied as `t ← t + L u`, `R ← exp([ω]×) R`: the
//! rotation is parametrised around the current pose at every step, so
//! there is no singular orientation. A mate's variables follow: angles in
//! radians, shifts in units of `L`. Residuals are dimensionless the same
//! way: lengths over `L`, angles in radians.

use crate::ldl::{Budget, Ordering, Pattern};
use crate::types::{
    CouplingKind, Diagnosis, Frame, Group, Mat3, MateKind, MateSpec, Pose, Problem, VarKind,
};
use craft3d_math::Vec3;
use std::collections::BTreeSet;
use std::f64::consts::PI;

/// At most this many instances, mates, couplings and terms: a cap on what
/// a hostile problem can make the solver allocate.
pub(crate) const MAX_ITEMS: usize = 200_000;

/// The finest relative tolerance the residuals can resolve: coordinates
/// carry rounding of `EPSILON` times their size, and the residuals are
/// differences of sums of a few dozen such products.
pub(crate) const FINEST_RELATIVE: f64 = 1e4 * f64::EPSILON;

/// The largest angle (radians) a mate's angle or a driven angle may have:
/// some 160 000 turns, far past any motion, and small enough that a turn's
/// rounding stays below 1e-9 rad.
pub(crate) const MAX_ANGLE: f64 = 1e6;

/// A mate with its frames checked, made orthonormal and flipped.
#[derive(Clone, Debug)]
pub(crate) struct Mate {
    pub(crate) kind: MateKind,
    pub(crate) a: usize,
    pub(crate) b: usize,
    pub(crate) fa: Frame,
    pub(crate) fb: Frame,
    pub(crate) offset: f64,
    pub(crate) angle: f64,
}

/// One coupling as a linear equation `Σ cᵢ qᵢ = value` over mate
/// variables (model units and radians).
#[derive(Clone, Debug)]
pub(crate) struct Linear {
    pub(crate) terms: Vec<(usize, usize, f64)>,
    pub(crate) value: f64,
    /// Divides the residual so the largest dimensionless coefficient is 1.
    pub(crate) norm: f64,
}

/// What a group of equations is.
#[derive(Clone, Debug)]
pub(crate) enum Rows {
    Mate(usize),
    Coupling(usize),
    /// `q − value`; the value lives in the state (drives) or the limit.
    Driver {
        mate: usize,
        var: usize,
        slot: usize,
    },
    Limit {
        mate: usize,
        var: usize,
    },
}

#[derive(Clone, Debug)]
pub(crate) struct GroupInfo {
    pub(crate) group: Group,
    pub(crate) rows: Rows,
    pub(crate) size: usize,
    /// The unknown blocks the group touches, sorted.
    pub(crate) blocks: Vec<usize>,
}

/// The positions of everything, in model units and radians.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct State {
    pub(crate) poses: Vec<Pose>,
    pub(crate) values: Vec<Vec<f64>>,
}

/// One group's residuals and Jacobian blocks at a state.
#[derive(Clone, Debug)]
pub(crate) struct Eval {
    pub(crate) r: Vec<f64>,
    /// `(block, rows × size(block) row-major)`, blocks sorted, distinct.
    pub(crate) parts: Vec<(usize, Vec<f64>)>,
}

#[derive(Debug)]
pub(crate) struct System {
    pub(crate) problem: Problem,
    pub(crate) mates: Vec<Mate>,
    pub(crate) linears: Vec<Linear>,
    pub(crate) grounded: Vec<bool>,
    /// The scale `L` and the relative tolerance `ε = tolerance / L`.
    pub(crate) scale: f64,
    pub(crate) eps: f64,
    /// Per instance, its unknown block (`None` when grounded).
    pub(crate) body_block: Vec<Option<usize>>,
    /// Per mate, its variables' block (`None` without variables).
    pub(crate) var_block: Vec<Option<usize>>,
    /// Per block: what it is.
    pub(crate) block_owner: Vec<BlockOwner>,
    pub(crate) block_sizes: Vec<usize>,
    pub(crate) block_offsets: Vec<usize>,
    pub(crate) unknowns: usize,
    pub(crate) groups: Vec<GroupInfo>,
    /// The driven slots `(mate, var)` in order.
    pub(crate) slots: Vec<(usize, usize)>,
    /// `JᵀJ` over the unknown blocks, minimum degree.
    pub(crate) normal: Pattern,
    /// `J Jᵀ` over the groups, in their order.
    pub(crate) gram: Pattern,
    /// Per unknown block, the groups that touch it, in order.
    pub(crate) by_block: Vec<Vec<usize>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BlockOwner {
    Body(usize),
    Vars(usize),
}

fn invalid(reason: impl Into<String>) -> Diagnosis {
    Diagnosis::Invalid {
        reason: reason.into(),
    }
}

/// The arithmetic the symbolic analysis may spend.
pub(crate) const SYMBOLIC_BUDGET: u64 = 4_000_000_000;

/// The index of a mate kind's angle variable.
pub(crate) fn angle_var(kind: MateKind) -> Option<usize> {
    kind.variables().iter().position(|v| *v == VarKind::Angle)
}

/// The index of a mate kind's distance along z.
pub(crate) fn distance_var(kind: MateKind) -> Option<usize> {
    kind.variables()
        .iter()
        .position(|v| *v == VarKind::Distance)
}

impl System {
    pub(crate) fn new(p: &Problem) -> Result<System, Diagnosis> {
        let n = p.instances;
        if n == 0 {
            return Err(invalid("the assembly has no instances"));
        }
        if n > MAX_ITEMS
            || p.mates.len() > MAX_ITEMS
            || p.couplings.len() > MAX_ITEMS
            || p.grounded.len() > MAX_ITEMS
            || p.names.len() > MAX_ITEMS
        {
            return Err(Diagnosis::TooLarge {
                reason: format!("more than {MAX_ITEMS} instances, mates or couplings"),
            });
        }
        if p.start.len() != n {
            return Err(invalid(format!(
                "{} start poses for {n} instances",
                p.start.len()
            )));
        }
        if !(p.scale.is_finite() && p.scale > 0.0) {
            return Err(invalid("the scale must be positive and finite"));
        }
        if !(p.tolerance.is_finite() && p.tolerance > 0.0) {
            return Err(invalid("the tolerance must be positive and finite"));
        }
        let eps = p.tolerance / p.scale;
        if !(eps < 1.0) {
            return Err(invalid("the tolerance must be smaller than the scale"));
        }
        if eps < FINEST_RELATIVE {
            return Err(invalid(format!(
                "a tolerance of {} at a scale of {} is finer than double precision resolves",
                p.tolerance, p.scale
            )));
        }
        // A coordinate whose rounding exceeds the tolerance cannot meet it.
        let reach = p.tolerance / FINEST_RELATIVE;
        let place_ok = |v: Vec3| v.is_finite() && v.length() <= reach;
        let mut grounded = vec![false; n];
        for &g in &p.grounded {
            *grounded
                .get_mut(g)
                .ok_or_else(|| invalid(format!("grounded instance {g} does not exist")))? = true;
        }
        let mut problem = p.clone();
        for (i, pose) in problem.start.iter_mut().enumerate() {
            let r = pose.rotation.orthonormalized(eps).ok_or_else(|| {
                invalid(format!(
                    "the start rotation of {} is not a rotation",
                    p.instance_name(i)
                ))
            })?;
            if !place_ok(pose.translation) {
                return Err(invalid(format!(
                    "the start position of {} is not finite or too far out for the tolerance",
                    p.instance_name(i)
                )));
            }
            pose.rotation = r;
        }
        let mut mates = Vec::with_capacity(p.mates.len());
        for (m, spec) in p.mates.iter().enumerate() {
            mates.push(check_mate(p, m, spec, eps, &place_ok)?);
        }
        let mut linears = Vec::with_capacity(p.couplings.len());
        for (c, spec) in p.couplings.iter().enumerate() {
            linears.push(check_coupling(p, c, &spec.kind, spec.mates, spec.value)?);
        }
        // Blocks: free bodies, then each mate's variables.
        let mut body_block = vec![None; n];
        let mut block_owner = Vec::new();
        let mut block_sizes = Vec::new();
        for (i, g) in grounded.iter().enumerate() {
            if !g {
                if let Some(slot) = body_block.get_mut(i) {
                    *slot = Some(block_owner.len());
                }
                block_owner.push(BlockOwner::Body(i));
                block_sizes.push(6);
            }
        }
        let mut var_block = vec![None; mates.len()];
        for (m, mate) in mates.iter().enumerate() {
            let k = mate.kind.variables().len();
            if k > 0 {
                if let Some(slot) = var_block.get_mut(m) {
                    *slot = Some(block_owner.len());
                }
                block_owner.push(BlockOwner::Vars(m));
                block_sizes.push(k);
            }
        }
        let mut block_offsets = Vec::with_capacity(block_sizes.len());
        let mut unknowns = 0usize;
        for &s in &block_sizes {
            block_offsets.push(unknowns);
            unknowns += s;
        }
        // Groups: mates, couplings, drivers, limits.
        let mut groups = Vec::new();
        for (m, mate) in mates.iter().enumerate() {
            let mut blocks: Vec<usize> = [
                body_block.get(mate.a).copied().flatten(),
                body_block.get(mate.b).copied().flatten(),
                var_block.get(m).copied().flatten(),
            ]
            .into_iter()
            .flatten()
            .collect();
            blocks.sort_unstable();
            groups.push(GroupInfo {
                group: Group::Mate(m),
                rows: Rows::Mate(m),
                size: if mate.kind.holds_rotation() { 6 } else { 3 },
                blocks,
            });
        }
        for (c, lin) in linears.iter().enumerate() {
            let mut blocks: Vec<usize> = lin
                .terms
                .iter()
                .filter_map(|&(m, _, _)| var_block.get(m).copied().flatten())
                .collect();
            blocks.sort_unstable();
            blocks.dedup();
            groups.push(GroupInfo {
                group: Group::Coupling(c),
                rows: Rows::Coupling(c),
                size: 1,
                blocks,
            });
        }
        let slots = p.driven_slots();
        for (slot, &(m, v)) in slots.iter().enumerate() {
            groups.push(GroupInfo {
                group: Group::Driver {
                    mate: m,
                    variable: v,
                },
                rows: Rows::Driver {
                    mate: m,
                    var: v,
                    slot,
                },
                size: 1,
                blocks: var_block.get(m).copied().flatten().into_iter().collect(),
            });
        }
        for (m, spec) in p.mates.iter().enumerate() {
            for (v, lim) in spec.limits.iter().enumerate() {
                if lim.is_some() {
                    groups.push(GroupInfo {
                        group: Group::Limit {
                            mate: m,
                            variable: v,
                        },
                        rows: Rows::Limit { mate: m, var: v },
                        size: 1,
                        blocks: var_block.get(m).copied().flatten().into_iter().collect(),
                    });
                }
            }
        }
        let mut budget = Budget::new(SYMBOLIC_BUDGET);
        let too_large = |reason: String| Diagnosis::TooLarge { reason };
        let mut edges = BTreeSet::new();
        for g in &groups {
            for (i, &x) in g.blocks.iter().enumerate() {
                for &y in g.blocks.iter().skip(i + 1) {
                    edges.insert((x, y));
                }
            }
        }
        let normal = Pattern::new(&block_sizes, &edges, Ordering::MinimumDegree, &mut budget)
            .map_err(too_large)?;
        // Groups sharing an unknown block are coupled in J Jᵀ.
        let mut by_block: Vec<Vec<usize>> = vec![Vec::new(); block_sizes.len()];
        for (gi, g) in groups.iter().enumerate() {
            for &b in &g.blocks {
                if let Some(list) = by_block.get_mut(b) {
                    list.push(gi);
                }
            }
        }
        let mut gedges = BTreeSet::new();
        for list in &by_block {
            budget
                .spend(list.len().saturating_mul(list.len()))
                .map_err(too_large)?;
            for (i, &x) in list.iter().enumerate() {
                for &y in list.iter().skip(i + 1) {
                    gedges.insert((x, y));
                }
            }
        }
        let gsizes: Vec<usize> = groups.iter().map(|g| g.size).collect();
        let gram =
            Pattern::new(&gsizes, &gedges, Ordering::Natural, &mut budget).map_err(too_large)?;
        Ok(System {
            problem,
            mates,
            linears,
            grounded,
            scale: p.scale,
            eps,
            body_block,
            var_block,
            block_owner,
            block_sizes,
            block_offsets,
            unknowns,
            groups,
            slots,
            normal,
            gram,
            by_block,
        })
    }

    /// The state the problem starts from: its poses, and each mate's
    /// variables measured there (a driven angle taken the full turns
    /// nearest its driven value).
    pub(crate) fn start_state(&self, drives: &[f64]) -> State {
        let poses = self.problem.start.clone();
        let mut values = Vec::with_capacity(self.mates.len());
        for (m, mate) in self.mates.iter().enumerate() {
            let mut vals = self.measure(mate, &poses);
            for (v, val) in vals.iter_mut().enumerate() {
                let driven = self
                    .slots
                    .iter()
                    .position(|&s| s == (m, v))
                    .and_then(|k| drives.get(k).copied());
                if let (Some(target), Some(VarKind::Angle)) =
                    (driven, mate.kind.variables().get(v).copied())
                {
                    *val += (2.0 * PI) * ((target - *val) / (2.0 * PI)).round();
                }
            }
            values.push(vals);
        }
        State { poses, values }
    }

    /// A mate's variables measured from where its frames are.
    fn measure(&self, mate: &Mate, poses: &[Pose]) -> Vec<f64> {
        let (Some(pa), Some(pb)) = (poses.get(mate.a), poses.get(mate.b)) else {
            return vec![0.0; mate.kind.variables().len()];
        };
        let wa = pa.frame(&mate.fa);
        let wb = pb.frame(&mate.fb);
        let local = wa.axes.apply_transposed(wb.origin - wa.origin);
        let gx = wb.axes.x();
        let phi = wa.axes.y().dot(gx).atan2(wa.axes.x().dot(gx));
        let theta = wrap(phi - mate.angle);
        mate.kind
            .variables()
            .iter()
            .map(|v| match v {
                VarKind::Angle => theta,
                VarKind::Distance => local.z - mate.offset,
                VarKind::PlaneX => local.x,
                VarKind::PlaneY => local.y,
            })
            .collect()
    }

    /// The residuals and Jacobian of group `gi`, its rows multiplied by
    /// `weight`. `drives` are the driven values in slot order.
    pub(crate) fn eval(&self, gi: usize, s: &State, drives: &[f64], weight: f64) -> Option<Eval> {
        let g = self.groups.get(gi)?;
        let mut e = match g.rows {
            Rows::Mate(m) => self.eval_mate(m, s)?,
            Rows::Coupling(c) => self.eval_linear(c, s)?,
            Rows::Driver { mate, var, slot } => {
                self.eval_value(mate, var, *drives.get(slot)?, s)?
            }
            Rows::Limit { mate, var } => {
                let (lo, hi) = self
                    .problem
                    .mates
                    .get(mate)?
                    .limits
                    .get(var)
                    .copied()
                    .flatten()?;
                let q = *s.values.get(mate)?.get(var)?;
                let bound = if (q - lo).abs() <= (q - hi).abs() {
                    lo
                } else {
                    hi
                };
                self.eval_value(mate, var, bound, s)?
            }
        };
        if weight != 1.0 {
            for r in &mut e.r {
                *r *= weight;
            }
            for (_, j) in &mut e.parts {
                for x in j {
                    *x *= weight;
                }
            }
        }
        Some(e)
    }

    /// `q − value`, dimensionless.
    fn eval_value(&self, mate: usize, var: usize, value: f64, s: &State) -> Option<Eval> {
        let kind = self.mates.get(mate)?.kind;
        let q = *s.values.get(mate)?.get(var)?;
        let unit = if kind.variables().get(var)?.is_angle() {
            1.0
        } else {
            self.scale
        };
        let block = (*self.var_block.get(mate)?)?;
        let size = *self.block_sizes.get(block)?;
        let mut j = vec![0.0; size];
        *j.get_mut(var)? = 1.0;
        Some(Eval {
            r: vec![(q - value) / unit],
            parts: vec![(block, j)],
        })
    }

    fn eval_linear(&self, c: usize, s: &State) -> Option<Eval> {
        let lin = self.linears.get(c)?;
        let mut sum = -lin.value;
        let mut parts: Vec<(usize, Vec<f64>)> = Vec::new();
        for &(m, v, coef) in &lin.terms {
            let kind = self.mates.get(m)?.kind;
            sum += coef * *s.values.get(m)?.get(v)?;
            let unit = if kind.variables().get(v)?.is_angle() {
                1.0
            } else {
                self.scale
            };
            let block = (*self.var_block.get(m)?)?;
            let size = *self.block_sizes.get(block)?;
            let k = match parts.iter().position(|(b, _)| *b == block) {
                Some(k) => k,
                None => {
                    parts.push((block, vec![0.0; size]));
                    parts.len() - 1
                }
            };
            *parts.get_mut(k)?.1.get_mut(v)? += coef * unit / lin.norm;
        }
        parts.sort_by_key(|(b, _)| *b);
        Some(Eval {
            r: vec![sum / lin.norm],
            parts,
        })
    }

    fn eval_mate(&self, m: usize, s: &State) -> Option<Eval> {
        let mate = self.mates.get(m)?;
        let pa = s.poses.get(mate.a)?;
        let pb = s.poses.get(mate.b)?;
        let vals = s.values.get(m)?;
        let l = self.scale;
        let mut v = Vec3::new(0.0, 0.0, mate.offset);
        let mut phi = mate.angle;
        for (k, kind) in mate.kind.variables().iter().enumerate() {
            let q = *vals.get(k)?;
            match kind {
                VarKind::Angle => phi += q,
                VarKind::Distance => v.z += q,
                VarKind::PlaneX => v.x += q,
                VarKind::PlaneY => v.y += q,
            }
        }
        let aw = pa.rotation.mul(&mate.fa.axes);
        let oa = pa.point(mate.fa.origin);
        let p_a = oa + aw.apply(v);
        let e = aw.mul(&Mat3::rotation(Vec3::Z, phi));
        let p_b = pb.point(mate.fb.origin);
        let gm = pb.rotation.mul(&mate.fb.axes);
        let rows = if mate.kind.holds_rotation() { 6 } else { 3 };
        let mut r = vec![0.0; rows];
        let dp = (p_b - p_a) * (1.0 / l);
        r.get_mut(0..3)?.copy_from_slice(&[dp.x, dp.y, dp.z]);
        // The turn from e to g, as a rotation vector: zero only when the
        // axes agree (a half turn is π, not a false zero).
        let rel = gm.mul(&e.transpose());
        let phi_rot = log_so3(&rel);
        if rows == 6 {
            r.get_mut(3..6)?
                .copy_from_slice(&[phi_rot.x, phi_rot.y, phi_rot.z]);
        }
        // Left perturbations: g ← exp(ω_b) g gives J_l⁻¹ ω_b; e ← exp(ω_a) e
        // gives −J_l⁻¹ R ω_a.
        let jinv = jl_inv(phi_rot);
        let rot_b = jinv;
        let rot_a = mul33(&jinv, &to_rows(&rel));
        let mut parts = Vec::with_capacity(3);
        let body = |sign: f64, arm: Vec3, rotm: &[[f64; 3]; 3]| -> Vec<f64> {
            // Columns: u (3), ω (3). pos/u = sign I; pos/ω = −sign [arm]×.
            let mut j = vec![0.0; rows * 6];
            let skew = skew(arm * (1.0 / l));
            for row in 0..3 {
                if let Some(x) = j.get_mut(row * 6 + row) {
                    *x = sign;
                }
                for col in 0..3 {
                    if let Some(x) = j.get_mut(row * 6 + 3 + col) {
                        *x = -sign * at(&skew, row, col);
                    }
                }
            }
            if rows == 6 {
                for row in 0..3 {
                    for col in 0..3 {
                        if let Some(x) = j.get_mut((3 + row) * 6 + 3 + col) {
                            *x = at(rotm, row, col);
                        }
                    }
                }
            }
            j
        };
        if let Some(Some(ba)) = self.body_block.get(mate.a) {
            let neg_rot_a = rot_a.map(|r| r.map(|x| -x));
            parts.push((*ba, body(-1.0, p_a - pa.translation, &neg_rot_a)));
        }
        if let Some(Some(bb)) = self.body_block.get(mate.b) {
            parts.push((*bb, body(1.0, p_b - pb.translation, &rot_b)));
        }
        if let Some(Some(bv)) = self.var_block.get(m) {
            let k = mate.kind.variables().len();
            let mut j = vec![0.0; rows * k];
            let n_axis = aw.z();
            for (col, kind) in mate.kind.variables().iter().enumerate() {
                match kind {
                    VarKind::Angle => {
                        if rows == 6 {
                            // −J_l⁻¹ R n.
                            for row in 0..3 {
                                let mut x = 0.0;
                                for t in 0..3 {
                                    x -= at(&rot_a, row, t) * comp(n_axis, t);
                                }
                                *j.get_mut((3 + row) * k + col)? = x;
                            }
                        }
                    }
                    VarKind::Distance | VarKind::PlaneX | VarKind::PlaneY => {
                        let axis = match kind {
                            VarKind::PlaneX => aw.x(),
                            VarKind::PlaneY => aw.y(),
                            _ => aw.z(),
                        };
                        for row in 0..3 {
                            *j.get_mut(row * k + col)? = -comp(axis, row);
                        }
                    }
                }
            }
            parts.push((*bv, j));
        }
        parts.sort_by_key(|(b, _)| *b);
        Some(Eval { r, parts })
    }

    /// Eigenvalues of `JᵀJ` (and `J Jᵀ`) at or below this count as zero:
    /// a direction with singular value `σ ≤ √ε` (`ε = tolerance / scale`)
    /// can move by `√ε` of the scale while its residual stays within the
    /// tolerance to second order, so it is free as far as the tolerance can
    /// tell.
    pub(crate) fn rank_threshold(&self) -> f64 {
        self.eps
    }

    /// The unit a mate variable is measured in: 1 for an angle, the scale
    /// for a distance.
    pub(crate) fn unit(&self, mate: usize, var: usize) -> f64 {
        match self
            .mates
            .get(mate)
            .and_then(|m| m.kind.variables().get(var))
        {
            Some(VarKind::Angle) => 1.0,
            _ => self.scale,
        }
    }

    /// The state moved by the dimensionless step `h` (block order).
    pub(crate) fn step(&self, s: &State, h: &[f64]) -> Option<State> {
        let mut out = s.clone();
        for (b, owner) in self.block_owner.iter().enumerate() {
            let o = *self.block_offsets.get(b)?;
            match *owner {
                BlockOwner::Body(i) => {
                    let x = h.get(o..o + 6)?;
                    let u = Vec3::new(*x.first()?, *x.get(1)?, *x.get(2)?);
                    let w = Vec3::new(*x.get(3)?, *x.get(4)?, *x.get(5)?);
                    let pose = out.poses.get_mut(i)?;
                    pose.translation += u * self.scale;
                    let r = Mat3::exp(w).mul(&pose.rotation);
                    pose.rotation = r.orthonormalized(0.5).unwrap_or(r);
                }
                BlockOwner::Vars(m) => {
                    let kinds = self.mates.get(m)?.kind.variables();
                    let vals = out.values.get_mut(m)?;
                    for (k, kind) in kinds.iter().enumerate() {
                        let unit = if kind.is_angle() { 1.0 } else { self.scale };
                        *vals.get_mut(k)? += *h.get(o + k)? * unit;
                    }
                }
            }
        }
        Some(out)
    }

    /// How far apart two states are, dimensionless, largest over the
    /// unknowns (rotations by their angle).
    pub(crate) fn distance(&self, a: &State, b: &State) -> f64 {
        let mut d = 0.0f64;
        for owner in &self.block_owner {
            match *owner {
                BlockOwner::Body(i) => {
                    let (Some(pa), Some(pb)) = (a.poses.get(i), b.poses.get(i)) else {
                        continue;
                    };
                    let t = (pb.translation - pa.translation) * (1.0 / self.scale);
                    d = d.max(t.x.abs()).max(t.y.abs()).max(t.z.abs());
                    d = d.max(rotation_angle(&pb.rotation.mul(&pa.rotation.transpose())));
                }
                BlockOwner::Vars(m) => {
                    let (Some(va), Some(vb), Some(mate)) =
                        (a.values.get(m), b.values.get(m), self.mates.get(m))
                    else {
                        continue;
                    };
                    for ((x, y), kind) in va.iter().zip(vb).zip(mate.kind.variables()) {
                        let unit = if kind.is_angle() { 1.0 } else { self.scale };
                        d = d.max(((y - x) / unit).abs());
                    }
                }
            }
        }
        d
    }
}

/// The angle of a rotation matrix, in [0, π].
pub(crate) fn rotation_angle(r: &Mat3) -> f64 {
    let tr = r.x().x + r.y().y + r.z().z;
    let s = Vec3::new(r.y().z - r.z().y, r.z().x - r.x().z, r.x().y - r.y().x).length() * 0.5;
    s.atan2((tr - 1.0) * 0.5)
}

/// An angle wrapped into (−π, π].
pub(crate) fn wrap(a: f64) -> f64 {
    let w = a - (2.0 * PI) * (a / (2.0 * PI)).round();
    if w <= -PI { w + 2.0 * PI } else { w }
}

pub(crate) fn comp(v: Vec3, i: usize) -> f64 {
    match i {
        0 => v.x,
        1 => v.y,
        _ => v.z,
    }
}

/// Entry `(r, c)` of a 3×3 array, 0 outside.
fn at(m: &[[f64; 3]; 3], r: usize, c: usize) -> f64 {
    m.get(r).and_then(|row| row.get(c)).copied().unwrap_or(0.0)
}

/// A matrix as rows: `out[r][c]` is entry `(r, c)`.
fn to_rows(m: &Mat3) -> [[f64; 3]; 3] {
    let mut out = [[0.0; 3]; 3];
    for (r, row) in out.iter_mut().enumerate() {
        for (c, cell) in row.iter_mut().enumerate() {
            *cell = m.cols.get(c).map_or(0.0, |v| comp(*v, r));
        }
    }
    out
}

fn mul33(a: &[[f64; 3]; 3], b: &[[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let mut out = [[0.0; 3]; 3];
    for (r, row) in out.iter_mut().enumerate() {
        for (c, cell) in row.iter_mut().enumerate() {
            *cell = (0..3).map(|k| at(a, r, k) * at(b, k, c)).sum();
        }
    }
    out
}

/// The rotation vector of a rotation matrix (the inverse of
/// [`Mat3::exp`]), its angle in [0, π]. Near a half turn the axis comes
/// from the symmetric part, where the skew part has lost it.
pub(crate) fn log_so3(r: &Mat3) -> Vec3 {
    let axial = Vec3::new(r.y().z - r.z().y, r.z().x - r.x().z, r.x().y - r.y().x) * 0.5;
    let cos = ((r.x().x + r.y().y + r.z().z - 1.0) * 0.5).clamp(-1.0, 1.0);
    let sin = axial.length();
    let theta = sin.atan2(cos);
    if !theta.is_finite() {
        return Vec3::ZERO;
    }
    if cos > -0.99 {
        // θ / sin θ is well conditioned here (1 at θ = 0).
        if sin < 1e-300 {
            return axial;
        }
        return axial * (theta / sin);
    }
    // (R + Rᵀ)/2 − cos θ I = (1 − cos θ) k kᵀ: the column with the largest
    // diagonal is the axis, up to sign; the skew part gives the sign.
    let rows = to_rows(r);
    let s = |i: usize, j: usize| {
        0.5 * (at(&rows, i, j) + at(&rows, j, i)) - if i == j { cos } else { 0.0 }
    };
    let mut best = 0;
    for i in 1..3 {
        if s(i, i) > s(best, best) {
            best = i;
        }
    }
    let col = Vec3::new(s(0, best), s(1, best), s(2, best));
    let Some(mut k) = col.normalized() else {
        return Vec3::ZERO;
    };
    if k.dot(axial) < 0.0 {
        k = -k;
    }
    k * theta
}

/// `J_l⁻¹(φ)`, the inverse left Jacobian of SO(3):
/// `log(exp(δ) exp(φ)) ≈ φ + J_l⁻¹(φ) δ`.
pub(crate) fn jl_inv(phi: Vec3) -> [[f64; 3]; 3] {
    let theta = phi.length();
    let k = skew(phi);
    let k2 = mul33(&k, &k);
    // 1/θ² − cot(θ/2)/(2θ), by its series near 0.
    let coef = if theta < 1e-4 {
        1.0 / 12.0 + theta * theta / 720.0
    } else {
        let half = 0.5 * theta;
        1.0 / (theta * theta) - half.cos() / (half.sin() * 2.0 * theta)
    };
    let mut out = [[0.0; 3]; 3];
    for (r, row) in out.iter_mut().enumerate() {
        for (c, cell) in row.iter_mut().enumerate() {
            let id = if r == c { 1.0 } else { 0.0 };
            *cell = id - 0.5 * at(&k, r, c) + coef * at(&k2, r, c);
        }
    }
    out
}

/// `[w]×`: `[w]× v = w × v`.
fn skew(w: Vec3) -> [[f64; 3]; 3] {
    [[0.0, -w.z, w.y], [w.z, 0.0, -w.x], [-w.y, w.x, 0.0]]
}

fn check_frame(f: &Frame, eps: f64, place_ok: &dyn Fn(Vec3) -> bool) -> Option<Frame> {
    if !place_ok(f.origin) {
        return None;
    }
    Some(Frame {
        origin: f.origin,
        axes: f.axes.orthonormalized(eps)?,
    })
}

fn check_mate(
    p: &Problem,
    m: usize,
    spec: &MateSpec,
    eps: f64,
    place_ok: &dyn Fn(Vec3) -> bool,
) -> Result<Mate, Diagnosis> {
    let name = p.mate_name(m);
    let (a, b) = (spec.a.0, spec.b.0);
    if a >= p.instances || b >= p.instances {
        return Err(invalid(format!(
            "{name} names an instance that does not exist"
        )));
    }
    if a == b {
        return Err(invalid(format!(
            "{name} mates {} with itself",
            p.instance_name(a)
        )));
    }
    let fa = check_frame(&spec.a.1, eps, place_ok).ok_or_else(|| {
        invalid(format!(
            "{name}: the frame on {} is degenerate or not finite",
            p.instance_name(a)
        ))
    })?;
    let mut fb = check_frame(&spec.b.1, eps, place_ok).ok_or_else(|| {
        invalid(format!(
            "{name}: the frame on {} is degenerate or not finite",
            p.instance_name(b)
        ))
    })?;
    if spec.flip {
        fb.axes = Mat3::from_cols(fb.axes.x(), -fb.axes.y(), -fb.axes.z());
    }
    if !(spec.offset.is_finite() && spec.offset.abs() <= p.tolerance / FINEST_RELATIVE) {
        return Err(invalid(format!(
            "{name}: the offset is not finite or too large"
        )));
    }
    if !spec.angle.is_finite() || spec.angle.abs() > MAX_ANGLE {
        return Err(invalid(format!(
            "{name}: the angle is not finite or too large"
        )));
    }
    let nvars = spec.kind.variables().len();
    for (v, lim) in spec.limits.iter().enumerate() {
        if let Some((lo, hi)) = lim {
            if v >= nvars {
                return Err(invalid(format!(
                    "{name}: a {} mate has no variable {v} to limit",
                    spec.kind.label()
                )));
            }
            if !(lo.is_finite() && hi.is_finite() && lo <= hi) {
                return Err(invalid(format!(
                    "{name}: limits must be finite with low ≤ high"
                )));
            }
        }
    }
    for (v, d) in spec.driven.iter().enumerate() {
        if let Some(d) = d {
            if v >= nvars {
                return Err(invalid(format!(
                    "{name}: a {} mate has no variable {v} to drive",
                    spec.kind.label()
                )));
            }
            let cap = if spec.kind.variables().get(v).is_some_and(|k| k.is_angle()) {
                MAX_ANGLE
            } else {
                p.tolerance / FINEST_RELATIVE
            };
            if !d.is_finite() || d.abs() > cap {
                return Err(invalid(format!(
                    "{name}: the driven value is not finite or too large"
                )));
            }
        }
    }
    Ok(Mate {
        kind: spec.kind,
        a,
        b,
        fa,
        fb,
        offset: spec.offset,
        angle: spec.angle,
    })
}

fn check_coupling(
    p: &Problem,
    c: usize,
    kind: &CouplingKind,
    mates: (usize, usize),
    value: f64,
) -> Result<Linear, Diagnosis> {
    let what = format!("coupling {c}");
    if !value.is_finite() {
        return Err(invalid(format!("{what}: its value is not finite")));
    }
    let kind_of = |m: usize| -> Result<MateKind, Diagnosis> {
        p.mates
            .get(m)
            .map(|s| s.kind)
            .ok_or_else(|| invalid(format!("{what} names mate {m}, which does not exist")))
    };
    let need = |m: usize, var: Option<usize>, what_var: &str| -> Result<usize, Diagnosis> {
        var.ok_or_else(|| {
            invalid(format!(
                "{what}: {} has no {what_var} to couple",
                p.mate_name(m)
            ))
        })
    };
    let (a, b) = mates;
    let terms = match kind {
        CouplingKind::Gear => {
            let va = need(a, angle_var(kind_of(a)?), "angle")?;
            let vb = need(b, angle_var(kind_of(b)?), "angle")?;
            if a == b {
                return Err(invalid(format!(
                    "{what}: a gear couples two different mates"
                )));
            }
            vec![(a, va, 1.0), (b, vb, value)]
        }
        CouplingKind::Rack | CouplingKind::Screw => {
            let va = need(a, distance_var(kind_of(a)?), "distance along its axis")?;
            let vb = need(b, angle_var(kind_of(b)?), "angle")?;
            let k = if matches!(kind, CouplingKind::Screw) {
                value / (2.0 * PI)
            } else {
                value
            };
            vec![(a, va, 1.0), (b, vb, -k)]
        }
        CouplingKind::Linear(terms) => {
            if terms.is_empty() || terms.len() > MAX_ITEMS {
                return Err(invalid(format!(
                    "{what}: a linear coupling needs 1 to {MAX_ITEMS} terms"
                )));
            }
            for &(m, v, coef) in terms {
                let k = kind_of(m)?;
                if v >= k.variables().len() {
                    return Err(invalid(format!(
                        "{what}: {} has no variable {v}",
                        p.mate_name(m)
                    )));
                }
                if !coef.is_finite() {
                    return Err(invalid(format!("{what}: a coefficient is not finite")));
                }
            }
            terms.clone()
        }
    };
    let rhs = if matches!(kind, CouplingKind::Linear(_)) {
        value
    } else {
        0.0
    };
    let mut norm = 0.0f64;
    for &(m, v, coef) in &terms {
        let unit = match p.mates.get(m).and_then(|s| s.kind.variables().get(v)) {
            Some(VarKind::Angle) => 1.0,
            _ => p.scale,
        };
        norm = norm.max((coef * unit).abs());
    }
    if !(norm.is_finite() && norm > 0.0) {
        return Err(invalid(format!("{what}: every coefficient is zero")));
    }
    Ok(Linear {
        terms,
        value: rhs,
        norm,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CouplingSpec, MAX_MATE_VARIABLES};

    /// A small deterministic generator.
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> f64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((self.0 >> 11) as f64) / ((1u64 << 53) as f64) * 2.0 - 1.0
        }
        fn vec(&mut self) -> Vec3 {
            Vec3::new(self.next(), self.next(), self.next())
        }
        fn frame(&mut self, scale: f64) -> Frame {
            Frame::new(self.vec() * scale, Mat3::exp(self.vec() * 2.0))
        }
    }

    fn problem(scale: f64, seed: u64) -> Problem {
        let mut g = Lcg(seed);
        let mut p = Problem::new(3, 1e-9 * scale, scale);
        p.grounded = vec![0];
        let kinds = [
            MateKind::Fixed,
            MateKind::Revolute,
            MateKind::Slider,
            MateKind::Cylindrical,
            MateKind::Planar,
            MateKind::Ball,
        ];
        for (k, kind) in kinds.iter().enumerate() {
            let (a, b) = [(0, 1), (1, 2), (2, 0)][k % 3];
            let mut m = MateSpec::new(*kind, (a, g.frame(scale)), (b, g.frame(scale)));
            m.offset = g.next() * scale;
            m.angle = g.next();
            m.flip = k % 2 == 1;
            p.mates.push(m);
        }
        p.couplings.push(CouplingSpec {
            kind: CouplingKind::Screw,
            mates: (3, 3),
            value: 0.3 * scale,
        });
        p.couplings.push(CouplingSpec {
            kind: CouplingKind::Gear,
            mates: (1, 3),
            value: 1.7,
        });
        for (i, pose) in p.start.iter_mut().enumerate() {
            if i > 0 {
                *pose = Pose::new(Mat3::exp(g.vec() * 2.0), g.vec() * scale);
            }
        }
        let _ = MAX_MATE_VARIABLES;
        p
    }

    #[test]
    fn the_jacobian_matches_finite_differences() {
        for (seed, scale) in [(1u64, 1e-3), (7, 1.0), (42, 1e6)] {
            let p = problem(scale, seed);
            let sys = System::new(&p).unwrap();
            let mut g = Lcg(seed + 100);
            let mut s = sys.start_state(&[]);
            for vals in &mut s.values {
                for v in vals.iter_mut() {
                    *v += g.next() * 0.5 * scale.min(1.0);
                }
            }
            let h = 1e-6;
            for gi in 0..sys.groups.len() {
                let e = sys.eval(gi, &s, &[], 1.0).unwrap();
                for (b, part) in &e.parts {
                    let size = sys.block_sizes[*b];
                    for c in 0..size {
                        let mut dx = vec![0.0; sys.unknowns];
                        dx[sys.block_offsets[*b] + c] = h;
                        let plus = sys.eval(gi, &sys.step(&s, &dx).unwrap(), &[], 1.0).unwrap();
                        dx[sys.block_offsets[*b] + c] = -h;
                        let minus = sys.eval(gi, &sys.step(&s, &dx).unwrap(), &[], 1.0).unwrap();
                        for row in 0..e.r.len() {
                            let fd = (plus.r[row] - minus.r[row]) / (2.0 * h);
                            let an = part[row * size + c];
                            assert!(
                                (fd - an).abs() < 1e-5 * (1.0 + an.abs()),
                                "group {gi} block {b} col {c} row {row}: fd {fd} analytic {an} (scale {scale})"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn the_log_map_inverts_exp_up_to_a_half_turn() {
        let mut g = Lcg(3);
        for k in 0..200 {
            let axis = g.vec().normalized().unwrap();
            let angle = std::f64::consts::PI * (k as f64 / 199.0);
            let w = axis * angle;
            let back = log_so3(&Mat3::exp(w));
            if angle < std::f64::consts::PI - 1e-9 {
                assert!((back - w).length() < 1e-9, "{angle}: {back:?} vs {w:?}");
            } else {
                assert!((back.length() - angle).abs() < 1e-9);
            }
        }
        // A half turn is π, not a false zero.
        let half = Mat3::from_cols(Vec3::X, -Vec3::Y, -Vec3::Z);
        assert!((log_so3(&half).length() - std::f64::consts::PI).abs() < 1e-12);
    }
}
