//! One face to triangles, in the parameter space of its surface.
//!
//! 1. **Boundary.** The points of the face's loops (the edge points, see
//!    `edge.rs`) are projected onto the surface and followed around each
//!    loop, keeping the parameters continuous across the seam of a
//!    surface that closes on itself: a point's parameter is the one
//!    nearest its predecessor's, never the raw projection, which may come
//!    back as 0 or as a rounding error below a whole period.
//! 2. **Region.** The loops become closed chains of segments in the
//!    parameter plane. A loop that winds once around the axis is cut open
//!    along a *seam*, a line from it to the loop on the other side (or to
//!    the end of the surface: a pole or an apex); both sides of the cut
//!    get the same 3D points. Where the parametrisation is singular (a
//!    pole, an apex, a collapsed edge of a patch) a whole stretch of the
//!    parameter boundary is one 3D point; its vertices all carry that
//!    point, and triangles between two of them are dropped at the end.
//!    This follows `craft3d_topo`'s mass properties, which treat winding
//!    loops, poles and apexes the same way.
//! 3. **Triangulation.** A constrained Delaunay triangulation of the
//!    chains (`cdt.rs`), in parameters scaled by the size of the surface's
//!    derivatives so that distances resemble those in space.
//!    Before that, the face says which chords of its edges it cannot do
//!    with (`Boundary::chords_to_refine`): those that cross another part
//!    of the boundary, and those the parameters distort. `body.rs` has
//!    them cut in two and comes back with the new points.
//! 4. **Refinement.** Curved faces are seeded with a grid of interior
//!    points sized from the surface's curvature, and then every triangle
//!    is measured: the surface point at the mean parameters of each free
//!    edge and of the triangle must be within the chordal tolerance of the
//!    edge's midpoint and the triangle's centroid, and the triangle must
//!    face the way the surface does. A triangle that fails gets a point on
//!    its worst edge or at its centroid, until none fails. Boundary edges
//!    are never split: their chords were sized against their own curves.
//!
//! The 3D position of a boundary vertex is always the edge point itself.

use crate::cdt::{Cdt, CdtError, FIRST_VERTEX, Interior, P2, orient};
use crate::edge::arc_segments;
use crate::{TessError, TessParams};
use craft3d_geom::{Curve3, GeomError, Surface};
use craft3d_math::Vec3;
use craft3d_topo::{Body, Face};
use std::f64::consts::FRAC_PI_2;

/// Surface parameters, in the order this face works in (see [`Param`]).
type Uv = [f64; 2];

/// A chord of an edge's polyline: the edge, and the number of the chord
/// counted from the edge's start.
pub(crate) type Chord = (u32, u32);

/// What a face adds to the mesh: its own vertices and triangles over them.
#[derive(Debug, Default)]
pub(crate) struct FaceMesh {
    pub positions: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    pub triangles: Vec<[u32; 3]>,
}

fn fail<T>(face: usize, reason: impl Into<String>) -> Result<T, TessError> {
    Err(TessError::Face {
        face,
        reason: reason.into(),
    })
}

fn unsupported<T>(face: usize, what: impl Into<String>) -> Result<T, TessError> {
    Err(TessError::Unsupported {
        face,
        what: what.into(),
    })
}

/// A surface with its two parameters possibly swapped, so that a loop
/// that winds around the tube of a torus can be handled as one that winds
/// around its axis. Parameter 0 is called `a`, parameter 1 `b`.
#[derive(Clone, Copy)]
struct Param<'a> {
    surface: &'a Surface,
    swapped: bool,
}

impl Param<'_> {
    fn raw(&self, p: Uv) -> (f64, f64) {
        let [a, b] = p;
        if self.swapped { (b, a) } else { (a, b) }
    }

    fn order<T>(&self, u: T, v: T) -> [T; 2] {
        if self.swapped { [v, u] } else { [u, v] }
    }

    fn eval(&self, p: Uv) -> Vec3 {
        let (u, v) = self.raw(p);
        self.surface.eval(u, v)
    }

    /// `(point, ∂/∂a, ∂/∂b)`.
    fn derivatives(&self, p: Uv) -> (Vec3, [Vec3; 2]) {
        let (u, v) = self.raw(p);
        let (point, du, dv) = self.surface.derivatives(u, v);
        (point, self.order(du, dv))
    }

    /// The parameters of the surface point nearest `point`, which is a
    /// point of an edge of the face and so lies on the surface.
    fn project(&self, point: Vec3) -> Result<Uv, GeomError> {
        let (u, v) = self.surface.project(point)?;
        Ok(self.order(u, v))
    }

    fn periods(&self) -> [Option<f64>; 2] {
        self.order(self.surface.period_u(), self.surface.period_v())
    }

    fn domain(&self) -> [(f64, f64); 2] {
        let (u, v) = self.surface.domain();
        self.order(u, v)
    }

    /// The surface's own unit normal `∂u × ∂v`, whatever the order.
    fn normal(&self, p: Uv) -> Option<Vec3> {
        let (u, v) = self.raw(p);
        self.surface.normal(u, v)
    }

    /// `∂a × ∂b` points along the surface normal times this.
    fn handedness(&self) -> f64 {
        if self.swapped { -1.0 } else { 1.0 }
    }

    fn is_plane(&self) -> bool {
        matches!(self.surface, Surface::Plane(_))
    }

    fn is_freeform(&self) -> bool {
        matches!(self.surface, Surface::Bezier(_) | Surface::Nurbs(_))
    }
}

/// A boundary vertex of the region.
#[derive(Clone, Copy, Debug)]
struct Bv {
    uv: Uv,
    pos: Vec3,
    /// The parameter that means nothing here: this vertex is on a pole,
    /// an apex or a collapsed edge, where changing it does not move the
    /// point.
    free: Option<usize>,
    /// The chord of an edge that runs from this vertex to the next of its
    /// loop. (Of two vertices on a pole both name the chord that leaves
    /// the second; the stretch between them is told apart later.)
    chord: Option<Chord>,
}

/// One loop in parameter space. Vertex `i` is joined to `i + 1`, and the
/// last to the first moved by `winding` periods.
struct Ring {
    verts: Vec<Bv>,
    winding: [i32; 2],
}

/// `x > 0`; false for NaN.
fn positive(x: f64) -> bool {
    x > 0.0
}

/// Not `a ≤ b`: `a` is larger, or one of them is NaN. Limits are tested
/// with this, so that a NaN is over every limit.
fn exceeds(a: f64, b: f64) -> bool {
    !matches!(
        a.partial_cmp(&b),
        Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal)
    )
}

fn get2<T: Copy>(a: [T; 2], k: usize) -> T {
    let [x, y] = a;
    if k == 0 { x } else { y }
}

fn set2<T>(a: &mut [T; 2], k: usize, value: T) {
    let [x, y] = a;
    if k == 0 {
        *x = value;
    } else {
        *y = value;
    }
}

fn tuple(p: Uv) -> P2 {
    let [a, b] = p;
    (a, b)
}

/// `value` moved by whole periods to be as near `to` as it can.
fn nearest(value: f64, to: f64, period: Option<f64>) -> f64 {
    match period {
        Some(p) if p.is_finite() && p > 0.0 => {
            let turns = ((to - value) / p).round();
            if turns.is_finite() {
                value + turns * p
            } else {
                value
            }
        }
        _ => value,
    }
}

fn shifted(uv: Uv, turns: [i32; 2], periods: [Option<f64>; 2]) -> Uv {
    let [a, b] = uv;
    let [ta, tb] = turns;
    let [pa, pb] = periods;
    [
        a + f64::from(ta) * pa.unwrap_or(0.0),
        b + f64::from(tb) * pb.unwrap_or(0.0),
    ]
}

/// The points around one loop, in loop order, without repeating the point
/// where it closes; each with the chord that runs from it to the next.
fn loop_points(
    body: &Body,
    edges: &[Vec<Vec3>],
    face: &Face,
    index: usize,
) -> Result<Vec<Vec<(Vec3, Chord)>>, TessError> {
    let mut loops = Vec::with_capacity(face.loops.len());
    for (number, loop_id) in face.loops.iter().enumerate() {
        let loop_ = body.loop_(*loop_id).ok_or_else(|| {
            TessError::InvalidBody(format!("face {index} uses a loop that does not exist"))
        })?;
        let mut points = Vec::new();
        let mut ends = Vec::with_capacity(loop_.coedges.len());
        for coedge in &loop_.coedges {
            let (Some(edge), Some(pair)) =
                (edges.get(coedge.edge.0 as usize), body.coedge_ends(*coedge))
            else {
                return Err(TessError::InvalidBody(format!(
                    "face {index} uses edge {}, which does not exist",
                    coedge.edge.0
                )));
            };
            ends.push(pair);
            // Each coedge contributes all its points but the last, which is
            // the first of the next coedge.
            let count = edge.len().saturating_sub(1);
            let chord = |k: usize| (coedge.edge.0, u32::try_from(k).unwrap_or(u32::MAX));
            if coedge.reversed {
                // Backwards, point `count − j` is followed by the chord
                // that ends there.
                let backwards = edge.iter().rev().take(count).enumerate();
                points.extend(backwards.map(|(j, p)| (*p, chord(count - 1 - j))));
            } else {
                let forwards = edge.iter().take(count).enumerate();
                points.extend(forwards.map(|(j, p)| (*p, chord(j))));
            }
        }
        let closed = ends
            .iter()
            .zip(ends.iter().cycle().skip(1))
            .all(|(this, next)| this.1 == next.0);
        if !closed || points.is_empty() {
            return fail(
                index,
                format!("its loop {number} is not a closed chain of edges"),
            );
        }
        loops.push(points);
    }
    Ok(loops)
}

