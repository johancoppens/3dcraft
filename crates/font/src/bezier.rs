//! Bézier segments of degree 1 to 3 in the plane, and closed or open runs
//! of them: what a glyph outline is made of.
//!
//! TrueType outlines are lines and quadratic Béziers, CFF outlines lines
//! and cubic Béziers; both are kept exactly as the font has them (the
//! points are the font's numbers, converted to `f64` without rounding).

/// A point in font units (or, after scaling, in document units).
pub type P2 = [f64; 2];

pub(crate) fn sub(a: P2, b: P2) -> P2 {
    [a[0] - b[0], a[1] - b[1]]
}

pub(crate) fn add(a: P2, b: P2) -> P2 {
    [a[0] + b[0], a[1] + b[1]]
}

pub(crate) fn scale(a: P2, s: f64) -> P2 {
    [a[0] * s, a[1] * s]
}

pub(crate) fn lerp(a: P2, b: P2, t: f64) -> P2 {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

pub(crate) fn cross(a: P2, b: P2) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}

pub(crate) fn dist(a: P2, b: P2) -> f64 {
    libm::hypot(a[0] - b[0], a[1] - b[1])
}

/// A Bézier segment: `degree + 1` points of `p` are used (1 a line, 2 a
/// quadratic, 3 a cubic); the rest are zero.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Seg {
    pub degree: usize,
    pub p: [P2; 4],
}

impl Seg {
    pub fn line(a: P2, b: P2) -> Self {
        Self {
            degree: 1,
            p: [a, b, [0.0; 2], [0.0; 2]],
        }
    }

    pub fn quad(a: P2, b: P2, c: P2) -> Self {
        Self {
            degree: 2,
            p: [a, b, c, [0.0; 2]],
        }
    }

    pub fn cubic(a: P2, b: P2, c: P2, d: P2) -> Self {
        Self {
            degree: 3,
            p: [a, b, c, d],
        }
    }

    /// The points that define it, `degree + 1` of them.
    pub fn points(&self) -> &[P2] {
        self.p.get(..=self.degree.min(3)).unwrap_or(&[])
    }

    pub fn start(&self) -> P2 {
        self.p[0]
    }

    pub fn end(&self) -> P2 {
        self.points().last().copied().unwrap_or(self.p[0])
    }

    pub(crate) fn set_start(&mut self, at: P2) {
        self.p[0] = at;
    }

    pub(crate) fn set_end(&mut self, at: P2) {
        let d = self.degree.min(3);
        if let Some(slot) = self.p.get_mut(d) {
            *slot = at;
        }
    }

    /// The point at `t` (de Casteljau).
    pub fn eval(&self, t: f64) -> P2 {
        let mut q = self.p;
        let d = self.degree.min(3);
        for r in 1..=d {
            for i in 0..=(d - r) {
                if let (Some(&a), Some(&b)) = (q.get(i), q.get(i + 1))
                    && let Some(slot) = q.get_mut(i)
                {
                    *slot = lerp(a, b, t);
                }
            }
        }
        q[0]
    }

    /// The derivative at `t`.
    pub fn derivative(&self, t: f64) -> P2 {
        let d = self.degree.min(3);
        if d == 0 {
            return [0.0; 2];
        }
        // The hodograph: degree d − 1 on the differences, times d.
        let pts = self.points();
        let mut h = [[0.0; 2]; 3];
        for (i, w) in pts.windows(2).enumerate() {
            if let (Some(slot), [a, b]) = (h.get_mut(i), w) {
                *slot = scale(sub(*b, *a), d as f64);
            }
        }
        let hodo = Seg {
            degree: d - 1,
            p: [h[0], h[1], h[2], [0.0; 2]],
        };
        hodo.eval(t)
    }

    /// The two halves at `t`, exactly the same curve.
    pub fn split(&self, t: f64) -> (Seg, Seg) {
        let d = self.degree.min(3);
        let mut left = [[0.0; 2]; 4];
        let mut right = [[0.0; 2]; 4];
        let mut q = self.p;
        for r in 0..=d {
            if let (Some(l), Some(&first)) = (left.get_mut(r), q.first()) {
                *l = first;
            }
            if let (Some(rr), Some(&last)) = (right.get_mut(d - r), q.get(d - r)) {
                *rr = last;
            }
            for i in 0..(d - r) {
                if let (Some(&a), Some(&b)) = (q.get(i), q.get(i + 1))
                    && let Some(slot) = q.get_mut(i)
                {
                    *slot = lerp(a, b, t);
                }
            }
        }
        (
            Seg { degree: d, p: left },
            Seg {
                degree: d,
                p: right,
            },
        )
    }

