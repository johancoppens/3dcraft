//! Hostile input never panics: random problems with garbage numbers,
//! indices out of range, degenerate frames, contradictions and sequences
//! of wild drives all return a solution or a diagnosis.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_assembly::{
    CouplingKind, CouplingSpec, Frame, Mat3, MateKind, MateSpec, Pose, Problem, Solver, solve,
};
use craft3d_math::Vec3;

struct Lcg(u64);

impl Lcg {
    fn u(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn pick(&mut self, n: u64) -> u64 {
        self.u() % n.max(1)
    }
    fn f(&mut self) -> f64 {
        match self.pick(12) {
            0 => f64::NAN,
            1 => f64::INFINITY,
            2 => -1e300,
            3 => 0.0,
            4 => 1e-300,
            _ => (self.u() as f64 / (1u64 << 31) as f64 - 1.0) * 10.0,
        }
    }
    fn tame(&mut self) -> f64 {
        (self.u() as f64 / (1u64 << 31) as f64 - 1.0) * 10.0
    }
    fn v(&mut self, wild: bool) -> Vec3 {
        if wild {
            Vec3::new(self.f(), self.f(), self.f())
        } else {
            Vec3::new(self.tame(), self.tame(), self.tame())
        }
    }
    fn frame(&mut self, wild: bool) -> Frame {
        let axes = if self.pick(6) == 0 {
            Mat3::from_cols(self.v(wild), self.v(wild), self.v(wild))
        } else {
            Mat3::exp(self.v(false))
        };
        Frame::new(self.v(wild), axes)
    }
}

const KINDS: [MateKind; 6] = [
    MateKind::Fixed,
    MateKind::Revolute,
    MateKind::Slider,
    MateKind::Cylindrical,
    MateKind::Planar,
    MateKind::Ball,
];

fn random_problem(g: &mut Lcg) -> Problem {
    let wild = g.pick(3) == 0;
    let n = g.pick(6) as usize;
    let tol = if wild { g.f() } else { 1e-6 };
    let scale = if wild { g.f() } else { 10.0 };
    let mut p = Problem::new(n, tol, scale);
    if g.pick(4) > 0 {
        p.grounded.push(g.pick(n as u64 + 1) as usize);
    }
    for _ in 0..g.pick(8) {
        let a = g.pick(n as u64 + 1) as usize;
        let b = g.pick(n as u64 + 1) as usize;
        let kind = KINDS[g.pick(6) as usize];
        let mut m = MateSpec::new(kind, (a, g.frame(wild)), (b, g.frame(wild)));
        m.offset = if wild { g.f() } else { g.tame() };
        m.angle = if wild { g.f() } else { g.tame() };
        m.flip = g.pick(2) == 0;
        for v in 0..3 {
            if g.pick(3) == 0 {
                m.driven[v] = Some(if wild { g.f() } else { g.tame() });
            }
            if g.pick(4) == 0 {
                let (x, y) = (g.tame(), g.tame());
                m.limits[v] = Some((x.min(y), if wild { g.f() } else { x.max(y) }));
            }
        }
        p.mates.push(m);
    }
    for _ in 0..g.pick(3) {
        let kind = match g.pick(4) {
            0 => CouplingKind::Gear,
            1 => CouplingKind::Rack,
            2 => CouplingKind::Screw,
            _ => CouplingKind::Linear(
                (0..g.pick(4))
                    .map(|_| (g.pick(10) as usize, g.pick(4) as usize, g.f()))
                    .collect(),
            ),
        };
        let mates = (g.pick(10) as usize, g.pick(10) as usize);
        p.couplings.push(CouplingSpec {
            kind,
            mates,
            value: g.f(),
        });
    }
    for pose in &mut p.start {
        if g.pick(3) == 0 {
            *pose = Pose::new(Mat3::exp(g.v(false)), g.v(wild));
        }
    }
    if g.pick(5) == 0 {
        p.start.pop();
    }
    p
}

#[test]
fn random_problems_never_panic() {
    let mut g = Lcg(2026);
    let mut solved = 0;
    let mut diagnosed = 0;
    for _ in 0..1500 {
        let p = random_problem(&mut g);
        match solve(&p) {
            Ok(sol) => {
                solved += 1;
                assert!(
                    sol.poses
                        .iter()
                        .all(|q| q.translation.is_finite() && q.rotation.is_finite())
                );
                assert_eq!(sol.free.len(), sol.dof);
            }
            Err(craft3d_assembly::Diagnosis::Invalid { .. }) => {}
            Err(_) => diagnosed += 1,
        }
        if let Ok(mut s) = Solver::new(&p) {
            let k = p.driven_slots().len();
            let frames: Vec<Vec<f64>> = (0..3)
                .map(|_| (0..k + g.pick(2) as usize).map(|_| g.f()).collect())
                .collect();
            let seq = s.sequence(&frames);
            assert!(seq.frames.len() <= frames.len());
            let _ = s.advance(&[]);
        }
    }
    // Random frames mostly contradict each other: both solutions and real
    // diagnoses (beyond malformed input) are reached.
    assert!(
        solved > 0 && diagnosed > 0,
        "{solved} solved, {diagnosed} diagnosed"
    );
}