/// Follows one loop through parameter space.
///
/// `sense` is +1 when the loops of this face run counter-clockwise in
/// `(a, b)` and −1 when clockwise.
fn build_ring(
    param: &Param,
    points: &[(Vec3, Chord)],
    sense: f64,
    face: usize,
    number: usize,
) -> Result<Ring, TessError> {
    let periods = param.periods();
    let domain = param.domain();
    let mut raw: Vec<Uv> = Vec::with_capacity(points.len());
    let mut sizes: Vec<[f64; 2]> = Vec::with_capacity(points.len());
    let mut largest = [0.0_f64; 2];
    for &(point, _) in points {
        let uv = param.project(point).map_err(|e| TessError::Face {
            face,
            reason: format!(
                "a point of its loop {number} cannot be projected onto its surface: {e}"
            ),
        })?;
        let [a, b] = uv;
        if !(a.is_finite() && b.is_finite()) {
            return fail(
                face,
                format!("a point of its loop {number} has no finite surface parameters"),
            );
        }
        let (_, [da, db]) = param.derivatives(uv);
        let [la, lb] = &mut largest;
        *la = la.max(da.length());
        *lb = lb.max(db.length());
        raw.push(uv);
        sizes.push([da.length(), db.length()]);
    }
    // A parameter is free where the surface does not move with it: there
    // the projection returned an arbitrary value for it.
    let free_at = |i: usize| -> Option<usize> {
        let [sa, sb] = sizes.get(i).copied().unwrap_or([1.0, 1.0]);
        let [la, lb] = largest;
        match (la > 0.0 && sa <= 1e-9 * la, lb > 0.0 && sb <= 1e-9 * lb) {
            (true, false) => Some(0),
            (false, true) => Some(1),
            _ => None,
        }
    };
    let n = points.len();
    let Some(start) = (0..n).find(|&i| free_at(i).is_none()) else {
        return fail(
            face,
            format!("every point of its loop {number} is a singular point of the surface"),
        );
    };
    let at = |i: usize| -> Option<(Uv, Vec3)> { Some((*raw.get(i)?, points.get(i)?.0)) };
    let chord_at = |i: usize| -> Option<Chord> { points.get(i).map(|p| p.1) };
    let Some((first, first_pos)) = at(start) else {
        return fail(face, "an empty loop");
    };
    let mut verts = vec![Bv {
        uv: first,
        pos: first_pos,
        free: None,
        chord: chord_at(start),
    }];
    let mut prev = first;
    for step in 1..n {
        let i = (start + step) % n;
        let Some((cur, pos)) = at(i) else { continue };
        match free_at(i) {
            None => {
                let uv = [
                    nearest(get2(cur, 0), get2(prev, 0), get2(periods, 0)),
                    nearest(get2(cur, 1), get2(prev, 1), get2(periods, 1)),
                ];
                verts.push(Bv {
                    uv,
                    pos,
                    free: None,
                    chord: chord_at(i),
                });
                prev = uv;
            }
            Some(k) => {
                // Arrive with the free parameter unchanged.
                let m = 1 - k;
                let mut uv = prev;
                set2(
                    &mut uv,
                    m,
                    nearest(get2(cur, m), get2(prev, m), get2(periods, m)),
                );
                verts.push(Bv {
                    uv,
                    pos,
                    free: Some(k),
                    chord: chord_at(i),
                });
                // Leave with the value of the next point. Around a closed
                // parameter there are two ways to get there; the face is
                // to the left of its boundary, which at the upper end of
                // the other parameter means running backwards.
                let j = (i + 1) % n;
                if let (None, Some((next, _))) = (free_at(j), at(j)) {
                    let target = get2(next, k);
                    let here = get2(uv, k);
                    let leave = match get2(periods, k) {
                        None => target,
                        Some(p) => {
                            let (low, high) = get2(domain, m);
                            let other = get2(uv, m);
                            let at_high = high.is_finite()
                                && (!low.is_finite() || (other - high).abs() < (other - low).abs());
                            let direction = if (k == 0) == at_high { -sense } else { sense };
                            let delta = (direction * (target - here)).rem_euclid(p);
                            if delta < 1e-9 * p || delta > p * (1.0 - 1e-9) {
                                here
                            } else {
                                here + direction * delta
                            }
                        }
                    };
                    if leave != here && leave.is_finite() {
                        set2(&mut uv, k, leave);
                        verts.push(Bv {
                            uv,
                            pos,
                            free: Some(k),
                            chord: chord_at(i),
                        });
                    }
                }
                prev = uv;
            }
        }
    }
    let mut winding = [0_i32; 2];
    for k in 0..2 {
        if let Some(p) = get2(periods, k) {
            let closing = nearest(get2(first, k), get2(prev, k), Some(p));
            let turns = ((closing - get2(first, k)) / p).round();
            if exceeds(turns.abs(), 1e6) {
                return fail(face, format!("its loop {number} is not finite"));
            }
            // In range by the check above.
            set2(&mut winding, k, turns as i32);
        }
    }
    Ok(Ring { verts, winding })
}

impl Ring {
    /// The far end of the edge that starts at vertex `i`.
    fn next(&self, i: usize, periods: [Option<f64>; 2]) -> Option<Bv> {
        if i + 1 < self.verts.len() {
            self.verts.get(i + 1).copied()
        } else {
            let mut v = *self.verts.first()?;
            v.uv = shifted(v.uv, self.winding, periods);
            Some(v)
        }
    }

    /// Twice the signed area enclosed (meaningful when it does not wind).
    fn twice_area(&self, periods: [Option<f64>; 2]) -> f64 {
        let Some(origin) = self.verts.first().map(|v| v.uv) else {
            return 0.0;
        };
        let [oa, ob] = origin;
        let mut sum = 0.0;
        for (i, v) in self.verts.iter().enumerate() {
            if let Some(w) = self.next(i, periods) {
                let ([a0, b0], [a1, b1]) = (v.uv, w.uv);
                sum += (a0 - oa) * (b1 - ob) - (a1 - oa) * (b0 - ob);
            }
        }
        sum
    }

    fn mean(&self, k: usize) -> f64 {
        let sum: f64 = self.verts.iter().map(|v| get2(v.uv, k)).sum();
        sum / self.verts.len().max(1) as f64
    }
}

/// A cut through a surface that closes on itself: the straight line in
/// parameter space between two boundary vertices, and its twin one period
/// further, which is the same curve in space.
#[derive(Clone, Copy)]
struct Seam {
    from: usize,
    to: usize,
    twin_from: usize,
    twin_to: usize,
    shift: Uv,
}

/// The boundary of a face in parameter space.
#[derive(Default)]
struct Region {
    verts: Vec<Bv>,
    segs: Vec<(usize, usize)>,
    /// Per segment, the chord of an edge it is, if any: seams and the
    /// stretches of a pole are not.
    chords: Vec<Option<Chord>>,
    seams: Vec<Seam>,
    /// What is wrong with the region as it stands, when that may be the
    /// doing of chords that are too coarse: a hole whose first point lies
    /// outside the polygon of the loop around it. The face is refused
    /// with this error unless finer chords make it go away.
    doubt: Option<TessError>,
}

impl Region {
    fn add(&mut self, v: Bv) -> usize {
        self.verts.push(v);
        self.verts.len() - 1
    }

    /// A segment from vertex `i` to vertex `j`.
    fn link(&mut self, i: usize, j: usize, chord: Option<Chord>) {
        self.segs.push((i, j));
        self.chords.push(chord);
    }

    /// A loop that does not wind, moved by whole periods.
    fn add_ring(&mut self, ring: &Ring, turns: [i32; 2], periods: [Option<f64>; 2]) {
        let base = self.verts.len();
        let n = ring.verts.len();
        for v in &ring.verts {
            self.verts.push(Bv {
                uv: shifted(v.uv, turns, periods),
                ..*v
            });
        }
        for (i, v) in ring.verts.iter().enumerate() {
            self.link(base + i, base + (i + 1) % n, v.chord);
        }
    }

    /// A loop that winds around parameter `a`, cut open at vertex `start`
    /// and moved by `turns` periods. Returns its ends: the vertex with the
    /// lower `a` first.
    fn add_chain(
        &mut self,
        ring: &Ring,
        start: usize,
        turns: [i32; 2],
        periods: [Option<f64>; 2],
    ) -> (usize, usize) {
        let base = self.verts.len();
        let n = ring.verts.len();
        let [wa, _] = ring.winding;
        let mut leaving: Option<Chord> = None;
        for i in 0..=n {
            let Some(v) = ring.verts.get((start + i) % n.max(1)) else {
                continue;
            };
            let [ta, tb] = turns;
            let wrapped = if start + i >= n { wa } else { 0 };
            self.verts.push(Bv {
                uv: shifted(v.uv, [ta + wrapped, tb], periods),
                ..*v
            });
            if i > 0 {
                self.link(base + i - 1, base + i, leaving);
            }
            leaving = v.chord;
        }
        if wa >= 0 {
            (base, base + n)
        } else {
            (base + n, base)
        }
    }

    fn uv(&self, i: usize) -> Option<Uv> {
        self.verts.get(i).map(|v| v.uv)
    }

