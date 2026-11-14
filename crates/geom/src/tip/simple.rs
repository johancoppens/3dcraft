//! A closed curve in a plane shown not to cross or touch itself.
//!
//! The curve comes as rational Bézier pieces end to end. Three facts
//! about control points carry the proof, each a hull property:
//!
//! - A piece lies in the box of its control points, so two pieces whose
//!   boxes are apart do not meet.
//! - The direction of a rational Bézier curve with positive weights is a
//!   positive combination of the legs of its control polygon (Farin,
//!   *Curves and Surfaces for CAGD*, 5th ed., ch. 13: the derivative is
//!   `Σ wᵢ wⱼ (Bᵢ Bⱼ′ − Bᵢ′ Bⱼ)(Pⱼ − Pᵢ) / w²` over `i < j`, and each
//!   factor is positive). So a run of pieces whose legs all point to one
//!   side of a line moves steadily across it and passes no point twice.
//! - Halving a piece (de Casteljau) gives two pieces that are the same
//!   curve, with boxes and legs nearer to it.
//!
//! A pair of pieces that neither fact settles is halved; what is still
//! unsettled when the work allowed is used up is told as not proven,
//! which is also what a curve that does cross or touch itself comes to.

/// How often a piece is halved before the proof gives up on it.
const MAX_HALVINGS: u32 = 40;
/// The most work the search may do: the control points of every piece
/// it halves, squared and added up (a fraction of a second).
const MAX_WORK: usize = 1 << 27;
/// The least share of a leg's length by which it must point along.
const ALONG: f64 = 1e-12;

/// One rational Bézier piece in the plane: homogeneous control points
/// `[x w, y w, w]`, and which rim curve it is of.
#[derive(Clone, Debug)]
pub(super) struct Piece {
    pub rim: usize,
    h: Vec<[f64; 3]>,
    points: Vec<[f64; 2]>,
    /// `[least x, least y, largest x, largest y]`.
    bounds: [f64; 4],
}

impl Piece {
    /// `None` when a weight is not positive or a number not finite.
    pub fn new(rim: usize, h: Vec<[f64; 3]>) -> Option<Piece> {
        let mut points = Vec::with_capacity(h.len());
        let mut bounds = [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        for &[x, y, w] in &h {
            if !(w.is_finite() && w > 0.0) {
                return None;
            }
            let (x, y) = (x / w, y / w);
            if !(x.is_finite() && y.is_finite()) {
                return None;
            }
            let [x0, y0, x1, y1] = bounds;
            bounds = [x0.min(x), y0.min(y), x1.max(x), y1.max(y)];
            points.push([x, y]);
        }
        (h.len() >= 2).then_some(Piece {
            rim,
            h,
            points,
            bounds,
        })
    }

    /// The piece's two halves by parameter.
    pub fn halves(&self) -> Option<(Piece, Piece)> {
        let mut work = self.h.clone();
        let mut left = Vec::with_capacity(work.len());
        let mut right = Vec::with_capacity(work.len());
        while let (Some(&first), Some(&last)) = (work.first(), work.last()) {
            left.push(first);
            right.push(last);
            work = work
                .windows(2)
                .map(|pair| match pair {
                    [[a0, a1, a2], [b0, b1, b2]] => {
                        [0.5 * (a0 + b0), 0.5 * (a1 + b1), 0.5 * (a2 + b2)]
                    }
                    _ => [0.0; 3],
                })
                .collect();
        }
        right.reverse();
        Some((Piece::new(self.rim, left)?, Piece::new(self.rim, right)?))
    }

    /// The legs of the control polygon that have a length, as unit
    /// vectors.
    fn legs(&self) -> impl Iterator<Item = [f64; 2]> + '_ {
        self.points
            .iter()
            .zip(self.points.iter().skip(1))
            .filter_map(|(&[x0, y0], &[x1, y1])| {
                let (dx, dy) = (x1 - x0, y1 - y0);
                let length = (dx * dx + dy * dy).sqrt();
                (length > 0.0).then(|| [dx / length, dy / length])
            })
    }

    /// Whether every leg points along `e`, and there is one.
    fn runs_along(&self, e: [f64; 2]) -> bool {
        let [ex, ey] = e;
        let mut any = false;
        for [dx, dy] in self.legs() {
            if dx * ex + dy * ey <= ALONG {
                return false;
            }
            any = true;
        }
        any
    }

    fn size(&self) -> f64 {
        let [x0, y0, x1, y1] = self.bounds;
        (x1 - x0).max(y1 - y0)
    }
}

fn unit(x: f64, y: f64) -> Option<[f64; 2]> {
    let length = (x * x + y * y).sqrt();
    (length.is_finite() && length > 0.0).then(|| [x / length, y / length])
}

