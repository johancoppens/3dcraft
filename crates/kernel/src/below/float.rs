//! The free-floating equilibrium of closed bodies: the plane, an offset and
//! a turn of its normal, below which the bodies have a given volume and
//! whose centroid below lies on the plane's normal through a given centre
//! of gravity (docs/design/sections-and-levels.md › *A level with a tilt*).
//!
//! Method. For a normal `n` the offset is [`level_for`] (the volume below
//! held exactly), which gives the centroid below `B` and the section's
//! moments about its centroid. With `e1`, `e2` the section's axes and
//! `h = n · (G − B)`, the residual is the arm `r = (e1 · (G − B),
//! e2 · (G − B))`: zero exactly at an equilibrium. Turning the normal by
//! a small `δ` towards `δ1 e1 + δ2 e2` at constant volume moves `B` by
//! `−(1/V) (∫x² , ∫xy ; ∫xy , ∫y²) δ` in the plane (the wedges that
//! emerge and immerse about the section's centroid: the metacentric
//! relation), and turns the axes by `−δ n`, so the derivative of the arm
//! is the symmetric matrix `K = (1/V) (iy, ixy; ixy, ix) − h I`: the
//! metacentric heights GM in the plane. The equilibrium is a zero of `r`,
//! found by Newton's method with that exact derivative, each step at most
//! [`MAX_TURN`] and halved until the arm shrinks. An equilibrium is stable
//! in a principal direction of `K` where its eigenvalue is above the
//! floor (`max(tolerance, 1e-9 size)`), unstable where it is below minus
//! the floor and neutral between (a sphere about its centre turns
//! freely).
//!
//! What is reported: the equilibrium Newton's method reaches from the
//! start (the nearest one), and, where it is unstable, those the body falls
//! to from it: from the unstable equilibrium turned by [`LEAVE`] each way
//! along each direction in which it is unstable, a descent of the energy
//! `n · (G − B)` (the height of `G` above `B`, which an equilibrium makes
//! stationary since `B` moves in the plane) by the same steps with `|K|`
//! (each eigenvalue by its size, at least the floor), each step accepted
//! where the energy falls. So an unstable upright box finds its two angles
//! of loll. Equilibria the search does not reach from there are not
//! reported.

use super::{CutError, CutPlane, Level, extent, level_for};
use craft3d_math::Portable;
use craft3d_math::{Tolerance, Vec3};
use craft3d_topo::Body;

/// Most Newton steps of one search for an equilibrium.
pub const MAX_ITERATIONS: usize = 40;

/// Most offsets (each a level search) one call measures in all, past which
/// it is "did not converge": a bound on the work, far above the dozen or
/// two a settled search takes.
pub const MAX_LEVELS: usize = 240;

/// An equilibrium is found once the arm `|r|` is within this much of the
/// bodies' size (half the diagonal of their box). The centroid below is
/// exact to the level's volume, held to 1e-10 of itself.
pub const ARM_SETTLED: f64 = 1e-9;

/// Where no step shrinks the arm any more, the plane found is still the
/// answer when the arm is within this much of the size, a hundred times
/// [`ARM_SETTLED`]; further off it is "did not converge".
pub const ARM_UNSETTLED: f64 = 1e-7;

/// The largest turn of the normal one step makes, in radians: Newton's
/// method keeps to where the derivative still tells the way.
pub const MAX_TURN: f64 = 0.25;

/// How far, in radians, a search leaves an unstable equilibrium along a
/// direction in which it is unstable.
pub const LEAVE: f64 = 0.1;

/// Most halvings of one step before the search stops.
const MAX_HALVINGS: usize = 10;

/// Two equilibria are one when their normals are this close, in radians.
const SAME: f64 = 1e-6;

/// How an equilibrium answers a small turn in one principal direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stability {
    /// It turns back: GM above the floor.
    Stable,
    /// It turns further: GM below minus the floor.
    Unstable,
    /// Neither, to second order: GM within the floor of zero.
    Neutral,
}