    /// Whether `p` is enclosed by the segments and seams (even–odd).
    fn encloses(&self, p: Uv) -> bool {
        let [pa, pb] = p;
        let seam_ends = self
            .seams
            .iter()
            .flat_map(|s| [(s.from, s.to), (s.twin_from, s.twin_to)]);
        let mut inside = false;
        for (i, j) in self.segs.iter().copied().chain(seam_ends) {
            let (Some([a0, b0]), Some([a1, b1])) = (self.uv(i), self.uv(j)) else {
                continue;
            };
            if (b0 > pb) != (b1 > pb) {
                let a = a0 + (pb - b0) / (b1 - b0) * (a1 - a0);
                if a > pa {
                    inside = !inside;
                }
            }
        }
        inside
    }
}

/// True when the seam from `a` to `b` cannot be used next to the
/// boundary segment (c, d): they cross, or one passes through an end of
/// the other. Meeting end to end is fine: a seam starts and ends on
/// boundary vertices. (Also what two segments of the boundary may not do
/// to each other, see `Boundary::chords_to_refine`.)
fn seam_blocked(a: Uv, b: Uv, c: Uv, d: Uv) -> bool {
    let (ta, tb, tc, td) = (tuple(a), tuple(b), tuple(c), tuple(d));
    let (side_c, side_d) = (orient(ta, tb, tc), orient(ta, tb, td));
    let (side_a, side_b) = (orient(tc, td, ta), orient(tc, td, tb));
    if side_c * side_d < 0.0 && side_a * side_b < 0.0 {
        return true;
    }
    // `p` on the line through (q, r), strictly between them.
    let within = |p: P2, q: P2, r: P2| {
        let along = (p.0 - q.0) * (r.0 - q.0) + (p.1 - q.1) * (r.1 - q.1);
        let whole = (r.0 - q.0) * (r.0 - q.0) + (r.1 - q.1) * (r.1 - q.1);
        p != q && p != r && along > 0.0 && along < whole
    };
    (side_c == 0.0 && within(tc, ta, tb))
        || (side_d == 0.0 && within(td, ta, tb))
        || (side_a == 0.0 && within(ta, tc, td))
        || (side_b == 0.0 && within(tb, tc, td))
}

/// Moves a loop that does not wind by whole periods until `probe` says
/// its first proper vertex is where it belongs.
fn place(ring: &Ring, periods: [Option<f64>; 2], probe: impl Fn(Uv) -> bool) -> Option<[i32; 2]> {
    let vertex = ring
        .verts
        .iter()
        .find(|v| v.free.is_none())
        .or(ring.verts.first())?;
    place_by(vertex, periods, probe)
}

/// A hole that [`place`] finds no place for: where it must be if any of
/// its vertices is where it belongs (the others are then outside the
/// polygon around it, which chords too coarse can do), and where it was
/// found otherwise.
fn place_in_doubt(ring: &Ring, periods: [Option<f64>; 2], probe: impl Fn(Uv) -> bool) -> [i32; 2] {
    ring.verts
        .iter()
        .filter(|v| v.free.is_none())
        .find_map(|v| place_by(v, periods, &probe))
        .unwrap_or([0, 0])
}

fn place_by(
    vertex: &Bv,
    periods: [Option<f64>; 2],
    probe: impl Fn(Uv) -> bool,
) -> Option<[i32; 2]> {
    let range = |period: Option<f64>| if period.is_some() { -2..=2 } else { 0..=0 };
    // Nearest first, so that a loop already in place stays there.
    let mut candidates: Vec<[i32; 2]> = Vec::new();
    for ta in range(get2(periods, 0)) {
        for tb in range(get2(periods, 1)) {
            candidates.push([ta, tb]);
        }
    }
    candidates.sort_by_key(|[ta, tb]| (ta.abs() + tb.abs(), *ta, *tb));
    candidates
        .into_iter()
        .find(|turns| probe(shifted(vertex.uv, *turns, periods)))
}

/// The region of a face whose loops do not wind: an outer loop and holes.
fn patch_region(
    param: &Param,
    rings: &[Ring],
    sense: f64,
    face: usize,
) -> Result<Region, TessError> {
    let periods = param.periods();
    let mut region = Region::default();
    let areas: Vec<f64> = rings.iter().map(|r| r.twice_area(periods)).collect();
    let total: f64 = areas.iter().sum();
    if !positive(total * sense) {
        return fail(
            face,
            "its loops do not bound a region: they enclose no area, or run the wrong way \
             round (the face would be everything outside them)",
        );
    }
    let outer = areas
        .iter()
        .enumerate()
        .max_by(|x, y| x.1.abs().total_cmp(&y.1.abs()).then(y.0.cmp(&x.0)))
        .map_or(0, |(i, _)| i);
    if let Some(ring) = rings.get(outer) {
        region.add_ring(ring, [0, 0], periods);
    }
    // Every other loop is a hole in the outer one. On a closed surface
    // the projection may have put it a whole period away.
    let mut outline = Region::default();
    outline.verts.clone_from(&region.verts);
    outline.segs.clone_from(&region.segs);
    outline.chords.clone_from(&region.chords);
    for (i, ring) in rings.iter().enumerate() {
        if i == outer {
            continue;
        }
        let turns = place(ring, periods, |p| outline.encloses(p)).unwrap_or_else(|| {
            region.doubt.get_or_insert(TessError::Face {
                face,
                reason: format!(
                    "its loops do not bound a region: loop {i} lies outside the loop that \
                     should contain it"
                ),
            });
            place_in_doubt(ring, periods, |p| outline.encloses(p))
        });
        region.add_ring(ring, turns, periods);
    }
    Ok(region)
}

/// The point a whole end of the parameter range collapses to, when it
/// does: a pole of a sphere, the apex of a cone.
fn pole(param: &Param, a: f64, b: f64, typical: f64, face: usize) -> Result<Bv, TessError> {
    if !b.is_finite() {
        return fail(
            face,
            "it is not bounded: a loop winds around the axis and the surface has no end on \
             that side",
        );
    }
    let (pos, [da, _]) = param.derivatives([a, b]);
    if !(pos.is_finite() && da.length() <= 1e-9 * typical) {
        return fail(
            face,
            "it is not bounded: a loop winds around the axis and the surface does not close \
             to a point on that side",
        );
    }
    Ok(Bv {
        uv: [a, b],
        pos,
        free: Some(0),
        chord: None,
    })
}

/// How many places are tried for the seam of a face that goes around its
/// surface.
const SEAM_TRIES: usize = 16;

/// Whether the surface closes all the way around parameter `a` and to a
/// point at both ends of `b`, as a sphere does. It is then a closed
/// surface, on which loops that run clockwise bound a face as well:
/// everything outside them.
fn closes_to_points(param: &Param) -> bool {
    let [Some(_), None] = param.periods() else {
        return false;
    };
    let [(a_low, _), (b_low, b_high)] = param.domain();
    if !(a_low.is_finite() && b_low.is_finite() && b_high.is_finite()) {
        return false;
    }
    let speed = |b: f64| {
        let (_, [da, _]) = param.derivatives([a_low, b]);
        da.length()
    };
    let typical = speed(0.5 * (b_low + b_high));
    positive(typical)
        && typical.is_finite()
        && speed(b_low) <= 1e-9 * typical
        && speed(b_high) <= 1e-9 * typical
}