/// The search, with the work it has done.
pub(super) struct Search {
    work: usize,
    /// Pieces nearer than this are not apart.
    clear: f64,
}

impl Search {
    pub fn new(clear: f64) -> Search {
        Search { work: 0, clear }
    }

    /// Whether nothing more may be halved; counts the halving of pieces
    /// with `points` control points together otherwise.
    fn spent(&mut self, halvings: u32, points: usize) -> bool {
        self.work = self.work.saturating_add(points.saturating_mul(points));
        halvings >= MAX_HALVINGS || self.work >= MAX_WORK
    }

    /// Whether the piece is proven to pass no point twice.
    pub fn simple(&mut self, piece: &Piece, halvings: u32) -> bool {
        // Steadily along its own chord: no point twice.
        let (Some(&[x0, y0]), Some(&[x1, y1])) = (piece.points.first(), piece.points.last()) else {
            return false;
        };
        if unit(x1 - x0, y1 - y0).is_some_and(|e| piece.runs_along(e)) {
            return true;
        }
        if self.spent(halvings, piece.points.len()) {
            return false;
        }
        let Some((first, second)) = piece.halves() else {
            return false;
        };
        self.simple(&first, halvings + 1)
            && self.simple(&second, halvings + 1)
            && self.joined(&first, &second, halvings + 1)
    }

    /// Whether two pieces, the second starting where the first ends and
    /// each passing no point twice, are proven to meet nowhere else.
    pub fn joined(&mut self, a: &Piece, b: &Piece, halvings: u32) -> bool {
        // Both steadily along the line that halves the corner between
        // them (the tangent, where there is no corner): one arc. Two
        // pieces that turn straight back on one another have no such
        // line.
        let (Some([ax, ay]), Some([bx, by])) = (a.legs().last(), b.legs().next()) else {
            return false;
        };
        if unit(ax + bx, ay + by).is_some_and(|e| a.runs_along(e) && b.runs_along(e)) {
            return true;
        }
        if self.spent(halvings, a.points.len() + b.points.len()) {
            return false;
        }
        let (Some((a1, a2)), Some((b1, b2))) = (a.halves(), b.halves()) else {
            return false;
        };
        self.joined(&a2, &b1, halvings + 1)
            && self.apart(&a1, &b1, halvings + 1)
            && self.apart(&a1, &b2, halvings + 1)
            && self.apart(&a2, &b2, halvings + 1)
    }

    /// Whether two pieces are proven more than the clearance apart.
    pub fn apart(&mut self, a: &Piece, b: &Piece, halvings: u32) -> bool {
        let ([ax0, ay0, ax1, ay1], [bx0, by0, bx1, by1]) = (a.bounds, b.bounds);
        let clear = self.clear;
        if ax1 + clear < bx0 || bx1 + clear < ax0 || ay1 + clear < by0 || by1 + clear < ay0 {
            return true;
        }
        if self.spent(halvings, a.points.len().max(b.points.len())) {
            return false;
        }
        if a.size() >= b.size() {
            let Some((first, second)) = a.halves() else {
                return false;
            };
            self.apart(&first, b, halvings + 1) && self.apart(&second, b, halvings + 1)
        } else {
            let Some((first, second)) = b.halves() else {
                return false;
            };
            self.apart(a, &first, halvings + 1) && self.apart(a, &second, halvings + 1)
        }
    }
}