/// A principal direction of the metacentric heights of an equilibrium.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Principal {
    /// The metacentric height GM for a turn about `about`.
    pub gm: f64,
    /// The axis in the plane about which the body turns (unit).
    pub about: Vec3,
    pub stability: Stability,
}

/// One equilibrium.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Equilibrium {
    /// The plane's direction; its offset is `level.below.offset`.
    pub plane: CutPlane,
    pub level: Level,
    /// `|r|`: how far `G` is off the normal through the centroid below.
    pub arm: f64,
    /// `n · (G − B)`: the height of `G` above the centroid below.
    pub energy: f64,
    /// The two principal directions, the smaller GM first.
    pub principal: [Principal; 2],
    /// Newton steps taken to reach it.
    pub iterations: usize,
    /// False where it was taken within [`ARM_UNSETTLED`] because no step
    /// shrank the arm any more.
    pub settled: bool,
}

impl Equilibrium {
    /// Stable in both directions.
    pub fn is_stable(&self) -> bool {
        self.principal
            .iter()
            .all(|p| p.stability == Stability::Stable)
    }

    /// Unstable in some direction.
    pub fn is_unstable(&self) -> bool {
        self.principal
            .iter()
            .any(|p| p.stability == Stability::Unstable)
    }
}

/// What [`floating`] found.
#[derive(Clone, Debug, PartialEq)]
pub struct Floating {
    /// The equilibrium reached from the start first, then those found
    /// from it where it is unstable.
    pub equilibria: Vec<Equilibrium>,
    /// The searches from an unstable equilibrium that ended without one,
    /// and why.
    pub unfinished: Vec<CutError>,
    /// Offsets measured (level searches) in all.
    pub levels: usize,
    /// The floor of GM within which a direction is neutral.
    pub floor: f64,
}

/// What is known at one normal.
#[derive(Clone, Copy, Debug)]
struct State {
    plane: CutPlane,
    level: Level,
    arm: [f64; 2],
    energy: f64,
    /// `K`, symmetric, in the axes `(x_axis, y_axis)`.
    stiffness: [f64; 3],
}

impl State {
    fn arm_length(&self) -> f64 {
        self.arm[0].phypot(self.arm[1])
    }
}

/// The eigenvalues of the symmetric `(a, b; b, c)`, the smaller first,
/// with their unit eigenvectors.
fn eigen([a, b, c]: [f64; 3]) -> [(f64, [f64; 2]); 2] {
    let mean = 0.5 * (a + c);
    let radius = (0.5 * (a - c)).phypot(b);
    // The larger one's eigenvector is at θ with tan 2θ = 2b / (a − c).
    let theta = 0.5 * (2.0 * b).patan2(a - c);
    let (s, co) = theta.psin_cos();
    [(mean - radius, [-s, co]), (mean + radius, [co, s])]
}

/// The plane turned so that its normal tips by `|delta|` towards
/// `delta[0] x_axis + delta[1] y_axis`, the axes turned with it.
fn turned(plane: &CutPlane, delta: [f64; 2]) -> Result<CutPlane, CutError> {
    let (n, e1, e2) = (plane.normal, plane.x_axis, plane.y_axis());
    let toward = e1 * delta[0] + e2 * delta[1];
    let angle = toward.length();
    if !angle.is_finite() {
        return Err(CutError::NotFinite("a turn of the plane"));
    }
    if angle == 0.0 {
        return Ok(*plane);
    }
    let Some(axis) = n.cross(toward).normalized() else {
        return Ok(*plane);
    };
    let (s, c) = angle.psin_cos();
    // Rodrigues: v cos + (axis × v) sin + axis (axis · v)(1 − cos).
    let rotate = |v: Vec3| v * c + axis.cross(v) * s + axis * (axis.dot(v) * (1.0 - c));
    let normal = rotate(n);
    let x = rotate(e1);
    // Square again what rounding left.
    let x = (x - normal * normal.dot(x))
        .normalized()
        .ok_or(CutError::InvalidPlane("the x axis turned along the normal"))?;
    CutPlane::new(normal, x)
}