/// The region of a face that goes all the way around parameter `a`:
/// between two loops that wind around it, or between one and the end of
/// the surface, or (without loops that wind) the whole surface; less the
/// holes its other loops bound.
fn band_region(
    param: &Param,
    rings: &[Ring],
    sense: f64,
    face: usize,
) -> Result<Region, TessError> {
    let periods = param.periods();
    let [Some(period), period_b] = periods else {
        return fail(face, "a face without loops on a surface that is not closed");
    };
    let [(a_low, _), (b_low, b_high)] = param.domain();
    // How fast the surface moves with `a` where it is not singular.
    let mut typical = 0.0_f64;

    // The face lies to the left of its loops: above (towards +b) one that
    // runs counter-clockwise around the axis, below one that runs
    // clockwise.
    let mut lower: Option<&Ring> = None;
    let mut upper: Option<&Ring> = None;
    let mut holes: Vec<(usize, &Ring)> = Vec::new();
    for (i, ring) in rings.iter().enumerate() {
        let [wa, _] = ring.winding;
        let slot = match f64::from(wa) * sense {
            t if t > 0.0 => &mut lower,
            t if t < 0.0 => &mut upper,
            _ => {
                holes.push((i, ring));
                continue;
            }
        };
        if slot.is_some() {
            return fail(
                face,
                "its loops do not bound a region: two of them wind around the axis the same \
                 way",
            );
        }
        *slot = Some(ring);
    }
    for ring in lower.iter().chain(upper.iter()) {
        for v in &ring.verts {
            let (_, [da, _]) = param.derivatives(v.uv);
            typical = typical.max(da.length());
        }
    }
    if lower.is_none() && upper.is_none() && b_low.is_finite() && b_high.is_finite() {
        let (_, [da, _]) = param.derivatives([a_low, 0.5 * (b_low + b_high)]);
        typical = da.length();
    }
    if !(typical > 0.0 && typical.is_finite()) {
        typical = 1.0;
    }

    // The upper loop, moved up by whole periods of `b` to lie above the
    // lower one (only a torus has such a period).
    let mut upper_turns_b = 0_i32;
    if let (Some(low), Some(up)) = (lower, upper) {
        let gap = up.mean(1) - low.mean(1);
        match period_b {
            Some(pb) => {
                let turns = (-gap / pb).floor() + 1.0;
                if exceeds(turns.abs(), 1e6) {
                    return fail(face, "its loops are not finite");
                }
                // In range by the check above.
                upper_turns_b = if gap > 0.0 { 0 } else { turns as i32 };
            }
            None => {
                if !positive(gap) {
                    return fail(
                        face,
                        "its loops do not bound a region: the face would lie outside both \
                         of the loops that wind around the axis",
                    );
                }
            }
        }
    }
    if lower.is_none() && upper.is_none() && period_b.is_some() {
        return torus_region(param, period, period_b.unwrap_or(period));
    }

    // The seam: from a vertex of one winding loop to the vertex of the
    // other that is nearest in `a`, or straight to the end of the surface.
    // It must not cross any loop; a few starting vertices are tried.
    struct Cut {
        a: f64,
        lower_start: Option<usize>,
        upper_start: Option<(usize, i32)>,
        ends: [Uv; 2],
    }
    let left_a = |ring: &Ring, s: usize| -> Option<f64> {
        let [a, _] = ring.verts.get(s)?.uv;
        let [wa, _] = ring.winding;
        Some(if wa < 0 { a - period } else { a })
    };
    let nearest_vertex = |ring: &Ring, a: f64| -> Option<(usize, i32)> {
        let mut best: Option<(f64, usize, i32)> = None;
        for (j, v) in ring.verts.iter().enumerate() {
            if v.free.is_some() {
                continue;
            }
            let natural = left_a(ring, j)?;
            let turns = ((a - natural) / period).round();
            let distance = (natural + turns * period - a).abs();
            if turns.abs() <= 1e6 && best.is_none_or(|(d, _, _)| distance < d) {
                // In range by the check above.
                best = Some((distance, j, turns as i32));
            }
        }
        best.map(|(_, j, turns)| (j, turns))
    };
    let crosses_a_loop = |from: Uv, to: Uv| -> bool {
        for ring in rings {
            for (i, v) in ring.verts.iter().enumerate() {
                let Some(w) = ring.next(i, periods) else {
                    continue;
                };
                let around = ((get2(from, 0) - get2(v.uv, 0)) / period).round();
                if exceeds(around.abs(), 1e6) {
                    return true;
                }
                // In range by the check above.
                let around = around as i32;
                let b_turns = if period_b.is_some() { -2..=2 } else { 0..=0 };
                for tb in b_turns {
                    for ta in around - 1..=around + 1 {
                        let p = shifted(v.uv, [ta, tb], periods);
                        let q = shifted(w.uv, [ta, tb], periods);
                        if seam_blocked(from, to, p, q) {
                            return true;
                        }
                    }
                }
            }
        }
        false
    };
    // Where a winding loop is missing, the surface must end in a point.
    if lower.is_none() {
        pole(param, a_low, b_low, typical, face)?;
    }
    if upper.is_none() {
        pole(param, a_low, b_high, typical, face)?;
    }
    let anchor = lower.or(upper);
    let mut cut: Option<Cut> = None;
    match anchor {
        None => {
            // From one end of the surface to the other, along the first
            // of a few meridians that no hole lies on. Without holes that
            // is the surface's own seam.
            let tries = if holes.is_empty() { 1 } else { SEAM_TRIES };
            for c in 0..tries {
                let a = a_low + period * c as f64 / tries as f64;
                let ends = [[a, b_low], [a, b_high]];
                let [from, to] = ends;
                if from.iter().chain(&to).all(|x| x.is_finite()) && !crosses_a_loop(from, to) {
                    cut = Some(Cut {
                        a,
                        lower_start: None,
                        upper_start: None,
                        ends,
                    });
                    break;
                }
            }
        }
        Some(anchor) => {
            let n = anchor.verts.len();
            let tries = n.min(SEAM_TRIES);
            for c in 0..tries {
                let s = c * n / tries;
                let Some(v) = anchor.verts.get(s).filter(|v| v.free.is_none()) else {
                    continue;
                };
                let Some(a) = left_a(anchor, s) else { continue };
                let [_, b] = v.uv;
                let candidate = if lower.is_some() {
                    match upper {
                        Some(up) => {
                            let Some((j, turns)) = nearest_vertex(up, a) else {
                                continue;
                            };
                            let (Some(end_a), Some(end)) = (left_a(up, j), up.verts.get(j)) else {
                                continue;
                            };
                            let [_, end_b] = shifted(end.uv, [0, upper_turns_b], periods);
                            Cut {
                                a,
                                lower_start: Some(s),
                                upper_start: Some((j, turns)),
                                ends: [[a, b], [end_a + f64::from(turns) * period, end_b]],
                            }
                        }
                        None => Cut {
                            a,
                            lower_start: Some(s),
                            upper_start: None,
                            ends: [[a, b], [a, b_high]],
                        },
                    }
                } else {
                    Cut {
                        a,
                        lower_start: None,
                        upper_start: Some((s, 0)),
                        ends: [[a, b_low], [a, b]],
                    }
                };
                let [from, to] = candidate.ends;
                if from.iter().chain(&to).all(|x| x.is_finite()) && !crosses_a_loop(from, to) {
                    cut = Some(candidate);
                    break;
                }
            }
        }
    }
    let Some(cut) = cut else {
        return unsupported(
            face,
            "a face around the axis of its surface with no place for a straight seam between \
             its loops",
        );
    };

    let mut region = Region::default();
    let (lower_left, lower_right) = match (lower, cut.lower_start) {
        (Some(ring), Some(start)) => region.add_chain(ring, start, [0, 0], periods),
        _ => {
            let left = pole(param, cut.a, b_low, typical, face)?;
            let right = Bv {
                uv: [cut.a + period, b_low],
                ..left
            };
            let ends = (region.add(left), region.add(right));
            region.link(ends.0, ends.1, None);
            ends
        }
    };
    let (upper_left, upper_right) = match (upper, cut.upper_start) {
        (Some(ring), Some((start, turns))) => {
            region.add_chain(ring, start, [turns, upper_turns_b], periods)
        }
        _ => {
            let left = pole(param, cut.a, b_high, typical, face)?;
            let right = Bv {
                uv: [cut.a + period, b_high],
                ..left
            };
            let ends = (region.add(left), region.add(right));
            region.link(ends.0, ends.1, None);
            ends
        }
    };
    region.seams.push(Seam {
        from: lower_left,
        to: upper_left,
        twin_from: lower_right,
        twin_to: upper_right,
        shift: [period, 0.0],
    });
    let mut outline = Region::default();
    outline.verts.clone_from(&region.verts);
    outline.segs.clone_from(&region.segs);
    outline.chords.clone_from(&region.chords);
    outline.seams.clone_from(&region.seams);
    for (i, ring) in holes {
        let turns = place(ring, periods, |p| outline.encloses(p)).unwrap_or_else(|| {
            region.doubt.get_or_insert(TessError::Face {
                face,
                reason: format!(
                    "its loops do not bound a region: loop {i} lies outside the region its \
                     other loops bound"
                ),
            });
            place_in_doubt(ring, periods, |p| outline.encloses(p))
        });
        region.add_ring(ring, turns, periods);
    }
    Ok(region)
}

/// A whole torus: a rectangle of one period by one period, cut along both
/// seams. All four corners are one point.
fn torus_region(param: &Param, period_a: f64, period_b: f64) -> Result<Region, TessError> {
    let [(a, _), (b, _)] = param.domain();
    let corner = |uv: Uv| Bv {
        uv,
        pos: param.eval([a, b]),
        free: None,
        chord: None,
    };
    let mut region = Region::default();
    let c00 = region.add(corner([a, b]));
    let c10 = region.add(corner([a + period_a, b]));
    let c01 = region.add(corner([a, b + period_b]));
    let c11 = region.add(corner([a + period_a, b + period_b]));
    region.seams.push(Seam {
        from: c00,
        to: c01,
        twin_from: c10,
        twin_to: c11,
        shift: [period_a, 0.0],
    });
    region.seams.push(Seam {
        from: c00,
        to: c10,
        twin_from: c01,
        twin_to: c11,
        shift: [0.0, period_b],
    });
    Ok(region)
}

/// Grid cells for a turn of `sweep` radians at `radius`, as for an arc
/// but never more than a quarter turn each: a closed surface must keep
/// some volume however coarse the tolerance.
fn arc_cells(
    radius: f64,
    sweep: f64,
    tolerance: f64,
    params: &TessParams,
    face: usize,
) -> Result<u32, TessError> {
    let least = (sweep.abs() / FRAC_PI_2 - 1e-9).ceil().max(1.0);
    if exceeds(least, f64::from(params.max_segments) * 8.0) {
        return Err(TessError::TooFine {
            patch: face,
            needed_segments: least,
            max_segments: params.max_segments,
        });
    }
    if !positive(radius) {
        // In range by the check above.
        return Ok(least as u32);
    }
    let with_tolerance = TessParams {
        chordal_tolerance: tolerance,
        ..*params
    };
    let cells = arc_segments(radius, sweep, &with_tolerance).map_err(|e| match e {
        TessError::TooFine {
            needed_segments,
            max_segments,
            ..
        } => TessError::TooFine {
            patch: face,
            needed_segments,
            max_segments,
        },
        other => other,
    })?;
    // In range by the check above.
    Ok(cells.max(least as u32))
}