    /// The part from `t0` to `t1` (`t0 < t1`).
    pub fn sub_range(&self, t0: f64, t1: f64) -> Seg {
        if t0 <= 0.0 && t1 >= 1.0 {
            return *self;
        }
        let right = if t0 > 0.0 { self.split(t0).1 } else { *self };
        if t1 >= 1.0 {
            return right;
        }
        let u = (t1 - t0) / (1.0 - t0);
        right.split(u.clamp(0.0, 1.0)).0
    }

    /// The same curve run the other way.
    pub fn reversed(&self) -> Seg {
        let d = self.degree.min(3);
        let mut p = [[0.0; 2]; 4];
        for i in 0..=d {
            if let (Some(slot), Some(&q)) = (p.get_mut(i), self.p.get(d - i)) {
                *slot = q;
            }
        }
        Seg { degree: d, p }
    }

    /// The same curve with a higher degree (at most 3).
    pub fn elevated_to(&self, degree: usize) -> Seg {
        let mut s = *self;
        while s.degree < degree.min(3) {
            let d = s.degree;
            let mut p = [[0.0; 2]; 4];
            let n = (d + 1) as f64;
            for i in 0..=(d + 1) {
                let a = if i == 0 {
                    [0.0; 2]
                } else {
                    s.p.get(i - 1).copied().unwrap_or([0.0; 2])
                };
                let b = s.p.get(i).copied().unwrap_or([0.0; 2]);
                let w = i as f64 / n;
                if let Some(slot) = p.get_mut(i) {
                    *slot = if i == 0 {
                        b
                    } else if i == d + 1 {
                        a
                    } else {
                        add(scale(a, w), scale(b, 1.0 - w))
                    };
                }
            }
            s = Seg { degree: d + 1, p };
        }
        s
    }

    /// `[min x, min y, max x, max y]` of the control points, which hold
    /// the curve.
    pub fn bbox(&self) -> [f64; 4] {
        let mut b = [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        for p in self.points() {
            b = [
                b[0].min(p[0]),
                b[1].min(p[1]),
                b[2].max(p[0]),
                b[3].max(p[1]),
            ];
        }
        b
    }

    /// Whether every control point is within `eps` of the start: a
    /// segment of no length.
    pub(crate) fn is_point(&self, eps: f64) -> bool {
        let s = self.start();
        self.points().iter().all(|p| dist(*p, s) <= eps)
    }

    /// The signed area term of Green's theorem, `∫ x dy − y dx / 2`,
    /// exact for a Bézier (closed form per degree via the control points).
    pub fn area_term(&self) -> f64 {
        // ∫ (x y' − y x') dt / 2 over [0, 1], by Gauss–Legendre with five
        // points: the integrand is a polynomial of degree at most
        // 2·degree − 1 ≤ 5, which five points integrate exactly.
        const X: [f64; 5] = [
            0.046_910_077_030_668,
            0.230_765_344_947_158,
            0.5,
            0.769_234_655_052_842,
            0.953_089_922_969_332,
        ];
        const W: [f64; 5] = [
            0.118_463_442_528_095,
            0.239_314_335_249_683,
            0.284_444_444_444_444,
            0.239_314_335_249_683,
            0.118_463_442_528_095,
        ];
        X.iter()
            .zip(W)
            .map(|(&t, w)| {
                let p = self.eval(t);
                let d = self.derivative(t);
                w * cross(p, d) / 2.0
            })
            .sum()
    }

    /// The parameters in (0, 1) where `y` has a turning point.
    pub(crate) fn y_extrema(&self) -> Vec<f64> {
        let y: Vec<f64> = self.points().iter().map(|p| p[1]).collect();
        let mut out = Vec::new();
        match y.as_slice() {
            [a, b, c] => {
                let den = a - 2.0 * b + c;
                if den != 0.0 {
                    out.push((a - b) / den);
                }
            }
            [a, b, c, d] => {
                // Derivative /3: (b−a)(1−t)² + 2(c−b)(1−t)t + (d−c)t².
                let (p, q, r) = (b - a, c - b, d - c);
                let qa = p - 2.0 * q + r;
                let qb = 2.0 * (q - p);
                let qc = p;
                if qa.abs() < 1e-300 {
                    if qb != 0.0 {
                        out.push(-qc / qb);
                    }
                } else {
                    let disc = qb * qb - 4.0 * qa * qc;
                    if disc >= 0.0 {
                        let s = disc.sqrt();
                        out.push((-qb + s) / (2.0 * qa));
                        out.push((-qb - s) / (2.0 * qa));
                    }
                }
            }
            _ => {}
        }
        out.retain(|t| t.is_finite() && *t > 0.0 && *t < 1.0);
        out.sort_by(f64::total_cmp);
        out
    }
}

/// A run of segments, each starting where the one before it ends; closed
/// when the last ends where the first starts.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Contour {
    pub segs: Vec<Seg>,
    pub closed: bool,
}