/// The search's inputs and its count of offsets measured.
struct Search<'a> {
    bodies: &'a [&'a Body],
    volume: f64,
    centre: Vec3,
    tolerance: &'a Tolerance,
    size: f64,
    floor: f64,
    levels: usize,
}

impl Search<'_> {
    fn measure(&mut self, plane: CutPlane) -> Result<State, CutError> {
        self.levels += 1;
        if self.levels > MAX_LEVELS {
            return Err(CutError::NoEquilibrium {
                steps: self.levels,
                arm: f64::NAN,
                normal: plane.normal,
            });
        }
        let level = level_for(self.bodies, &plane, self.volume, self.tolerance)?;
        let below = level.below;
        let b = below.centroid.ok_or(CutError::Section {
            body: 0,
            what: "nothing lies below the plane, so it has no centroid",
        })?;
        let v = below.volume;
        if !(v > 0.0 && v.is_finite()) {
            return Err(CutError::Section {
                body: 0,
                what: "the volume below is not positive",
            });
        }
        let d = self.centre - b;
        let h = plane.normal.dot(d);
        let s = below.section;
        let stiffness = [s.i_y / v - h, s.product / v, s.i_x / v - h];
        let arm = [plane.x_axis.dot(d), plane.y_axis().dot(d)];
        if !(arm.iter().chain(&stiffness).all(|x| x.is_finite()) && h.is_finite()) {
            return Err(CutError::NotFinite("the arm of the centre of gravity"));
        }
        Ok(State {
            plane,
            level,
            arm,
            energy: h,
            stiffness,
        })
    }

    /// The step from `state`: `−K⁻¹ r` in the principal directions, each
    /// eigenvalue at least the floor in size, by its size when
    /// `descend`; at most [`MAX_TURN`].
    fn step(&self, state: &State, descend: bool) -> [f64; 2] {
        let mut delta = [0.0, 0.0];
        for (lambda, u) in eigen(state.stiffness) {
            let along = state.arm[0] * u[0] + state.arm[1] * u[1];
            let size = lambda.abs().max(self.floor);
            let lambda = if descend || lambda == 0.0 {
                size
            } else {
                size.copysign(lambda)
            };
            let k = -along / lambda;
            delta[0] += k * u[0];
            delta[1] += k * u[1];
        }
        let length = delta[0].phypot(delta[1]);
        if length > MAX_TURN {
            let k = MAX_TURN / length;
            delta = [delta[0] * k, delta[1] * k];
        }
        delta
    }

    /// Newton's method from `state` until the arm is settled.
    fn settle(&mut self, mut state: State, descend: bool) -> Result<Equilibrium, CutError> {
        for iteration in 0..MAX_ITERATIONS {
            let arm = state.arm_length();
            if arm <= ARM_SETTLED * self.size {
                return Ok(self.equilibrium(state, iteration, true));
            }
            let mut delta = self.step(&state, descend);
            let mut accepted = None;
            for _ in 0..=MAX_HALVINGS {
                let plane = turned(&state.plane, delta)?;
                match self.measure(plane) {
                    Ok(next) => {
                        let shorter = next.arm_length() < arm;
                        let better = if descend {
                            next.energy < state.energy || next.arm_length() <= 0.5 * arm
                        } else {
                            shorter
                        };
                        if better {
                            accepted = Some(next);
                            break;
                        }
                    }
                    // A plane the boolean refuses, or whose level does not
                    // settle, is stepped around by a shorter step.
                    Err(CutError::Kernel { .. } | CutError::NotConverged { .. }) => {}
                    Err(error) => return Err(error),
                }
                delta = [0.5 * delta[0], 0.5 * delta[1]];
            }
            match accepted {
                Some(next) => state = next,
                None if arm <= ARM_UNSETTLED * self.size => {
                    return Ok(self.equilibrium(state, iteration, false));
                }
                None => {
                    return Err(CutError::NoEquilibrium {
                        steps: iteration,
                        arm,
                        normal: state.plane.normal,
                    });
                }
            }
        }
        Err(CutError::NoEquilibrium {
            steps: MAX_ITERATIONS,
            arm: state.arm_length(),
            normal: state.plane.normal,
        })
    }

    fn equilibrium(&self, state: State, iterations: usize, settled: bool) -> Equilibrium {
        let (n, e1, e2) = (state.plane.normal, state.plane.x_axis, state.plane.y_axis());
        let principal = eigen(state.stiffness).map(|(gm, u)| {
            let toward = e1 * u[0] + e2 * u[1];
            // Tipping the normal towards `toward` turns the body about
            // the axis square to it in the plane.
            let about = n.cross(toward).normalized().unwrap_or(e1);
            let stability = if gm > self.floor {
                Stability::Stable
            } else if gm < -self.floor {
                Stability::Unstable
            } else {
                Stability::Neutral
            };
            Principal {
                gm,
                about,
                stability,
            }
        });
        Equilibrium {
            plane: state.plane,
            level: state.level,
            arm: state.arm_length(),
            energy: state.energy,
            principal,
            iterations,
            settled,
        }
    }
}