/// How many grid cells the box `low..high` of the surface's own `(u, v)`
/// needs in each direction for a first guess at the tolerance. Only a
/// guess: every triangle is measured afterwards.
fn seed_cells(
    surface: &Surface,
    low: (f64, f64),
    high: (f64, f64),
    params: &TessParams,
    face: usize,
) -> Result<(u32, u32), TessError> {
    let tolerance = params.chordal_tolerance;
    let (span_u, span_v) = (high.0 - low.0, high.1 - low.1);
    // On a surface curved in both directions a grid cell sags in u and in
    // v at once, and inside a triangle the two add up. Giving each
    // direction half the tolerance keeps their sum within it.
    let half = tolerance / 2.0;
    let from_second_derivatives = |muu: f64, muv: f64, mvv: f64| -> Result<(u32, u32), TessError> {
        // Filip, Magedson & Markot, "Surface algorithms using bounds on
        // derivatives" (CAGD 3, 1986): linear interpolation over a right
        // triangle with legs `h` in parameter space deviates by at most
        // `(Muu + 2 Muv + Mvv) h² / 8`.
        let per_unit = ((muu + 2.0 * muv + mvv) / (8.0 * tolerance)).sqrt();
        let (nu, nv) = ((per_unit * span_u).ceil(), (per_unit * span_v).ceil());
        let limit = f64::from(params.max_segments);
        if !(nu <= limit && nv <= limit) {
            return Err(TessError::TooFine {
                patch: face,
                needed_segments: nu.max(nv),
                max_segments: params.max_segments,
            });
        }
        // In range by the check above.
        Ok(((nu as u32).max(1), (nv as u32).max(1)))
    };
    match surface {
        Surface::Plane(_) => Ok((1, 1)),
        Surface::Cylinder(c) => Ok((arc_cells(c.radius(), span_u, tolerance, params, face)?, 1)),
        Surface::Cone(c) => {
            let slope = c.half_angle().tan();
            let radius = (c.radius() + low.1 * slope)
                .abs()
                .max((c.radius() + high.1 * slope).abs());
            Ok((arc_cells(radius, span_u, tolerance, params, face)?, 1))
        }
        Surface::Sphere(s) => {
            let widest = if low.1 <= 0.0 && high.1 >= 0.0 {
                1.0
            } else {
                low.1.cos().max(high.1.cos()).max(0.0)
            };
            Ok((
                arc_cells(s.radius() * widest, span_u, half, params, face)?,
                arc_cells(s.radius(), span_v, half, params, face)?,
            ))
        }
        Surface::Torus(t) => {
            let (major, minor) = t.radii();
            Ok((
                arc_cells(major + minor, span_u, half, params, face)?,
                arc_cells(minor, span_v, half, params, face)?,
            ))
        }
        Surface::Bezier(b) => {
            let (muu, muv, mvv) = b.second_derivative_bounds();
            from_second_derivatives(muu, muv, mvv)
        }
        Surface::Nurbs(_) => {
            // No bound on the derivatives of a rational surface at hand:
            // second differences on a lattice, with a margin.
            const SAMPLES: u32 = 8;
            let (hu, hv) = (
                span_u / f64::from(2 * SAMPLES),
                span_v / f64::from(2 * SAMPLES),
            );
            let (mut muu, mut muv, mut mvv) = (0.0_f64, 0.0_f64, 0.0_f64);
            for i in 0..SAMPLES {
                for j in 0..SAMPLES {
                    let u = low.0 + hu * f64::from(2 * i + 1);
                    let v = low.1 + hv * f64::from(2 * j + 1);
                    let s = |du: f64, dv: f64| surface.eval(u + du * hu, v + dv * hv);
                    let centre = s(0.0, 0.0);
                    muu = muu.max((s(1.0, 0.0) + s(-1.0, 0.0) - centre * 2.0).length() / (hu * hu));
                    mvv = mvv.max((s(0.0, 1.0) + s(0.0, -1.0) - centre * 2.0).length() / (hv * hv));
                    let mixed = s(1.0, 1.0) - s(1.0, -1.0) - s(-1.0, 1.0) + s(-1.0, -1.0);
                    muv = muv.max(mixed.length() / (4.0 * hu * hv));
                }
            }
            from_second_derivatives(1.5 * muu, 1.5 * muv, 1.5 * mvv)
        }
    }
}

/// How far `target` is from the surface, starting the search at `uv`.
/// Never more than the distance to the surface point at `uv` itself.
fn distance_to_surface(param: &Param, uv: Uv, target: Vec3) -> f64 {
    let at_uv = param.eval(uv).distance(target);
    let found = if param.is_freeform() {
        // A few Gauss–Newton steps on |S(u, v) − target|² from the guess.
        let [(a0, a1), (b0, b1)] = param.domain();
        let [mut a, mut b] = uv;
        let mut best = at_uv;
        for _ in 0..8 {
            let (s, [da, db]) = param.derivatives([a, b]);
            let r = target - s;
            let (e, f, g) = (da.dot(da), da.dot(db), db.dot(db));
            let det = e * g - f * f;
            if !(det.is_finite() && det > 1e-14 * e * g) {
                break;
            }
            let (ra, rb) = (r.dot(da), r.dot(db));
            let next_a = (a + (g * ra - f * rb) / det).clamp(a0, a1);
            let next_b = (b + (e * rb - f * ra) / det).clamp(b0, b1);
            let distance = param.eval([next_a, next_b]).distance(target);
            if !positive(best - distance) {
                break;
            }
            (a, b, best) = (next_a, next_b, distance);
        }
        best
    } else {
        // Closed form for the analytic kinds.
        match param.surface.project(target) {
            Ok((u, v)) => param.surface.eval(u, v).distance(target),
            Err(_) => at_uv,
        }
    };
    if found < at_uv { found } else { at_uv }
}

/// What to do with a triangle that is not good enough.
enum Action {
    Keep,
    SplitEdge(usize),
    SplitTriangle,
}

/// The triangulation of one face while it is refined.
struct Work<'a> {
    param: Param<'a>,
    cdt: Cdt,
    /// Per triangulation vertex (after the bounding square's four).
    uv: Vec<Uv>,
    pos: Vec<Vec3>,
    free: Vec<Option<usize>>,
    /// Parameters are scaled about `centre` for the triangulation.
    centre: Uv,
    scale: Uv,
    tolerance: f64,
}

