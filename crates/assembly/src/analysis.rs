//! What the Jacobian says at a state: its rank, the motions it leaves free,
//! the equations that depend on others, and why a solve came to rest
//! without meeting the tolerance.

use crate::ldl::{Budget, factor};
use crate::newton::{Evals, FACTOR_BUDGET, evaluate, normal_equations};
use crate::system::{BlockOwner, Rows, State, System};
use crate::types::{
    Diagnosis, FreeMotion, Gap, Group, InstanceMotion, Motion, Redundancy, VarKind,
};
use craft3d_math::Vec3;

/// A component of a null vector below this fraction of its largest counts
/// as zero (null vectors are normalised to a largest component of 1).
const ZERO_COMPONENT: f64 = 1e-6;

/// The rank of `J` and a basis of its null space (block order), from
/// `JᵀJ` with the system's rank threshold.
pub(crate) fn rank_and_nulls(sys: &System, evals: &Evals) -> Option<(usize, Vec<Vec<f64>>)> {
    let (a, _) = normal_equations(sys, evals)?;
    let mut budget = Budget::new(FACTOR_BUDGET);
    let f = factor(&sys.normal, a, sys.rank_threshold(), &mut budget).ok()?;
    let nulls = f.null_vectors()?;
    Some((f.rank, nulls))
}

fn normalise(v: &mut [f64]) -> bool {
    let big = v.iter().fold(0.0f64, |m, x| m.max(x.abs()));
    if !(big.is_finite() && big > 0.0) {
        return false;
    }
    for x in v.iter_mut() {
        *x /= big;
    }
    true
}

/// The null vectors as motions of the instances and the mate variables.
pub(crate) fn free_motions(sys: &System, state: &State, nulls: &[Vec<f64>]) -> Vec<FreeMotion> {
    let mut out = Vec::with_capacity(nulls.len());
    for null in nulls {
        let mut x = null.clone();
        if !normalise(&mut x) {
            continue;
        }
        let mut motion = FreeMotion {
            instances: Vec::new(),
            variables: Vec::new(),
        };
        for (b, owner) in sys.block_owner.iter().enumerate() {
            let Some(&o) = sys.block_offsets.get(b) else {
                continue;
            };
            match *owner {
                BlockOwner::Body(i) => {
                    let Some(&[u0, u1, u2, w0, w1, w2]) = x.get(o..o + 6) else {
                        continue;
                    };
                    let u = Vec3::new(u0, u1, u2);
                    let w = Vec3::new(w0, w1, w2);
                    if u.length() <= ZERO_COMPONENT && w.length() <= ZERO_COMPONENT {
                        continue;
                    }
                    let v = u * sys.scale;
                    let t = state.poses.get(i).map_or(Vec3::ZERO, |p| p.translation);
                    let motion_kind = match (w.length() > ZERO_COMPONENT, w.normalized()) {
                        (true, Some(axis)) => {
                            let w2 = w.length_squared();
                            Motion::Turn {
                                axis,
                                point: t + w.cross(v) * (1.0 / w2),
                                pitch: w.dot(v) / w2,
                            }
                        }
                        _ => Motion::Slide {
                            direction: v.normalized().unwrap_or(Vec3::X),
                        },
                    };
                    motion.instances.push(InstanceMotion {
                        instance: i,
                        motion: motion_kind,
                        twist: (v, w),
                    });
                }
                BlockOwner::Vars(m) => {
                    let k = sys.mates.get(m).map_or(0, |mt| mt.kind.variables().len());
                    for var in 0..k {
                        if let Some(&rate) = x.get(o + var)
                            && rate.abs() > ZERO_COMPONENT
                        {
                            motion.variables.push((m, var, rate * sys.unit(m, var)));
                        }
                    }
                }
            }
        }
        out.push(motion);
    }
    out
}

/// A set of equations that depend on each other.
pub(crate) struct Dependency {
    /// The groups involved, in problem order; the last is the one that
    /// added nothing.
    pub(crate) groups: Vec<usize>,
    /// How far the dependent combination is from holding:
    /// `|yᵀr| / |y|`, dimensionless.
    pub(crate) inconsistency: f64,
}