impl Contour {
    /// The enclosed area of a closed contour: positive counter-clockwise
    /// (y up).
    pub fn signed_area(&self) -> f64 {
        self.segs.iter().map(Seg::area_term).sum()
    }

    /// The highest degree of its segments.
    pub fn degree(&self) -> usize {
        self.segs.iter().map(|s| s.degree).max().unwrap_or(1)
    }

    pub fn bbox(&self) -> [f64; 4] {
        let mut b = [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        for s in &self.segs {
            let o = s.bbox();
            b = [
                b[0].min(o[0]),
                b[1].min(o[1]),
                b[2].max(o[2]),
                b[3].max(o[3]),
            ];
        }
        b
    }

    /// Every point mapped by `f` (an affine map keeps the curves exact).
    pub fn mapped(&self, f: &dyn Fn(P2) -> P2) -> Contour {
        Contour {
            segs: self
                .segs
                .iter()
                .map(|s| {
                    let mut t = *s;
                    for p in t.p.iter_mut().take(s.degree + 1) {
                        *p = f(*p);
                    }
                    t
                })
                .collect(),
            closed: self.closed,
        }
    }

    /// The contour run the other way.
    pub fn reversed(&self) -> Contour {
        Contour {
            segs: self.segs.iter().rev().map(Seg::reversed).collect(),
            closed: self.closed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_halves_meet_and_match_the_curve() {
        let c = Seg::cubic([0.0, 0.0], [1.0, 3.0], [4.0, -1.0], [5.0, 2.0]);
        let (a, b) = c.split(0.3);
        assert_eq!(a.end(), b.start());
        let p = c.eval(0.3 + 0.7 * 0.5);
        let q = b.eval(0.5);
        assert!(dist(p, q) < 1e-12);
        let r = c.sub_range(0.2, 0.6);
        assert!(dist(r.eval(0.5), c.eval(0.4)) < 1e-12);
    }

    #[test]
    fn elevation_keeps_the_curve() {
        let q = Seg::quad([0.0, 0.0], [2.0, 4.0], [5.0, 1.0]);
        let c = q.elevated_to(3);
        assert_eq!(c.degree, 3);
        for t in [0.0, 0.25, 0.5, 0.9, 1.0] {
            assert!(dist(q.eval(t), c.eval(t)) < 1e-12);
        }
        let l = Seg::line([0.0, 0.0], [3.0, 3.0]).elevated_to(2);
        assert!(dist(l.p[1], [1.5, 1.5]) < 1e-15);
    }

    #[test]
    fn area_of_a_unit_square_and_a_quadratic_cap() {
        let sq = Contour {
            segs: vec![
                Seg::line([0.0, 0.0], [1.0, 0.0]),
                Seg::line([1.0, 0.0], [1.0, 1.0]),
                Seg::line([1.0, 1.0], [0.0, 1.0]),
                Seg::line([0.0, 1.0], [0.0, 0.0]),
            ],
            closed: true,
        };
        assert!((sq.signed_area() - 1.0).abs() < 1e-12);
        // Parabolic segment over a chord of 2 with control at height 2:
        // area 2/3 · base · height (height 1) = 4/3.
        let cap = Contour {
            segs: vec![
                Seg::quad([2.0, 0.0], [1.0, 2.0], [0.0, 0.0]),
                Seg::line([0.0, 0.0], [2.0, 0.0]),
            ],
            closed: true,
        };
        assert!((cap.signed_area() - 4.0 / 3.0).abs() < 1e-12);
        assert!((sq.reversed().signed_area() + 1.0).abs() < 1e-12);
    }
}