impl Work<'_> {
    fn plane(&self, uv: Uv) -> P2 {
        to_plane(uv, self.centre, self.scale)
    }

    fn vertex(&self, v: u32) -> Option<(Uv, Vec3, Option<usize>)> {
        let i = v.checked_sub(FIRST_VERTEX)? as usize;
        Some((*self.uv.get(i)?, *self.pos.get(i)?, *self.free.get(i)?))
    }

    /// Records the data of a vertex the triangulation has just made.
    fn record(&mut self, v: u32, uv: Uv, pos: Vec3, free: Option<usize>) -> Result<(), CdtError> {
        if v.checked_sub(FIRST_VERTEX).map(|i| i as usize) != Some(self.uv.len()) {
            return Err(CdtError::Internal);
        }
        self.uv.push(uv);
        self.pos.push(pos);
        self.free.push(free);
        Ok(())
    }

    /// The mean parameters of some vertices, leaving out for each
    /// parameter the vertices where it is free: a pole is at every
    /// longitude, so it takes the longitude of its neighbours.
    fn mean(corners: &[(Uv, Vec3, Option<usize>)]) -> Uv {
        let mut out = [0.0; 2];
        for k in 0..2 {
            let (mut fixed, mut fixed_count, mut all) = (0.0, 0.0, 0.0);
            for c in corners {
                let x = get2(c.0, k);
                all += x;
                if c.2 != Some(k) {
                    fixed += x;
                    fixed_count += 1.0;
                }
            }
            let value = if fixed_count > 0.0 {
                fixed / fixed_count
            } else {
                all / (corners.len().max(1) as f64)
            };
            set2(&mut out, k, value);
        }
        out
    }

    /// How far `target`, a point of a chord or a triangle, is from the
    /// surface. The surface point at the mean parameters `uv` is the
    /// first candidate. Only when that is too far is the nearest surface
    /// point looked for: the mean parameters are not where the surface is
    /// nearest when the parametrisation is far from uniform (a chord
    /// across a cone runs along a spiral in parameter space) or when the
    /// triangle lies along a boundary that is curved in parameter space.
    fn off_surface(&self, uv: Uv, target: Vec3) -> f64 {
        let off = self.param.eval(uv).distance(target);
        if off <= self.tolerance {
            off
        } else {
            distance_to_surface(&self.param, uv, target)
        }
    }

    /// Measures triangle `t` (see the module documentation).
    fn assess(&self, t: usize) -> Action {
        let Some(view) = self.cdt.triangle(t).filter(|view| view.inside) else {
            return Action::Keep;
        };
        let [Some(a), Some(b), Some(c)] = view.v.map(|v| self.vertex(v)) else {
            return Action::Keep;
        };
        if a.1 == b.1 || b.1 == c.1 || c.1 == a.1 {
            return Action::Keep; // Collapsed at a pole: dropped later.
        }
        let corners = [a, b, c];
        let pair = |i: usize| -> [(Uv, Vec3, Option<usize>); 2] {
            let [x, y, z] = corners;
            match i % 3 {
                0 => [y, z],
                1 => [z, x],
                _ => [x, y],
            }
        };
        // Free edges: the surface at the edge's mean parameters against
        // the chord's midpoint.
        let mut worst: Option<(f64, usize)> = None;
        let [c0, c1, c2] = view.constrained;
        for (i, constrained) in [c0, c1, c2].into_iter().enumerate() {
            if constrained {
                continue;
            }
            let ends = pair(i);
            let [p, q] = ends;
            let off = self.off_surface(Self::mean(&ends), p.1.lerp(q.1, 0.5));
            // NaN counts as failing: it ends in an error, not a bad mesh.
            if exceeds(off, self.tolerance) && worst.is_none_or(|(w, _)| exceeds(off, w)) {
                worst = Some((off, i));
            }
        }
        if let Some((_, i)) = worst {
            return Action::SplitEdge(i);
        }
        let middle = Self::mean(&corners);
        let centroid = (a.1 + b.1 + c.1) * (1.0 / 3.0);
        let off = self.off_surface(middle, centroid);
        if exceeds(off, self.tolerance) {
            return Action::SplitTriangle;
        }
        // Facing the way the surface does. The surface normal at the mean
        // parameters decides when it is clear; otherwise the normal at the
        // nearest surface point does.
        let facing = (b.1 - a.1).cross(c.1 - a.1);
        let (_, [da, db]) = self.param.derivatives(middle);
        let reference = da.cross(db);
        let agreement = facing.dot(reference);
        let clear = agreement > 0.5 * facing.length() * reference.length();
        if !clear {
            let exact = if self.param.is_freeform() {
                (reference.length() > 0.0).then_some(agreement)
            } else {
                self.param
                    .surface
                    .project(centroid)
                    .ok()
                    .and_then(|(u, v)| self.param.surface.normal(u, v))
                    .map(|n| facing.dot(n) * self.param.handedness())
            };
            if exact.is_some_and(|x| !positive(x)) {
                // The longest free edge, in the scaled parameter plane.
                let mut longest: Option<(f64, usize)> = None;
                for (i, constrained) in [c0, c1, c2].into_iter().enumerate() {
                    if constrained {
                        continue;
                    }
                    let [p, q] = pair(i);
                    let (p, q) = (self.plane(p.0), self.plane(q.0));
                    let length = (p.0 - q.0).hypot(p.1 - q.1);
                    if longest.is_none_or(|(l, _)| length > l) {
                        longest = Some((length, i));
                    }
                }
                return match longest {
                    Some((_, i)) => Action::SplitEdge(i),
                    None => Action::SplitTriangle,
                };
            }
        }
        Action::Keep
    }

    /// Adds a point in the middle of edge `i` of triangle `t`.
    fn split_edge(&mut self, t: usize, i: usize) -> Result<(), CdtError> {
        let view = self.cdt.triangle(t).ok_or(CdtError::Internal)?;
        let [x, y, z] = view.v;
        let (p, q) = match i % 3 {
            0 => (y, z),
            1 => (z, x),
            _ => (x, y),
        };
        let (Some(p), Some(q)) = (self.vertex(p), self.vertex(q)) else {
            return Err(CdtError::Internal);
        };
        let ([pa, pb], [qa, qb]) = (p.0, q.0);
        let uv = [0.5 * (pa + qa), 0.5 * (pb + qb)];
        let v = self.cdt.split_edge(t as u32, i, self.plane(uv))?;
        self.record(v, uv, self.param.eval(uv), None)
    }

    /// Adds a point at the centroid of triangle `t`.
    fn split_triangle(&mut self, t: usize) -> Result<(), CdtError> {
        let view = self.cdt.triangle(t).ok_or(CdtError::Internal)?;
        let [Some(a), Some(b), Some(c)] = view.v.map(|v| self.vertex(v)) else {
            return Err(CdtError::Internal);
        };
        let ([a0, a1], [b0, b1], [c0, c1]) = (a.0, b.0, c.0);
        let uv = [(a0 + b0 + c0) / 3.0, (a1 + b1 + c1) / 3.0];
        let v = self.cdt.split_triangle(t as u32, self.plane(uv))?;
        self.record(v, uv, self.param.eval(uv), None)
    }
}

/// Cuts every stretch of the boundary that is one point in space (two
/// neighbouring pole vertices) into grid-sized steps, and gives every seam
/// its points: the same 3D points on both sides of the cut.
fn finish_boundary(
    param: &Param,
    region: &mut Region,
    step: Uv,
    tolerance: f64,
    limit: usize,
    face: usize,
) -> Result<(), TessError> {
    let too_many = || TessError::TooFine {
        patch: face,
        needed_segments: limit as f64,
        max_segments: 0,
    };
    let old: Vec<(usize, usize)> = std::mem::take(&mut region.segs);
    let old_chords: Vec<Option<Chord>> = std::mem::take(&mut region.chords);
    for (n, (i, j)) in old.into_iter().enumerate() {
        let chord = old_chords.get(n).copied().flatten();
        let (Some(&p), Some(&q)) = (region.verts.get(i), region.verts.get(j)) else {
            continue;
        };
        let (pieces, pole) = match (p.free, q.free) {
            (Some(k), Some(l)) if k == l && p.pos == q.pos => {
                let length = (get2(q.uv, k) - get2(p.uv, k)).abs();
                ((length / get2(step, k) - 1e-9).ceil(), true)
            }
            _ => (1.0, false),
        };
        if exceeds(pieces, limit as f64) {
            return Err(too_many());
        }
        // In range by the check above.
        let pieces = (pieces as usize).max(1);
        let mut last = i;
        for n in 1..pieces {
            let s = n as f64 / pieces as f64;
            let ([pa, pb], [qa, qb]) = (p.uv, q.uv);
            let next = region.add(Bv {
                uv: [pa + (qa - pa) * s, pb + (qb - pb) * s],
                chord: None,
                ..p
            });
            region.link(last, next, None);
            last = next;
        }
        // A stretch of a pole is one point in space, not a chord.
        region.link(last, j, if pole { None } else { chord });
    }
    for seam in region.seams.clone() {
        let (Some(&from), Some(&to)) = (region.verts.get(seam.from), region.verts.get(seam.to))
        else {
            continue;
        };
        let ([fa, fb], [ta, tb]) = (from.uv, to.uv);
        let [sa, sb] = step;
        let pieces = ((ta - fa).abs() / sa).max((tb - fb).abs() / sb);
        let pieces = (pieces - 1e-9).ceil();
        if exceeds(pieces, limit as f64) {
            return Err(too_many());
        }
        // In range by the check above.
        let pieces = (pieces as usize).max(1);
        let at = |s: f64| -> (f64, Uv, Vec3) {
            let uv = [fa + (ta - fa) * s, fb + (tb - fb) * s];
            (s, uv, param.eval(uv))
        };
        let mut line: Vec<(f64, Uv, Vec3)> = vec![(0.0, from.uv, from.pos)];
        line.extend((1..pieces).map(|n| at(n as f64 / pieces as f64)));
        line.push((1.0, to.uv, to.pos));
        // Halve the pieces whose chord is not within the tolerance of the
        // seam curve. Half the tolerance, as for the grid: a seam mostly
        // runs across a doubly curved surface.
        if !param.is_plane() {
            for _ in 0..24 {
                let mut refined = Vec::with_capacity(line.len());
                let mut changed = false;
                for pair in line.windows(2) {
                    let [p, q] = pair else { continue };
                    refined.push(*p);
                    let middle = at(0.5 * (p.0 + q.0));
                    if middle.2.distance(p.2.lerp(q.2, 0.5)) > tolerance / 2.0 {
                        refined.push(middle);
                        changed = true;
                    }
                }
                if let Some(last) = line.last() {
                    refined.push(*last);
                }
                line = refined;
                if line.len() > limit {
                    return Err(too_many());
                }
                if !changed {
                    break;
                }
            }
        }
        let (mut last, mut twin_last) = (seam.from, seam.twin_from);
        for &(_, uv, pos) in line.iter().skip(1).take(line.len().saturating_sub(2)) {
            let ([a, b], [da, db]) = (uv, seam.shift);
            let next = region.add(Bv {
                uv,
                pos,
                free: None,
                chord: None,
            });
            let twin_next = region.add(Bv {
                uv: [a + da, b + db],
                pos,
                free: None,
                chord: None,
            });
            region.link(last, next, None);
            region.link(twin_last, twin_next, None);
            (last, twin_last) = (next, twin_next);
        }
        region.link(last, seam.to, None);
        region.link(twin_last, seam.twin_to, None);
    }
    Ok(())
}

/// The grid nodes `(i, j)`, `0 < i < columns`, `0 < j < rows`, coarse
/// levels first: points arrive spread out, as a Delaunay triangulation
/// likes them, and each next to one that is already there.
fn grid_order(columns: u32, rows: u32) -> Vec<(u32, u32)> {
    let mut order = Vec::new();
    let mut stride = columns.max(rows).next_power_of_two();
    let top = stride;
    while stride >= 1 {
        let mut reverse = false;
        let mut j = stride;
        while j < rows {
            let mut row = Vec::new();
            let mut i = stride;
            while i < columns {
                let coarser =
                    stride < top && i.is_multiple_of(2 * stride) && j.is_multiple_of(2 * stride);
                if !coarser {
                    row.push((i, j));
                }
                i += stride;
            }
            if reverse {
                row.reverse();
            }
            reverse = !reverse;
            order.extend(row);
            j += stride;
        }
        stride /= 2;
    }
    order
}