/// The left null space of `J` from `J Jᵀ` factored in problem order.
pub(crate) fn dependencies(sys: &System, evals: &Evals) -> Option<Vec<Dependency>> {
    let ng = sys.groups.len();
    let mut offsets = Vec::with_capacity(ng);
    let mut dim = 0usize;
    for g in &sys.groups {
        offsets.push(dim);
        dim = dim.checked_add(g.size)?;
    }
    let mut vals = sys.gram.zeros();
    let mut m = Vec::new();
    for (gi, g) in sys.groups.iter().enumerate() {
        match evals.get(gi)? {
            None => {
                // Not active: an identity row block, coupled to nothing.
                let mut id = vec![0.0; g.size * g.size];
                for k in 0..g.size {
                    *id.get_mut(k * g.size + k)? = 1.0;
                }
                sys.gram.add(&mut vals, gi, gi, &id)?;
            }
            Some(e) => {
                let mut d = vec![0.0; g.size * g.size];
                for (b, jb) in &e.parts {
                    let sb = *sys.block_sizes.get(*b)?;
                    for r1 in 0..g.size {
                        for r2 in 0..g.size {
                            let mut s = 0.0;
                            for c in 0..sb {
                                s += jb.get(r1 * sb + c)? * jb.get(r2 * sb + c)?;
                            }
                            *d.get_mut(r1 * g.size + r2)? += s;
                        }
                    }
                }
                sys.gram.add(&mut vals, gi, gi, &d)?;
            }
        }
    }
    for (b, list) in sys.by_block.iter().enumerate() {
        let sb = *sys.block_sizes.get(b)?;
        for (i, &g1) in list.iter().enumerate() {
            let Some(Some(e1)) = evals.get(g1) else {
                continue;
            };
            let Some((_, j1)) = e1.parts.iter().find(|(pb, _)| *pb == b) else {
                continue;
            };
            let s1 = e1.r.len();
            for &g2 in list.iter().skip(i + 1) {
                let Some(Some(e2)) = evals.get(g2) else {
                    continue;
                };
                let Some((_, j2)) = e2.parts.iter().find(|(pb, _)| *pb == b) else {
                    continue;
                };
                let s2 = e2.r.len();
                m.clear();
                m.resize(s1 * s2, 0.0);
                for r1 in 0..s1 {
                    for r2 in 0..s2 {
                        let mut s = 0.0;
                        for c in 0..sb {
                            s += j1.get(r1 * sb + c)? * j2.get(r2 * sb + c)?;
                        }
                        *m.get_mut(r1 * s2 + r2)? = s;
                    }
                }
                sys.gram.add(&mut vals, g1, g2, &m)?;
            }
        }
    }
    let mut budget = Budget::new(FACTOR_BUDGET);
    let f = factor(&sys.gram, vals, sys.rank_threshold(), &mut budget).ok()?;
    let mut out = Vec::new();
    for mut y in f.null_vectors()? {
        if !normalise(&mut y) {
            continue;
        }
        let mut dot = 0.0;
        let mut norm2 = 0.0;
        let mut groups = Vec::new();
        for (gi, g) in sys.groups.iter().enumerate() {
            let o = *offsets.get(gi)?;
            let part = y.get(o..o + g.size)?;
            norm2 += part.iter().map(|v| v * v).sum::<f64>();
            if part.iter().any(|v| v.abs() > ZERO_COMPONENT) {
                groups.push(gi);
            }
            if let Some(Some(e)) = evals.get(gi) {
                dot += part.iter().zip(&e.r).map(|(a, b)| a * b).sum::<f64>();
            }
        }
        if groups.is_empty() {
            continue;
        }
        out.push(Dependency {
            groups,
            inconsistency: dot.abs() / norm2.sqrt().max(f64::MIN_POSITIVE),
        });
    }
    Some(out)
}

/// The redundancies of a solved state: every dependency.
pub(crate) fn redundancies(sys: &System, deps: &[Dependency]) -> Vec<Redundancy> {
    deps.iter()
        .map(|d| Redundancy {
            groups: d
                .groups
                .iter()
                .filter_map(|&g| sys.groups.get(g).map(|gi| gi.group))
                .collect(),
        })
        .collect()
}