/// The equilibria of `bodies` floating with `volume` below the plane and
/// their centre of gravity at `centre`, from the plane `start` (its
/// direction: the upright waterplane), as the module documentation says.
/// An error where the search from the start does not settle; searches
/// from an unstable equilibrium that do not are listed in
/// [`Floating::unfinished`].
pub fn floating(
    bodies: &[&Body],
    start: &CutPlane,
    volume: f64,
    centre: Vec3,
    tolerance: &Tolerance,
) -> Result<Floating, CutError> {
    if !centre.is_finite() {
        return Err(CutError::NotFinite("the centre of gravity"));
    }
    if !volume.is_finite() {
        return Err(CutError::NotFinite("the volume"));
    }
    let (_, _, bbox) = extent(bodies, start.normal)?;
    let size = 0.5 * bbox.diagonal();
    if !(size.is_finite() && size > 0.0) {
        return Err(CutError::NotFinite("the size of the bodies"));
    }
    let mut search = Search {
        bodies,
        volume,
        centre,
        tolerance,
        size,
        floor: tolerance.linear.max(1e-9 * size),
        levels: 0,
    };
    let first = search.measure(*start)?;
    let nearest = search.settle(first, false)?;
    let mut equilibria = vec![nearest];
    let mut unfinished = Vec::new();
    // From an unstable equilibrium, each way along each direction in which
    // it is unstable.
    for principal in nearest.principal {
        if principal.stability != Stability::Unstable {
            continue;
        }
        let (n, e1, e2) = (
            nearest.plane.normal,
            nearest.plane.x_axis,
            nearest.plane.y_axis(),
        );
        // The normal tips square to the axis the body turns about.
        let toward = principal.about.cross(n);
        let along = [toward.dot(e1), toward.dot(e2)];
        for sign in [1.0, -1.0] {
            let delta = [sign * LEAVE * along[0], sign * LEAVE * along[1]];
            let found = turned(&nearest.plane, delta)
                .and_then(|plane| search.measure(plane))
                .and_then(|state| search.settle(state, true));
            match found {
                Ok(found) => {
                    let known = equilibria.iter().any(|e| {
                        let cos = e.plane.normal.dot(found.plane.normal).clamp(-1.0, 1.0);
                        cos.pacos() <= SAME
                    });
                    if !known {
                        equilibria.push(found);
                    }
                }
                Err(error) => unfinished.push(error),
            }
        }
    }
    Ok(Floating {
        equilibria,
        unfinished,
        levels: search.levels,
        floor: search.floor,
    })
}

#[cfg(test)]
mod tests;