/// The boundary of a face, ready to be triangulated: its region in
/// parameter space, and the grid and the scales that go with it.
pub(crate) struct Boundary<'a> {
    param: Param<'a>,
    region: Region,
    low: Uv,
    high: Uv,
    cells: [u32; 2],
    step: Uv,
    scale: Uv,
    centre: Uv,
    /// No face may grow beyond this many vertices.
    limit: usize,
}

/// A point of parameter space in the plane the triangulation works in.
fn to_plane(uv: Uv, centre: Uv, scale: Uv) -> P2 {
    let ([a, b], [ca, cb], [sa, sb]) = (uv, centre, scale);
    ((a - ca) * sa, (b - cb) * sb)
}

/// The distance from `p` to the segment from `a` to `b`.
fn distance_to_chord(p: Vec3, a: Vec3, b: Vec3) -> f64 {
    let along = b - a;
    let whole = along.length_squared();
    let s = if positive(whole) {
        ((p - a).dot(along) / whole).clamp(0.0, 1.0)
    } else {
        0.0
    };
    p.distance(a + along * s)
}

impl Boundary<'_> {
    /// The chords of edges that this face needs cut in two, because the
    /// edges were turned into points without regard to it (`edge.rs`):
    ///
    /// - chords that cross or touch another segment of the boundary. The
    ///   face's edges do not cross, so chords fine enough do not either;
    ///   but an edge that comes nearer to another than the chordal
    ///   tolerance may have a chord that cuts the corner (a hole close to
    ///   a rim, a notch that stops just short of an arc).
    /// - chords that are not what the triangulation takes them for. It
    ///   works in parameter space, where the chord is the straight segment
    ///   between its ends; the surface along that segment must stay near
    ///   the chord. Close to a pole it does not: the segment runs along a
    ///   parallel where the chord cuts across. The limit is twice the
    ///   chordal tolerance: one for the chord's own sag from its curve,
    ///   and one more that the segment's image may be from the curve.
    ///
    /// In no particular order, and a chord may be named more than once.
    /// Empty for a boundary that is fine, and for what finer chords cannot
    /// cure: straight edges on a surface whose straight lines are
    /// straight in its parameters, seams, poles.
    pub(crate) fn chords_to_refine(&self, body: &Body, tolerance: f64) -> Vec<Chord> {
        let region = &self.region;
        let analytic = !self.param.is_freeform();
        // A straight edge on a plane, a cylinder or a cone is its own
        // chord, in space and in parameters: cutting it changes nothing.
        let curable = |chord: Option<Chord>| -> Option<Chord> {
            let (edge, _) = chord?;
            let curve = &body.edges.get(edge as usize)?.curve;
            (!(analytic && matches!(curve, Curve3::Line(_)))).then_some(chord?)
        };
        struct Segment {
            low: P2,
            high: P2,
            ends: [P2; 2],
            chord: Option<Chord>,
        }
        let mut segments: Vec<Segment> = Vec::with_capacity(region.segs.len());
        let mut wanted: Vec<Chord> = Vec::new();
        let mut any_curable = false;
        for (n, &(i, j)) in region.segs.iter().enumerate() {
            let (Some(p), Some(q)) = (region.verts.get(i), region.verts.get(j)) else {
                continue;
            };
            let chord = curable(region.chords.get(n).copied().flatten());
            any_curable |= chord.is_some();
            if let Some(chord) = chord
                && !self.param.is_plane()
            {
                let ([pa, pb], [qa, qb]) = (p.uv, q.uv);
                let image = self.param.eval([0.5 * (pa + qa), 0.5 * (pb + qb)]);
                if exceeds(distance_to_chord(image, p.pos, q.pos), 2.0 * tolerance) {
                    wanted.push(chord);
                }
            }
            let a = to_plane(p.uv, self.centre, self.scale);
            let b = to_plane(q.uv, self.centre, self.scale);
            segments.push(Segment {
                low: (a.0.min(b.0), a.1.min(b.1)),
                high: (a.0.max(b.0), a.1.max(b.1)),
                ends: [a, b],
                chord,
            });
        }
        if any_curable {
            // Every pair of segments whose boxes overlap, found by going
            // through them from left to right.
            segments.sort_by(|x, y| x.low.0.total_cmp(&y.low.0));
            for (n, first) in segments.iter().enumerate() {
                for second in segments.iter().skip(n + 1) {
                    if second.low.0 > first.high.0 {
                        break;
                    }
                    if (first.chord.is_none() && second.chord.is_none())
                        || second.low.1 > first.high.1
                        || first.low.1 > second.high.1
                    {
                        continue;
                    }
                    // (Neighbours along a loop, and loops that touch in a
                    // vertex, meet end to end: that is no crossing.)
                    let ([a, b], [c, d]) = (first.ends, second.ends);
                    if seam_blocked([a.0, a.1], [b.0, b.1], [c.0, c.1], [d.0, d.1]) {
                        wanted.extend(first.chord);
                        wanted.extend(second.chord);
                    }
                }
            }
        }
        wanted
    }
}

/// The boundary of face `index` of `body`, whose edges are already points.
pub(crate) fn face_boundary<'a>(
    body: &Body,
    edges: &[Vec<Vec3>],
    face: &'a Face,
    index: usize,
    params: &TessParams,
) -> Result<Boundary<'a>, TessError> {
    let loops = loop_points(body, edges, face, index)?;
    let tolerance = params.chordal_tolerance;

    // The loops in parameter space, with the parameters swapped when they
    // wind around the second one.
    let mut found: Option<(Param, Vec<Ring>, f64)> = None;
    for swapped in [false, true] {
        let param = Param {
            surface: &face.surface,
            swapped,
        };
        // Loops run counter-clockwise around the outward normal, which is
        // the surface's `∂u × ∂v` unless the face is reversed.
        let sense = if swapped != face.reversed { -1.0 } else { 1.0 };
        let mut rings = Vec::with_capacity(loops.len());
        for (number, points) in loops.iter().enumerate() {
            rings.push(build_ring(&param, points, sense, index, number)?);
        }
        let around = |k: usize| rings.iter().any(|r| get2(r.winding, k) != 0);
        if rings.iter().any(|r| r.winding.iter().any(|w| w.abs() > 1)) {
            return unsupported(index, "a loop that winds around its surface more than once");
        }
        match (around(0), around(1)) {
            (true, true) => {
                return unsupported(
                    index,
                    "loops that wind around both the axis and the tube of a torus",
                );
            }
            (false, true) if !swapped => continue,
            _ => {
                found = Some((param, rings, sense));
                break;
            }
        }
    }
    let Some((param, rings, sense)) = found else {
        return unsupported(
            index,
            "loops that wind around its surface in a way not handled",
        );
    };
    let winding = rings.iter().filter(|r| r.winding != [0, 0]).count();
    // Loops that all run clockwise bound nothing in a plane. On a closed
    // surface they bound the rest of it: the face is the whole surface
    // with those loops as holes (the zone left of a ball with a hole
    // drilled through it across the sphere's axis).
    let periods = param.periods();
    let around_holes = winding == 0
        && !rings.is_empty()
        && rings
            .iter()
            .all(|r| positive(-r.twice_area(periods) * sense))
        && closes_to_points(&param);
    let mut region = if rings.is_empty() || winding > 0 || around_holes {
        if winding > 2 {
            return fail(
                index,
                "its loops do not bound a region: more than two of them wind around the axis",
            );
        }
        band_region(&param, &rings, sense, index)?
    } else {
        patch_region(&param, &rings, sense, index)?
    };

    // The box the region lies in, and the grid over it.
    let mut low = [f64::INFINITY; 2];
    let mut high = [f64::NEG_INFINITY; 2];
    for v in &region.verts {
        for k in 0..2 {
            let x = get2(v.uv, k);
            if !x.is_finite() {
                return fail(index, "its boundary has no finite surface parameters");
            }
            let (lowest, highest) = (get2(low, k).min(x), get2(high, k).max(x));
            set2(&mut low, k, lowest);
            set2(&mut high, k, highest);
        }
    }
    let span = [get2(high, 0) - get2(low, 0), get2(high, 1) - get2(low, 1)];
    if !span.iter().all(|s| s.is_finite() && *s > 0.0) {
        return fail(
            index,
            "its loops do not bound a region: they enclose no area on the surface",
        );
    }
    let cells = if param.is_plane() {
        [1, 1]
    } else {
        let (low, high) = (param.raw(low), param.raw(high));
        let (nu, nv) = seed_cells(&face.surface, low, high, params, index)?;
        param.order(nu, nv)
    };
    let step = [
        get2(span, 0) / f64::from(get2(cells, 0)),
        get2(span, 1) / f64::from(get2(cells, 1)),
    ];
    // No face may grow beyond this many vertices.
    let side = (params.max_segments as usize).saturating_add(1);
    let limit = side.saturating_mul(side).saturating_mul(4);
    finish_boundary(&param, &mut region, step, tolerance, limit, index)?;
    if region.verts.len() > limit {
        return Err(TessError::TooFine {
            patch: index,
            needed_segments: (region.verts.len() as f64).sqrt(),
            max_segments: params.max_segments,
        });
    }

    // Scale the parameters by how fast the surface moves with them, so
    // that "Delaunay" means roughly what it would in space.
    let mut scale = [1.0_f64; 2];
    if !param.is_plane() {
        let mut largest = [0.0_f64; 2];
        for i in 0..5 {
            for j in 0..5 {
                let uv = [
                    get2(low, 0) + get2(span, 0) * f64::from(i) / 4.0,
                    get2(low, 1) + get2(span, 1) * f64::from(j) / 4.0,
                ];
                let (_, [da, db]) = param.derivatives(uv);
                let [la, lb] = &mut largest;
                *la = la.max(da.length());
                *lb = lb.max(db.length());
            }
        }
        for k in 0..2 {
            let l = get2(largest, k);
            if l.is_finite() && l > 0.0 {
                set2(&mut scale, k, l);
            }
        }
    }
    if matches!(face.surface, Surface::Cylinder(_) | Surface::Cone(_)) {
        // Straight along v: how far apart two points are in v does not
        // matter for how well a triangle fits, only how far apart they
        // are around the axis. Squeeze v until the whole face is half a
        // grid column high, so that the triangulation joins points that
        // are neighbours around the axis, along the rulings, also where
        // the two ends of a wall are not parallel.
        let [sa, sb] = scale;
        let [da, _] = step;
        let [_, height] = span;
        let (column, tall) = (0.5 * da * sa, height * sb);
        if column > 0.0 && tall > column {
            scale = [sa, sb * column / tall];
        }
    }
    let centre = [
        0.5 * (get2(low, 0) + get2(high, 0)),
        0.5 * (get2(low, 1) + get2(high, 1)),
    ];
    Ok(Boundary {
        param,
        region,
        low,
        high,
        cells,
        step,
        scale,
        centre,
        limit,
    })
}