/// How far the groups are from holding, summed: lengths in model units,
/// angles in radians.
fn gap(sys: &System, evals: &Evals, groups: &[usize]) -> Gap {
    let mut length = 0.0;
    let mut angle = 0.0;
    for &gi in groups {
        let (Some(g), Some(Some(e))) = (sys.groups.get(gi), evals.get(gi)) else {
            continue;
        };
        let norm = |r: &[f64]| r.iter().map(|x| x * x).sum::<f64>().sqrt();
        match g.rows {
            Rows::Mate(_) => {
                length += sys.scale * norm(e.r.get(0..3).unwrap_or(&[]));
                angle += norm(e.r.get(3..).unwrap_or(&[]));
            }
            Rows::Driver { mate, var, .. } | Rows::Limit { mate, var } => {
                let r = norm(&e.r);
                if sys.unit(mate, var) == 1.0 {
                    angle += r;
                } else {
                    length += r * sys.scale;
                }
            }
            Rows::Coupling(c) => {
                let Some(lin) = sys.linears.get(c) else {
                    continue;
                };
                let r = norm(&e.r) * lin.norm;
                let any_length = lin.terms.iter().any(|&(m, v, _)| {
                    sys.mates
                        .get(m)
                        .and_then(|mt| mt.kind.variables().get(v))
                        .is_some_and(|k| *k != VarKind::Angle)
                });
                if any_length {
                    length += r;
                } else {
                    angle += r;
                }
            }
        }
    }
    Gap { length, angle }
}

/// A state moved off `s` by a fixed, irregular step of 1 % of the scale
/// and 0.01 rad per unknown: generic, so a rank the configuration loses
/// at `s` comes back there.
fn perturbed(sys: &System, s: &State) -> Option<State> {
    let golden = 0.618_033_988_749_894_9_f64;
    let h: Vec<f64> = (0..sys.unknowns)
        .map(|k| {
            let f = ((k as f64 + 1.0) * golden).fract();
            0.02 * (f - 0.5)
        })
        .collect();
    sys.step(s, &h)
}

/// Why a solve that came to rest at `state` cannot meet the tolerance.
pub(crate) fn diagnose_rest(
    sys: &System,
    state: &State,
    drives: &[f64],
    active: &[bool],
    iterations: usize,
    residual: f64,
) -> Diagnosis {
    let stuck = Diagnosis::DidNotConverge {
        iterations,
        residual: residual * sys.scale,
    };
    let Some(evals) = evaluate(sys, state, drives, active) else {
        return stuck;
    };
    let Some(deps) = dependencies(sys, &evals) else {
        return stuck;
    };
    let Some(worst) = deps
        .iter()
        .filter(|d| d.inconsistency > sys.eps)
        .max_by(|a, b| a.inconsistency.total_cmp(&b.inconsistency))
    else {
        return stuck;
    };
    let gap = gap(sys, &evals, &worst.groups);
    let groups: Vec<Group> = worst
        .groups
        .iter()
        .filter_map(|&g| sys.groups.get(g).map(|gi| gi.group))
        .collect();
    let rank_here = rank_and_nulls(sys, &evals).map(|(r, _)| r);
    let rank_generic = perturbed(sys, state)
        .and_then(|p| evaluate(sys, &p, drives, active))
        .and_then(|e| rank_and_nulls(sys, &e))
        .map(|(r, _)| r);
    let configuration = matches!((rank_here, rank_generic), (Some(h), Some(g)) if h < g);
    if configuration {
        let loop_mates = groups
            .iter()
            .filter_map(|g| match g {
                Group::Mate(m) => Some(*m),
                _ => None,
            })
            .collect();
        let blamed = groups.last().copied().unwrap_or(Group::Mate(0));
        Diagnosis::Infeasible {
            blamed,
            loop_mates,
            gap,
        }
    } else {
        Diagnosis::OverConstrained { groups, gap }
    }
}