/// Whether the closed curve of the pieces, each starting where the one
/// before ends and the first where the last ends, is proven to pass no
/// point twice and to stay more than `clear` from itself wherever it is
/// not one run. `Err` names the rim curves of two pieces that could not
/// be told apart.
pub(super) fn closed_curve_is_simple(pieces: &[Piece], clear: f64) -> Result<(), (usize, usize)> {
    // Halved once: a curve of two pieces has four, so that no two meet at
    // both of their ends.
    let mut halves = Vec::with_capacity(2 * pieces.len());
    for piece in pieces {
        let Some((first, second)) = piece.halves() else {
            return Err((piece.rim, piece.rim));
        };
        halves.extend([first, second]);
    }
    let count = halves.len();
    let mut search = Search::new(clear);
    for (i, piece) in halves.iter().enumerate() {
        if !search.simple(piece, 0) {
            return Err((piece.rim, piece.rim));
        }
        let Some(next) = halves.get((i + 1) % count) else {
            continue;
        };
        if !search.joined(piece, next, 0) {
            return Err((piece.rim, next.rim));
        }
    }
    // Every other pair, found by a sweep along x: only pieces whose boxes
    // overlap along it are looked at.
    let mut order: Vec<usize> = (0..count).collect();
    order.sort_by(|&i, &j| {
        let key = |k: usize| halves.get(k).map_or(f64::INFINITY, |p| p.bounds[0]);
        key(i).total_cmp(&key(j))
    });
    for (at, &i) in order.iter().enumerate() {
        let Some(a) = halves.get(i) else { continue };
        for &j in order.iter().skip(at + 1) {
            let Some(b) = halves.get(j) else { continue };
            if b.bounds[0] > a.bounds[2] + clear {
                break;
            }
            let between = i.abs_diff(j);
            if between == 1 || between == count - 1 {
                continue;
            }
            if !search.apart(a, b, 0) {
                return Err((a.rim.min(b.rim), a.rim.max(b.rim)));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(rim: usize, from: [f64; 2], to: [f64; 2]) -> Piece {
        Piece::new(rim, vec![[from[0], from[1], 1.0], [to[0], to[1], 1.0]]).unwrap()
    }

    fn polygon(corners: &[[f64; 2]]) -> Vec<Piece> {
        (0..corners.len())
            .map(|i| line(i, corners[i], corners[(i + 1) % corners.len()]))
            .collect()
    }

    #[test]
    fn polygons_are_told_simple_or_not() {
        let square = [[1.0, -1.0], [1.0, 1.0], [-1.0, 1.0], [-1.0, -1.0]];
        assert_eq!(closed_curve_is_simple(&polygon(&square), 1e-9), Ok(()));
        // A bow tie crosses itself; a square gone round twice lies on
        // itself; a spike folded back on itself touches itself.
        let bow = [[1.0, -1.0], [-1.0, 1.0], [1.0, 1.0], [-1.0, -1.0]];
        assert!(closed_curve_is_simple(&polygon(&bow), 1e-9).is_err());
        let twice: Vec<[f64; 2]> = square.iter().chain(&square).copied().collect();
        assert!(closed_curve_is_simple(&polygon(&twice), 1e-9).is_err());
        let spike = [[0.0, 0.0], [2.0, 0.0], [1.0, 0.0], [1.0, 1.0]];
        assert!(closed_curve_is_simple(&polygon(&spike), 1e-9).is_err());
        // A notch narrower than the clearance is not apart from itself.
        let notch = |gap: f64| {
            [
                [0.0, 0.0],
                [4.0, 0.0],
                [4.0, 4.0],
                [2.0 + gap, 4.0],
                [2.0 + gap, 1.0],
                [2.0, 1.0],
                [2.0, 4.0],
                [0.0, 4.0],
            ]
        };
        assert_eq!(closed_curve_is_simple(&polygon(&notch(1e-3)), 1e-6), Ok(()));
        assert!(closed_curve_is_simple(&polygon(&notch(1e-7)), 1e-6).is_err());
    }

    #[test]
    fn curves_are_told_simple_or_not() {
        let s = std::f64::consts::FRAC_1_SQRT_2;
        let quarter = |rim: usize, p: [[f64; 2]; 3]| {
            Piece::new(
                rim,
                vec![
                    [p[0][0], p[0][1], 1.0],
                    [p[1][0] * s, p[1][1] * s, s],
                    [p[2][0], p[2][1], 1.0],
                ],
            )
            .unwrap()
        };
        let circle = vec![
            quarter(0, [[1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]),
            quarter(1, [[0.0, 1.0], [-1.0, 1.0], [-1.0, 0.0]]),
            quarter(2, [[-1.0, 0.0], [-1.0, -1.0], [0.0, -1.0]]),
            quarter(3, [[0.0, -1.0], [1.0, -1.0], [1.0, 0.0]]),
        ];
        assert_eq!(closed_curve_is_simple(&circle, 1e-9), Ok(()));
        // Two cubic arcs with sharp ends, and the same with one arc
        // drawn through the other.
        let arc = |rim: usize, a: f64, b: f64| {
            Piece::new(
                rim,
                vec![[a, 0.0, 1.0], [a, b, 1.0], [-a, b, 1.0], [-a, 0.0, 1.0]],
            )
            .unwrap()
        };
        assert_eq!(
            closed_curve_is_simple(&[arc(0, 6.0, 1.0), arc(1, -6.0, -1.0)], 1e-9),
            Ok(())
        );
        let looped = Piece::new(
            1,
            vec![
                [-6.0, 0.0, 1.0],
                [20.0, 3.0, 1.0],
                [-20.0, 3.0, 1.0],
                [6.0, 0.0, 1.0],
            ],
        )
        .unwrap();
        assert!(closed_curve_is_simple(&[arc(0, 6.0, 1.0), looped], 1e-9).is_err());
    }
}