/// Triangulates face `index`, given its boundary.
pub(crate) fn mesh_face(
    boundary: &Boundary,
    face: &Face,
    index: usize,
    params: &TessParams,
) -> Result<FaceMesh, TessError> {
    let (param, region) = (boundary.param, &boundary.region);
    if let Some(doubt) = &region.doubt {
        return Err(doubt.clone());
    }
    let (low, high, cells, step) = (boundary.low, boundary.high, boundary.cells, boundary.step);
    let (scale, centre, limit) = (boundary.scale, boundary.centre, boundary.limit);
    let tolerance = params.chordal_tolerance;

    let explain = |error: CdtError| -> TessError {
        let reason = match error {
            CdtError::NonFinite => "its boundary has points that are not finite",
            CdtError::Degenerate => {
                "its boundary cannot be triangulated: points of it coincide or its loops \
                 enclose no area in parameter space"
            }
            CdtError::Crossing => {
                "its loops do not bound a region: the boundary crosses itself in parameter space"
            }
            CdtError::Open => {
                "its loops do not bound a region: the boundary is not closed in parameter space"
            }
            CdtError::TooMany => {
                return TessError::TooFine {
                    patch: index,
                    needed_segments: f64::from(params.max_segments) * 2.0 + 2.0,
                    max_segments: params.max_segments,
                };
            }
            CdtError::Internal => {
                return TessError::Internal(format!(
                    "face {index}: the triangulation lost track of its own triangles"
                ));
            }
        };
        TessError::Face {
            face: index,
            reason: reason.into(),
        }
    };

    let mut work = Work {
        param,
        cdt: Cdt::new(
            (
                (low[0] - centre[0]) * scale[0],
                (low[1] - centre[1]) * scale[1],
            ),
            (
                (high[0] - centre[0]) * scale[0],
                (high[1] - centre[1]) * scale[1],
            ),
            limit,
        )
        .map_err(explain)?,
        uv: Vec::new(),
        pos: Vec::new(),
        free: Vec::new(),
        centre,
        scale,
        tolerance,
    };

    // Boundary points, in an order that jumps around the boundary (a
    // fixed stride near the golden ratio of their number): inserting
    // neighbours one after the other makes long thin fans that the next
    // point has to undo.
    let count = region.verts.len();
    let mut stride = ((count as f64) * 0.618) as usize;
    let coprime = |mut a: usize, mut b: usize| {
        while b != 0 {
            (a, b) = (b, a % b);
        }
        a == 1
    };
    while stride > 1 && !coprime(stride, count) {
        stride -= 1;
    }
    let stride = stride.max(1);
    let mut vertex_of: Vec<u32> = vec![0; count];
    for n in 0..count {
        let i = (n * stride) % count;
        let Some(&v) = region.verts.get(i) else {
            continue;
        };
        let id = work
            .cdt
            .insert_boundary(work.plane(v.uv))
            .map_err(explain)?;
        match work.vertex(id) {
            None => work.record(id, v.uv, v.pos, v.free).map_err(explain)?,
            Some((_, pos, _)) => {
                if pos != v.pos {
                    return fail(
                        index,
                        "its boundary passes twice through one place on the surface with \
                         different points",
                    );
                }
            }
        }
        if let Some(slot) = vertex_of.get_mut(i) {
            *slot = id;
        }
    }
    for &(i, j) in &region.segs {
        let (Some(&a), Some(&b)) = (vertex_of.get(i), vertex_of.get(j)) else {
            continue;
        };
        work.cdt.constrain(a, b).map_err(explain)?;
    }
    if work.cdt.classify().map_err(explain)? == 0 {
        return fail(
            index,
            "its loops do not bound a region: they enclose no area on the surface",
        );
    }

    if !work.param.is_plane() {
        // Seed with the grid, away from the boundary: a point just inside
        // a boundary chord makes a triangle that stands on its side.
        let [columns, rows] = cells;
        let plane_step = (step[0] * scale[0]).min(step[1] * scale[1]);
        for (i, j) in grid_order(columns, rows) {
            let uv = [
                low[0] + step[0] * f64::from(i),
                low[1] + step[1] * f64::from(j),
            ];
            let added = work
                .cdt
                .insert_interior(work.plane(uv), 0.3 * plane_step)
                .map_err(explain)?;
            if let Interior::Added(v) = added {
                let pos = work.param.eval(uv);
                work.record(v, uv, pos, None).map_err(explain)?;
            }
        }

        // Measure every triangle, improve the ones that fail, and again
        // for the triangles that changed, until none fails.
        let mut measured: Vec<u32> = Vec::new();
        let mut done = false;
        for _ in 0..200 {
            let total = work.cdt.triangle_count();
            measured.resize(total, u32::MAX);
            let mut failing: Vec<(usize, u32, Action)> = Vec::new();
            for t in 0..total {
                let Some(view) = work.cdt.triangle(t) else {
                    continue;
                };
                if !view.inside || measured.get(t) == Some(&view.generation) {
                    continue;
                }
                match work.assess(t) {
                    Action::Keep => {
                        if let Some(slot) = measured.get_mut(t) {
                            *slot = view.generation;
                        }
                    }
                    action => failing.push((t, view.generation, action)),
                }
            }
            if failing.is_empty() {
                done = true;
                break;
            }
            for (t, generation, action) in failing {
                // Changed by an earlier split in this round: next round.
                if work.cdt.triangle(t).map(|view| view.generation) != Some(generation) {
                    continue;
                }
                let split = match action {
                    Action::SplitEdge(i) => match work.split_edge(t, i) {
                        Err(CdtError::Degenerate) => work.split_triangle(t),
                        other => other,
                    },
                    _ => work.split_triangle(t),
                };
                match split {
                    Ok(()) => {}
                    // No room left for a point between its corners: the
                    // tolerance is beyond what the numbers can resolve.
                    Err(CdtError::Degenerate) => return Err(explain(CdtError::TooMany)),
                    Err(e) => return Err(explain(e)),
                }
            }
        }
        if !done {
            return Err(explain(CdtError::TooMany));
        }
    }

    // The triangles, facing outward, over the vertices they use.
    let flip = work.param.swapped != face.reversed;
    let outward = if face.reversed { -1.0 } else { 1.0 };
    let mut mesh = FaceMesh::default();
    let mut index_of: Vec<u32> = vec![u32::MAX; work.uv.len()];
    for t in 0..work.cdt.triangle_count() {
        let Some(view) = work.cdt.triangle(t).filter(|view| view.inside) else {
            continue;
        };
        let [Some(a), Some(b), Some(c)] = view.v.map(|v| work.vertex(v)) else {
            return Err(explain(CdtError::Internal));
        };
        if a.1 == b.1 || b.1 == c.1 || c.1 == a.1 {
            // One point in space on two corners (a pole): no area, and
            // the neighbours on either side already meet there.
            continue;
        }
        let area = (b.1 - a.1).cross(c.1 - a.1).length_squared();
        if !(area > 0.0 && area.is_finite()) {
            return fail(
                index,
                "its surface folds three distinct points onto one line (a triangle without area)",
            );
        }
        let mut corners = [0_u32; 3];
        for (slot, v) in corners.iter_mut().zip(view.v) {
            let i = v
                .checked_sub(FIRST_VERTEX)
                .map_or(usize::MAX, |i| i as usize);
            let (Some(known), Some(vertex)) = (index_of.get_mut(i), work.vertex(v)) else {
                return Err(explain(CdtError::Internal));
            };
            if *known == u32::MAX {
                *known =
                    u32::try_from(mesh.positions.len()).map_err(|_| TessError::TooManyVertices)?;
                mesh.positions.push(vertex.1);
                let normal = work.param.normal(vertex.0);
                mesh.normals
                    .push(normal.map_or(Vec3::ZERO, |n| n * outward));
            }
            *slot = *known;
        }
        let [a, b, c] = corners;
        mesh.triangles
            .push(if flip { [a, c, b] } else { [a, b, c] });
    }
    if mesh.triangles.is_empty() {
        return fail(
            index,
            "its loops do not bound a region: they enclose no area on the surface",
        );
    }
    Ok(mesh)
}
