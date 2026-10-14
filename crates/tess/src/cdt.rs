//! A constrained Delaunay triangulation of the plane, built for the
//! tessellator: boundary points and boundary segments are given and never
//! moved, split or dropped; interior points are added afterwards.
//!
//! Method (textbook; no code was taken from any library):
//! - incremental insertion with Lawson's edge flips (C. L. Lawson,
//!   "Software for C¹ surface interpolation", 1977; de Berg et al.,
//!   *Computational Geometry*, ch. 9), inside a bounding square so that
//!   every point falls inside an existing triangle;
//! - constraint segments are recovered by flipping the edges they cross
//!   (S. W. Sloan, "A fast algorithm for generating constrained Delaunay
//!   triangulations", Computers & Structures 47, 1993);
//! - the region is whatever lies an odd number of constraint crossings
//!   away from the outside (even–odd rule), so holes need no bridges.
//!
//! Robustness. The orientation and in-circle tests are plain floating
//! point with a forward error bound (the static filters of J. R. Shewchuk,
//! "Adaptive precision floating-point arithmetic and fast robust geometric
//! predicates", 1997). A result inside the bound counts as "zero": such a
//! triangle is never created and such a flip is never made. Every stored
//! triangle is therefore certainly counter-clockwise with non-zero area,
//! whatever the input; input the filter cannot decide (points closer
//! together than rounding resolves) is an error, never a bad mesh. Flips
//! are only made when certainly improving, so they terminate; all loops
//! carry a budget on top of that.

/// A point of the plane.
pub(crate) type P2 = (f64, f64);

const NONE: u32 = u32::MAX;

/// The first four vertices are the corners of the bounding square.
pub(crate) const FIRST_VERTEX: u32 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CdtError {
    /// A coordinate is NaN or infinite.
    NonFinite,
    /// Points coincide or are collinear within rounding where they may not.
    Degenerate,
    /// Two constraint segments cross.
    Crossing,
    /// The constraint segments do not form closed curves.
    Open,
    /// More points than the caller allowed.
    TooMany,
    /// The triangulation's own bookkeeping is inconsistent (a bug).
    Internal,
    /// The work went beyond what the points so far justify
    /// ([`Cdt::spend`]): point location, flips or the recovery of a
    /// constraint did not converge.
    TooSlow,
}

/// Twice the signed area of (a, b, c): positive when counter-clockwise.
/// Exactly 0.0 when the sign is not certain.
pub(crate) fn orient(a: P2, b: P2, c: P2) -> f64 {
    let left = (b.0 - a.0) * (c.1 - a.1);
    let right = (b.1 - a.1) * (c.0 - a.0);
    let det = left - right;
    // Shewchuk's bound is (3 + 16 ε) ε (|left| + |right|), ε = 2⁻⁵³.
    let bound = 3.4e-16 * (left.abs() + right.abs());
    if det.abs() <= bound { 0.0 } else { det }
}

/// Positive when `d` is certainly inside the circle through the
/// counter-clockwise triangle (a, b, c); 0.0 when not certain.
fn in_circle(a: P2, b: P2, c: P2, d: P2) -> f64 {
    let (adx, ady) = (a.0 - d.0, a.1 - d.1);
    let (bdx, bdy) = (b.0 - d.0, b.1 - d.1);
    let (cdx, cdy) = (c.0 - d.0, c.1 - d.1);
    let (al, bl, cl) = (
        adx * adx + ady * ady,
        bdx * bdx + bdy * bdy,
        cdx * cdx + cdy * cdy,
    );
    let det =
        al * (bdx * cdy - bdy * cdx) + bl * (cdx * ady - cdy * adx) + cl * (adx * bdy - ady * bdx);
    let permanent = al * ((bdx * cdy).abs() + (bdy * cdx).abs())
        + bl * ((cdx * ady).abs() + (cdy * adx).abs())
        + cl * ((adx * bdy).abs() + (ady * bdx).abs());
    // Shewchuk's bound is (10 + 96 ε) ε × permanent.
    if det.abs() <= 1.2e-15 * permanent {
        0.0
    } else {
        det
    }
}

fn pick<T: Copy>(a: [T; 3], i: usize) -> T {
    let [x, y, z] = a;
    match i % 3 {
        0 => x,
        1 => y,
        _ => z,
    }
}

fn put<T>(a: &mut [T; 3], i: usize, value: T) {
    let [x, y, z] = a;
    match i % 3 {
        0 => *x = value,
        1 => *y = value,
        _ => *z = value,
    }
}

/// Squared distance from `p` to the segment (a, b).
fn distance_squared_to_segment(p: P2, a: P2, b: P2) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let length = dx * dx + dy * dy;
    let t = if length > 0.0 {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / length).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let (qx, qy) = (a.0 + t * dx - p.0, a.1 + t * dy - p.1);
    qx * qx + qy * qy
}

/// One triangle. Edge `i` is the one opposite vertex `i`: it runs from
/// vertex `i + 1` to vertex `i + 2`.
#[derive(Clone, Copy, Debug)]
struct Tri {
    v: [u32; 3],
    /// The triangle across each edge, or `NONE` on the bounding square.
    n: [u32; 3],
    /// How many constraint segments lie on each edge.
    c: [u8; 3],
    inside: bool,
    /// Changes whenever the triangle is rewritten.
    generation: u32,
}

/// What a caller may know about a triangle.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TriView {
    pub v: [u32; 3],
    pub constrained: [bool; 3],
    pub inside: bool,
    pub generation: u32,
}

enum Location {
    Inside(u32),
    OnEdge(u32, usize),
    Vertex(u32),
    /// Too close to a vertex to tell apart, yet not equal to it.
    Near,
}

enum Toward {
    /// The edge `i` of triangle `t` leads from the start to this vertex,
    /// which lies on the segment.
    Edge(u32, usize, u32),
    /// The segment leaves the start through triangle `t`, across the edge
    /// opposite its vertex `k`.
    Crossing(u32, usize),
}

/// How many triangles within the clearance of an interior point are
/// searched for constraints ([`Cdt::insert_interior`]).
const NEAR_TRIANGLES: usize = 64;

/// The work budget of a triangulation ([`Cdt::spend`]): steps allowed
/// whatever the number of points,
const WORK_BASE: f64 = 65536.0;
/// per point,
const WORK_PER_POINT: f64 = 4096.0;
/// and per point times the square root of their number.
const WORK_PER_ROOT: f64 = 64.0;

/// The result of offering an interior point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Interior {
    Added(u32),
    /// Outside the region, on or too close to a constraint, or on top of
    /// an existing point.
    Skipped,
}

pub(crate) struct Cdt {
    pts: Vec<P2>,
    tris: Vec<Tri>,
    /// One triangle that uses each vertex.
    vert_tri: Vec<u32>,
    max_points: usize,
    /// Where the next point location starts.
    hint: u32,
    /// The steps taken so far, for the work budget ([`Self::spend`]).
    work: u64,
    /// The box the points were said to lie in.
    bounds: (P2, P2),
    /// Where a walk that ran out of steps starts again
    /// ([`Self::locate_from_grid`]); made the first time it is needed.
    grid: Option<Grid>,
}

/// Vertices by where they lie: a box cut into cells of about one point
/// each, with the last vertex added in each cell. It gives a walk that ran
/// out of steps a start next to its point (on a long, thin region, such
/// as the land of a thread in its parameters, a walk from the last point
/// crosses a number of triangles that grows with all of them, and every
/// point would cost as much as all the points before it).
struct Grid {
    origin: P2,
    cell: P2,
    columns: usize,
    rows: usize,
    cells: Vec<u32>,
    /// The points there were when it was made: it is made again, finer,
    /// when there are twice as many.
    made_for: usize,
}

/// The most cells a [`Grid`] has.
const MOST_CELLS: usize = 1 << 20;

impl Grid {
    fn cell_of(&self, p: P2) -> (usize, usize) {
        let along = |x: f64, origin: f64, size: f64, count: usize| -> usize {
            let k = ((x - origin) / size).floor();
            if k.is_nan() || k < 0.0 {
                0
            } else {
                // In range: clamped to the last cell.
                (k as usize).min(count.saturating_sub(1))
            }
        };
        (
            along(p.0, self.origin.0, self.cell.0, self.columns),
            along(p.1, self.origin.1, self.cell.1, self.rows),
        )
    }

    fn put(&mut self, p: P2, v: u32) {
        let (i, j) = self.cell_of(p);
        if let Some(slot) = self.cells.get_mut(j * self.columns + i) {
            *slot = v;
        }
    }
}

impl Cdt {
    /// An empty triangulation of a square around the box `min..max`, for
    /// at most `max_points` points.
    pub(crate) fn new(min: P2, max: P2, max_points: usize) -> Result<Self, CdtError> {
        if ![min.0, min.1, max.0, max.1].iter().all(|x| x.is_finite()) {
            return Err(CdtError::NonFinite);
        }
        let size = (max.0 - min.0).max(max.1 - min.1);
        if !(size.is_finite() && size > 0.0) {
            return Err(CdtError::Degenerate);
        }
        let pts = vec![
            (min.0 - size, min.1 - size),
            (max.0 + size, min.1 - size),
            (max.0 + size, max.1 + size),
            (min.0 - size, max.1 + size),
        ];
        let tri = |v, n| Tri {
            v,
            n,
            c: [0; 3],
            inside: false,
            generation: 0,
        };
        Ok(Self {
            pts,
            tris: vec![
                tri([0, 1, 2], [NONE, 1, NONE]),
                tri([0, 2, 3], [NONE, NONE, 0]),
            ],
            vert_tri: vec![0, 0, 0, 1],
            max_points,
            hint: 0,
            work: 0,
            bounds: (min, max),
            grid: None,
        })
    }

    /// Counts `steps` of work (a triangle walked through or looked at, a
    /// flip, a crossed edge) against the budget: the work a triangulation
    /// of `n` points may take is [`WORK_BASE`] plus `n` times
    /// ([`WORK_PER_POINT`] plus [`WORK_PER_ROOT`] times `√n`), where `n` is
    /// the number of points so far. A walk from a far hint costs about
    /// `√n` steps, so that is what any order of insertion costs at most
    /// on the way; work that grows as `n` per point (a walk that cannot
    /// find its way, or flips that undo each other) runs out of it, with
    /// [`CdtError::TooSlow`], in time of the order of `n^1.5` instead of
    /// running on.
    fn spend(&mut self, steps: usize) -> Result<(), CdtError> {
        self.work = self.work.saturating_add(steps as u64);
        if self.work > self.allowance() {
            return Err(CdtError::TooSlow);
        }
        Ok(())
    }

    /// The work [`Self::spend`] allows for the points so far.
    fn allowance(&self) -> u64 {
        let n = self.pts.len() as f64;
        let allowed = WORK_BASE + n * (WORK_PER_POINT + WORK_PER_ROOT * n.sqrt());
        // Far below 2⁶³: the points are in memory.
        allowed as u64
    }

    pub(crate) fn triangle_count(&self) -> usize {
        self.tris.len()
    }

    pub(crate) fn triangle(&self, t: usize) -> Option<TriView> {
        self.tris.get(t).map(|tri| TriView {
            v: tri.v,
            constrained: tri.c.map(|c| c > 0),
            inside: tri.inside,
            generation: tri.generation,
        })
    }

    fn t(&self, t: u32) -> Result<Tri, CdtError> {
        self.tris.get(t as usize).copied().ok_or(CdtError::Internal)
    }

    fn tm(&mut self, t: u32) -> Result<&mut Tri, CdtError> {
        self.tris.get_mut(t as usize).ok_or(CdtError::Internal)
    }

    fn p(&self, v: u32) -> Result<P2, CdtError> {
        self.pts.get(v as usize).copied().ok_or(CdtError::Internal)
    }

    fn corners(&self, tri: &Tri) -> Result<[P2; 3], CdtError> {
        let [a, b, c] = tri.v;
        Ok([self.p(a)?, self.p(b)?, self.p(c)?])
    }

    /// Rewrites triangle `t`, keeping its `inside` flag.
    fn write(&mut self, t: u32, v: [u32; 3], n: [u32; 3], c: [u8; 3]) -> Result<(), CdtError> {
        let tri = self.tm(t)?;
        tri.v = v;
        tri.n = n;
        tri.c = c;
        tri.generation = tri.generation.wrapping_add(1);
        for vertex in v {
            if let Some(slot) = self.vert_tri.get_mut(vertex as usize) {
                *slot = t;
            }
        }
        Ok(())
    }

    fn push_triangle(&mut self, inside: bool) -> Result<u32, CdtError> {
        let index = u32::try_from(self.tris.len()).map_err(|_| CdtError::TooMany)?;
        if index == NONE {
            return Err(CdtError::TooMany);
        }
        self.tris.push(Tri {
            v: [0; 3],
            n: [NONE; 3],
            c: [0; 3],
            inside,
            generation: 0,
        });
        Ok(index)
    }

    fn push_point(&mut self, p: P2) -> Result<u32, CdtError> {
        if self.pts.len() >= self.max_points.saturating_add(FIRST_VERTEX as usize) {
            return Err(CdtError::TooMany);
        }
        let index = u32::try_from(self.pts.len()).map_err(|_| CdtError::TooMany)?;
        self.pts.push(p);
        self.vert_tri.push(NONE);
        if let Some(grid) = &mut self.grid {
            grid.put(p, index);
        }
        Ok(index)
    }

    /// The index in triangle `of` of the edge that borders triangle `to`.
    fn back(&self, of: u32, to: u32) -> Result<usize, CdtError> {
        let tri = self.t(of)?;
        (0..3)
            .find(|&j| pick(tri.n, j) == to)
            .ok_or(CdtError::Internal)
    }

    /// Makes `neighbour` point at `new` where it pointed at `old`.
    fn relink(&mut self, neighbour: u32, old: u32, new: u32) -> Result<(), CdtError> {
        if neighbour == NONE {
            return Ok(());
        }
        let j = self.back(neighbour, old)?;
        put(&mut self.tm(neighbour)?.n, j, new);
        Ok(())
    }

    fn classify_location(
        &self,
        t: u32,
        tri: &Tri,
        o: [f64; 3],
        p: P2,
    ) -> Result<Location, CdtError> {
        let zeros = o.iter().filter(|x| **x == 0.0).count();
        Ok(match zeros {
            0 => Location::Inside(t),
            1 => Location::OnEdge(t, o.iter().position(|x| *x == 0.0).unwrap_or(0)),
            _ => {
                let mut found = Location::Near;
                for v in tri.v {
                    if self.p(v)? == p {
                        found = Location::Vertex(v);
                    }
                }
                found
            }
        })
    }

    /// The triangle that holds `p`, by walking from the hint towards it.
    fn locate(&mut self, p: P2) -> Result<Location, CdtError> {
        let mut t = if (self.hint as usize) < self.tris.len() {
            self.hint
        } else {
            0
        };
        // A walk is a few steps from a nearby hint and about √n from far
        // away. Without the Delaunay property everywhere (constraints) it
        // can in theory go round in circles: then every triangle is tried.
        let budget = 64 + 8 * (self.tris.len() as f64).sqrt() as usize;
        let mut steps = 0;
        'walk: for _ in 0..budget {
            steps += 1;
            let tri = self.t(t)?;
            let [a, b, c] = self.corners(&tri)?;
            let mut o = [0.0; 3];
            for (i, (from, to)) in [(b, c), (c, a), (a, b)].into_iter().enumerate() {
                let side = orient(from, to, p);
                if side < 0.0 {
                    let next = pick(tri.n, i);
                    if next == NONE {
                        return Err(CdtError::Degenerate); // Outside the square.
                    }
                    t = next;
                    continue 'walk;
                }
                put(&mut o, i, side);
            }
            self.spend(steps)?;
            return self.classify_location(t, &tri, o, p);
        }
        self.spend(steps)?;
        if let Some(found) = self.locate_from_grid(p)? {
            return Ok(found);
        }
        self.spend(self.tris.len())?;
        self.scan(p)
    }

    /// The first triangle, in the order they were made, that holds `p`:
    /// what [`Self::locate`] answers when the walk runs out of steps.
    fn scan(&self, p: P2) -> Result<Location, CdtError> {
        for (index, tri) in self.tris.iter().enumerate() {
            let [a, b, c] = self.corners(tri)?;
            let o = [orient(b, c, p), orient(c, a, p), orient(a, b, p)];
            if o.iter().all(|x| *x >= 0.0) {
                return self.classify_location(index as u32, tri, o, p);
            }
        }
        Err(CdtError::Degenerate)
    }

    /// What [`Self::scan`] answers, found by a walk from a vertex near
    /// `p` instead of by looking at every triangle; `None` when that does
    /// not settle it (the walk does not arrive, or `p` is on or next to a
    /// vertex), and the scan must.
    ///
    /// Only the triangle of a point on an edge can differ between a walk
    /// and the scan: there two triangles hold it, and the scan takes the
    /// one made first. So the triangle the walk ends in and its three
    /// neighbours are looked at in the order they were made, the first
    /// that holds `p` is the answer, and the triangulation comes out the
    /// same whichever way it was found.
    fn locate_from_grid(&mut self, p: P2) -> Result<Option<Location>, CdtError> {
        let points = self.pts.len().saturating_sub(FIRST_VERTEX as usize);
        let stale = self
            .grid
            .as_ref()
            .is_none_or(|grid| points > grid.made_for.saturating_mul(2));
        if stale {
            self.make_grid(points)?;
        }
        let Some(start) = self.near_vertex(p)? else {
            return Ok(None);
        };
        let mut t = start;
        let mut walked = None;
        'walk: for _ in 0..=self.tris.len() {
            self.spend(1)?;
            let tri = self.t(t)?;
            let [a, b, c] = self.corners(&tri)?;
            let mut o = [0.0; 3];
            for (i, (from, to)) in [(b, c), (c, a), (a, b)].into_iter().enumerate() {
                let side = orient(from, to, p);
                if side < 0.0 {
                    let next = pick(tri.n, i);
                    if next == NONE {
                        return Err(CdtError::Degenerate); // Outside the square.
                    }
                    t = next;
                    continue 'walk;
                }
                put(&mut o, i, side);
            }
            walked = Some((t, tri, o));
            break;
        }
        let Some((t, tri, o)) = walked else {
            return Ok(None);
        };
        if o.iter().filter(|x| **x == 0.0).count() >= 2 {
            return Ok(None);
        }
        let mut candidates: Vec<u32> = std::iter::once(t)
            .chain(tri.n.into_iter().filter(|&u| u != NONE))
            .collect();
        candidates.sort_unstable();
        for u in candidates {
            let tri = self.t(u)?;
            let [a, b, c] = self.corners(&tri)?;
            let o = [orient(b, c, p), orient(c, a, p), orient(a, b, p)];
            if o.iter().all(|x| *x >= 0.0) {
                return self.classify_location(u, &tri, o, p).map(Some);
            }
        }
        Ok(None)
    }

    /// Makes the [`Grid`] for `points` points, from the vertices so far.
    fn make_grid(&mut self, points: usize) -> Result<(), CdtError> {
        let (min, max) = self.bounds;
        let (width, height) = (max.0 - min.0, max.1 - min.1);
        let positive = |x: f64| if x.is_finite() && x > 0.0 { x } else { 0.0 };
        let (width, height) = (positive(width), positive(height));
        let target = points.clamp(1, MOST_CELLS) as f64;
        // Square cells where the box allows, about one point each.
        let (columns, rows) = if width > 0.0 && height > 0.0 {
            let columns = (target * width / height).sqrt().ceil().clamp(1.0, target);
            let rows = (target / columns).ceil().clamp(1.0, target);
            (columns, rows)
        } else if width > 0.0 {
            (target, 1.0)
        } else {
            (1.0, target)
        };
        // In range: at most `MOST_CELLS` each.
        let (columns, rows) = (columns as usize, rows as usize);
        let cells = columns.saturating_mul(rows);
        self.spend(cells.saturating_add(points))?;
        let size = |extent: f64, count: usize| {
            let size = extent / count as f64;
            if size.is_finite() && size > 0.0 {
                size
            } else {
                1.0
            }
        };
        let mut grid = Grid {
            origin: min,
            cell: (size(width, columns), size(height, rows)),
            columns,
            rows,
            cells: vec![NONE; cells],
            made_for: points.max(1),
        };
        for (v, &p) in self.pts.iter().enumerate().skip(FIRST_VERTEX as usize) {
            let Ok(v) = u32::try_from(v) else { break };
            grid.put(p, v);
        }
        self.grid = Some(grid);
        Ok(())
    }

    /// A triangle at a vertex in the cell of `p`, or in the nearest ring
    /// of cells around it that has one.
    fn near_vertex(&mut self, p: P2) -> Result<Option<u32>, CdtError> {
        let Some(grid) = &self.grid else {
            return Ok(None);
        };
        let (ci, cj) = grid.cell_of(p);
        let (columns, rows) = (grid.columns, grid.rows);
        let triangle_at = |i: usize, j: usize| -> Option<u32> {
            let v = *grid.cells.get(j.checked_mul(columns)?.checked_add(i)?)?;
            let t = *self.vert_tri.get(v as usize)?;
            (v != NONE && t != NONE).then_some(t)
        };
        let mut looked = 0_usize;
        let mut found = None;
        'rings: for r in 0..columns.max(rows) {
            let (i0, i1) = (ci.saturating_sub(r), ci.saturating_add(r).min(columns - 1));
            let (j0, j1) = (cj.saturating_sub(r), cj.saturating_add(r).min(rows - 1));
            for j in j0..=j1 {
                // The whole row on the ring's top and bottom, its two ends
                // on the rows between.
                let edge_row = j + r == cj || j == cj + r;
                let step = if edge_row { 1 } else { (i1 - i0).max(1) };
                let mut i = i0;
                while i <= i1 {
                    looked += 1;
                    if (i + r == ci || i == ci + r || edge_row)
                        && let Some(t) = triangle_at(i, j)
                    {
                        found = Some(t);
                        break 'rings;
                    }
                    i += step;
                }
            }
        }
        self.spend(looked)?;
        Ok(found)
    }

    /// Splits triangle `t` into three around `p`, which is certainly
    /// inside it.
    fn add_in_triangle(&mut self, t: u32, p: P2) -> Result<u32, CdtError> {
        let tri = self.t(t)?;
        let [a, b, c] = tri.v;
        let [pa, pb, pc] = self.corners(&tri)?;
        if !(orient(pb, pc, p) > 0.0 && orient(pc, pa, p) > 0.0 && orient(pa, pb, p) > 0.0) {
            return Err(CdtError::Degenerate);
        }
        let [n0, n1, n2] = tri.n;
        let [c0, c1, c2] = tri.c;
        let v = self.push_point(p)?;
        let t1 = self.push_triangle(tri.inside)?;
        let t2 = self.push_triangle(tri.inside)?;
        self.write(t, [v, b, c], [n0, t1, t2], [c0, 0, 0])?;
        self.write(t1, [v, c, a], [n1, t2, t], [c1, 0, 0])?;
        self.write(t2, [v, a, b], [n2, t, t1], [c2, 0, 0])?;
        self.relink(n1, t, t1)?;
        self.relink(n2, t, t2)?;
        self.hint = t;
        self.legalize(vec![(t, 0), (t1, 0), (t2, 0)])?;
        Ok(v)
    }

    /// Splits edge `i` of triangle `t`, and the triangle across it, at `p`,
    /// which lies on that edge within rounding.
    fn add_on_edge(&mut self, t: u32, i: usize, p: P2) -> Result<u32, CdtError> {
        let tri = self.t(t)?;
        let u = pick(tri.n, i);
        if u == NONE {
            return Err(CdtError::Degenerate);
        }
        let other = self.t(u)?;
        let j = self.back(u, t)?;
        let (c, a, b) = (pick(tri.v, i), pick(tri.v, i + 1), pick(tri.v, i + 2));
        let d = pick(other.v, j);
        let (pa, pb, pc, pd) = (self.p(a)?, self.p(b)?, self.p(c)?, self.p(d)?);
        if !(orient(pc, pa, p) > 0.0
            && orient(pc, p, pb) > 0.0
            && orient(pd, pb, p) > 0.0
            && orient(pd, p, pa) > 0.0)
        {
            return Err(CdtError::Degenerate);
        }
        let split = pick(tri.c, i);
        let (n_bc, c_bc) = (pick(tri.n, i + 1), pick(tri.c, i + 1));
        let (n_ca, c_ca) = (pick(tri.n, i + 2), pick(tri.c, i + 2));
        // `other` is (d, b, a) seen from d.
        let (n_ad, c_ad) = (pick(other.n, j + 1), pick(other.c, j + 1));
        let (n_db, c_db) = (pick(other.n, j + 2), pick(other.c, j + 2));
        let v = self.push_point(p)?;
        let t2 = self.push_triangle(tri.inside)?;
        let u2 = self.push_triangle(other.inside)?;
        self.write(t, [c, a, v], [u2, t2, n_ca], [split, 0, c_ca])?;
        self.write(t2, [c, v, b], [u, n_bc, t], [split, c_bc, 0])?;
        self.write(u, [d, b, v], [t2, u2, n_db], [split, 0, c_db])?;
        self.write(u2, [d, v, a], [t, n_ad, u], [split, c_ad, 0])?;
        self.relink(n_bc, t, t2)?;
        self.relink(n_ad, u, u2)?;
        self.hint = t;
        self.legalize(vec![(t, 2), (t2, 1), (u, 2), (u2, 1)])?;
        Ok(v)
    }

    /// Replaces the edge `i` of `t` by the other diagonal of the
    /// quadrilateral it and its neighbour form. The caller has checked
    /// that the quadrilateral is strictly convex.
    fn flip(&mut self, t: u32, i: usize) -> Result<u32, CdtError> {
        let tri = self.t(t)?;
        let u = pick(tri.n, i);
        let other = self.t(u)?;
        let j = self.back(u, t)?;
        let (p, a, b) = (pick(tri.v, i), pick(tri.v, i + 1), pick(tri.v, i + 2));
        let d = pick(other.v, j);
        let (n_bp, c_bp) = (pick(tri.n, i + 1), pick(tri.c, i + 1));
        let (n_pa, c_pa) = (pick(tri.n, i + 2), pick(tri.c, i + 2));
        let (n_ad, c_ad) = (pick(other.n, j + 1), pick(other.c, j + 1));
        let (n_db, c_db) = (pick(other.n, j + 2), pick(other.c, j + 2));
        self.write(t, [p, a, d], [n_ad, u, n_pa], [c_ad, 0, c_pa])?;
        self.write(u, [p, d, b], [n_db, n_bp, t], [c_db, c_bp, 0])?;
        self.relink(n_ad, u, t)?;
        self.relink(n_bp, t, u)?;
        Ok(u)
    }

    /// Lawson's flips: every edge on the stack that is not a constraint
    /// and certainly not Delaunay is flipped, and the edges that exposes
    /// are looked at in turn.
    fn legalize(&mut self, mut stack: Vec<(u32, usize)>) -> Result<(), CdtError> {
        let mut budget = 256 + 16 * self.tris.len();
        while let Some((t, i)) = stack.pop() {
            let tri = self.t(t)?;
            let u = pick(tri.n, i);
            if u == NONE || pick(tri.c, i) > 0 {
                continue;
            }
            let other = self.t(u)?;
            let j = self.back(u, t)?;
            let (p, a, b) = (pick(tri.v, i), pick(tri.v, i + 1), pick(tri.v, i + 2));
            let d = pick(other.v, j);
            let (pp, pa, pb, pd) = (self.p(p)?, self.p(a)?, self.p(b)?, self.p(d)?);
            if in_circle(pp, pa, pb, pd) > 0.0
                && orient(pp, pa, pd) > 0.0
                && orient(pp, pd, pb) > 0.0
            {
                if budget == 0 {
                    return Ok(()); // Valid as it is, only less Delaunay.
                }
                budget -= 1;
                self.spend(1)?;
                let u = self.flip(t, i)?;
                stack.push((t, 0));
                stack.push((u, 0));
            }
        }
        Ok(())
    }

    /// Adds a boundary point and returns its vertex, which is an existing
    /// one when the very same point was added before.
    pub(crate) fn insert_boundary(&mut self, p: P2) -> Result<u32, CdtError> {
        if !(p.0.is_finite() && p.1.is_finite()) {
            return Err(CdtError::NonFinite);
        }
        match self.locate(p)? {
            Location::Inside(t) => self.add_in_triangle(t, p),
            Location::OnEdge(t, i) => self.add_on_edge(t, i, p),
            Location::Vertex(v) => Ok(v),
            Location::Near => Err(CdtError::Degenerate),
        }
    }

    /// Offers an interior point. It is added when it lies inside the
    /// region, at least `clearance` away from every constraint.
    pub(crate) fn insert_interior(&mut self, p: P2, clearance: f64) -> Result<Interior, CdtError> {
        if !(p.0.is_finite() && p.1.is_finite()) {
            return Err(CdtError::NonFinite);
        }
        let (t, edge) = match self.locate(p)? {
            Location::Inside(t) => (t, None),
            Location::OnEdge(t, i) => (t, Some(i)),
            Location::Vertex(_) | Location::Near => return Ok(Interior::Skipped),
        };
        self.hint = t;
        let tri = self.t(t)?;
        if !tri.inside {
            return Ok(Interior::Skipped);
        }
        // Every edge nearer than the clearance is looked at, in this
        // triangle and in the ones beyond such an edge: a constraint that
        // starts at a corner of this triangle, or lies behind a thin
        // neighbour, is as near as one of its own.
        let limit = clearance * clearance;
        let mut seen: Vec<u32> = vec![t];
        let mut next = 0;
        while let Some(&current) = seen.get(next) {
            next += 1;
            let tri = self.t(current)?;
            for i in 0..3 {
                let (a, b) = (self.p(pick(tri.v, i + 1))?, self.p(pick(tri.v, i + 2))?);
                let on_it = current == t && edge == Some(i);
                if !(on_it || distance_squared_to_segment(p, a, b) < limit) {
                    continue;
                }
                if pick(tri.c, i) > 0 {
                    return Ok(Interior::Skipped);
                }
                let beyond = pick(tri.n, i);
                if beyond != NONE && !seen.contains(&beyond) {
                    if seen.len() >= NEAR_TRIANGLES {
                        // Too crowded here to tell: no point is needed.
                        return Ok(Interior::Skipped);
                    }
                    seen.push(beyond);
                }
            }
        }
        let added = match edge {
            None => self.add_in_triangle(t, p),
            Some(i) => self.add_on_edge(t, i, p),
        };
        match added {
            Ok(v) => Ok(Interior::Added(v)),
            Err(CdtError::Degenerate) => Ok(Interior::Skipped),
            Err(e) => Err(e),
        }
    }

    /// Splits the unconstrained edge `i` of triangle `t` at `p`, a point
    /// on it away from its ends.
    pub(crate) fn split_edge(&mut self, t: u32, i: usize, p: P2) -> Result<u32, CdtError> {
        if !(p.0.is_finite() && p.1.is_finite()) {
            return Err(CdtError::NonFinite);
        }
        if pick(self.t(t)?.c, i) > 0 {
            return Err(CdtError::Degenerate);
        }
        self.add_on_edge(t, i, p)
    }

    /// Splits triangle `t` at `p`, a point certainly inside it.
    pub(crate) fn split_triangle(&mut self, t: u32, p: P2) -> Result<u32, CdtError> {
        if !(p.0.is_finite() && p.1.is_finite()) {
            return Err(CdtError::NonFinite);
        }
        self.add_in_triangle(t, p)
    }

    /// The triangles around vertex `a`, each with the index of `a` in it.
    fn fan(&self, a: u32) -> Result<Vec<(u32, usize)>, CdtError> {
        let start = self
            .vert_tri
            .get(a as usize)
            .copied()
            .filter(|t| *t != NONE)
            .ok_or(CdtError::Internal)?;
        let index_of = |t: u32| -> Result<usize, CdtError> {
            let tri = self.t(t)?;
            (0..3)
                .find(|&k| pick(tri.v, k) == a)
                .ok_or(CdtError::Internal)
        };
        let mut out = Vec::new();
        let mut t = start;
        let budget = self.tris.len() + 1;
        // Counter-clockwise; then, if the square's edge stopped that,
        // clockwise from the start.
        loop {
            let k = index_of(t)?;
            out.push((t, k));
            let next = pick(self.t(t)?.n, k + 1);
            if next == start {
                return Ok(out);
            }
            if next == NONE {
                break;
            }
            if out.len() > budget {
                return Err(CdtError::Internal);
            }
            t = next;
        }
        t = start;
        loop {
            let k = index_of(t)?;
            let next = pick(self.t(t)?.n, k + 2);
            if next == NONE {
                return Ok(out);
            }
            out.push((next, index_of(next)?));
            if out.len() > budget {
                return Err(CdtError::Internal);
            }
            t = next;
        }
    }

    /// [`Self::fan`], its triangles counted as work.
    fn spent_fan(&mut self, a: u32) -> Result<Vec<(u32, usize)>, CdtError> {
        let fan = self.fan(a)?;
        self.spend(fan.len())?;
        Ok(fan)
    }

    /// The triangle with the directed edge `a → b`, and that edge's index.
    fn find_edge(&mut self, a: u32, b: u32) -> Result<Option<(u32, usize)>, CdtError> {
        for (t, k) in self.spent_fan(a)? {
            if pick(self.t(t)?.v, k + 1) == b {
                return Ok(Some((t, k + 2)));
            }
        }
        Ok(None)
    }

    /// How the segment from `a` to `b` leaves `a`.
    fn toward(&mut self, a: u32, b: u32) -> Result<Toward, CdtError> {
        let (pa, pb) = (self.p(a)?, self.p(b)?);
        let fan = self.spent_fan(a)?;
        for &(t, k) in &fan {
            let tri = self.t(t)?;
            let q = pick(tri.v, k + 1);
            if q == b {
                return Ok(Toward::Edge(t, k + 2, q));
            }
        }
        for &(t, k) in &fan {
            let tri = self.t(t)?;
            let (q, r) = (pick(tri.v, k + 1), pick(tri.v, k + 2));
            let (pq, pr) = (self.p(q)?, self.p(r)?);
            let side_q = orient(pa, pb, pq);
            let along = (pq.0 - pa.0) * (pb.0 - pa.0) + (pq.1 - pa.1) * (pb.1 - pa.1);
            let whole = (pb.0 - pa.0) * (pb.0 - pa.0) + (pb.1 - pa.1) * (pb.1 - pa.1);
            if side_q == 0.0 && along > 0.0 && along < whole {
                // A vertex on the segment: the constraint stops there
                // and goes on from it.
                return Ok(Toward::Edge(t, k + 2, q));
            }
            if side_q < 0.0 && orient(pa, pb, pr) > 0.0 {
                return Ok(Toward::Crossing(t, k));
            }
        }
        Err(CdtError::Degenerate)
    }

    /// Makes the segment between vertices `a` and `b` an edge (or a chain
    /// of edges, when vertices lie on it) and marks it as a constraint.
    /// A segment given twice counts twice: the region is the same on both
    /// sides of it.
    pub(crate) fn constrain(&mut self, a: u32, b: u32) -> Result<(), CdtError> {
        if a < FIRST_VERTEX || b < FIRST_VERTEX {
            return Err(CdtError::Internal);
        }
        let mut from = a;
        let mut steps = 0;
        while from != b {
            steps += 1;
            if steps > self.pts.len() + 8 {
                return Err(CdtError::Internal);
            }
            from = self.constrain_step(from, b)?;
        }
        Ok(())
    }

    /// One step of [`Self::constrain`]: from `a` to the first vertex on
    /// the segment, which is returned.
    fn constrain_step(&mut self, a: u32, b: u32) -> Result<u32, CdtError> {
        let mut created: Vec<(u32, u32)> = Vec::new();
        for _ in 0..4 {
            match self.toward(a, b)? {
                Toward::Edge(t, i, end) => {
                    let u = pick(self.t(t)?.n, i);
                    let tri = self.tm(t)?;
                    let count = pick(tri.c, i).saturating_add(1);
                    put(&mut tri.c, i, count);
                    if u != NONE {
                        let j = self.back(u, t)?;
                        let other = self.tm(u)?;
                        let count = pick(other.c, j).saturating_add(1);
                        put(&mut other.c, j, count);
                    }
                    // The flips left edges that need not be Delaunay.
                    let mut stack = Vec::new();
                    for (x, y) in created {
                        if let Some(edge) = self.find_edge(x, y)? {
                            stack.push(edge);
                        }
                    }
                    self.legalize(stack)?;
                    return Ok(end);
                }
                Toward::Crossing(t, k) => {
                    let (crossed, end) = self.crossed_edges(a, b, t, k)?;
                    self.spend(crossed.len())?;
                    self.remove_crossings(a, end, crossed, &mut created)?;
                }
            }
        }
        Err(CdtError::Degenerate)
    }

    /// The edges the segment `a → b` crosses, starting with the one
    /// opposite vertex `k` of triangle `t`, up to the first vertex on the
    /// segment, which is returned too.
    fn crossed_edges(
        &self,
        a: u32,
        b: u32,
        t: u32,
        k: usize,
    ) -> Result<(Vec<(u32, u32)>, u32), CdtError> {
        let (pa, pb) = (self.p(a)?, self.p(b)?);
        let mut crossed = Vec::new();
        let (mut t, mut i) = (t, k);
        for _ in 0..=self.tris.len() {
            let tri = self.t(t)?;
            if pick(tri.c, i) > 0 {
                return Err(CdtError::Crossing);
            }
            // `q` is to the right of the segment, `r` to the left.
            let (q, r) = (pick(tri.v, i + 1), pick(tri.v, i + 2));
            crossed.push((q, r));
            let u = pick(tri.n, i);
            if u == NONE {
                return Err(CdtError::Internal);
            }
            let j = self.back(u, t)?;
            let other = self.t(u)?;
            let s = pick(other.v, j);
            if s == b {
                return Ok((crossed, s));
            }
            let side = orient(pa, pb, self.p(s)?);
            if side == 0.0 {
                return Ok((crossed, s));
            }
            // `other` is (s, r, q): the next edge crossed is (q, s) when
            // s is to the left, and (s, r) when it is to the right.
            t = u;
            i = if side > 0.0 { j + 1 } else { j + 2 };
        }
        Err(CdtError::Internal)
    }

    /// Flips the crossed edges until none of them crosses `a → end`.
    fn remove_crossings(
        &mut self,
        a: u32,
        end: u32,
        crossed: Vec<(u32, u32)>,
        created: &mut Vec<(u32, u32)>,
    ) -> Result<(), CdtError> {
        let (pa, pe) = (self.p(a)?, self.p(end)?);
        let mut budget = 64 + 16 * crossed.len() * crossed.len();
        let mut queue: std::collections::VecDeque<(u32, u32)> = crossed.into();
        while let Some((q, r)) = queue.pop_front() {
            if budget == 0 {
                return Err(CdtError::Degenerate);
            }
            budget -= 1;
            self.spend(1)?;
            let Some((t, i)) = self.find_edge(q, r)? else {
                return Err(CdtError::Internal);
            };
            let tri = self.t(t)?;
            let u = pick(tri.n, i);
            if u == NONE {
                return Err(CdtError::Internal);
            }
            let j = self.back(u, t)?;
            let (p, d) = (pick(tri.v, i), pick(self.t(u)?.v, j));
            let (pp, pd, pq, pr) = (self.p(p)?, self.p(d)?, self.p(q)?, self.p(r)?);
            if !(orient(pp, pq, pd) > 0.0 && orient(pp, pd, pr) > 0.0) {
                queue.push_back((q, r)); // Not convex yet; later.
                continue;
            }
            self.flip(t, i)?;
            let still_crossing = p != a
                && p != end
                && d != a
                && d != end
                && orient(pa, pe, pp) * orient(pa, pe, pd) < 0.0;
            if still_crossing {
                queue.push_back((p, d));
            } else {
                created.push((p, d));
            }
        }
        Ok(())
    }

    /// Decides which triangles are in the region: those an odd number of
    /// constraints away from the bounding square. Returns how many.
    pub(crate) fn classify(&mut self) -> Result<usize, CdtError> {
        // 0 unknown, 1 outside, 2 inside.
        let mut state = vec![0_u8; self.tris.len()];
        let mut queue = std::collections::VecDeque::new();
        for (index, tri) in self.tris.iter().enumerate() {
            if tri.n.contains(&NONE) {
                if let Some(slot) = state.get_mut(index) {
                    *slot = 1;
                }
                queue.push_back(index as u32);
            }
        }
        let mut inside = 0;
        while let Some(t) = queue.pop_front() {
            let tri = self.t(t)?;
            let here = state.get(t as usize).copied().unwrap_or(0);
            for i in 0..3 {
                let u = pick(tri.n, i);
                if u == NONE {
                    continue;
                }
                let odd = pick(tri.c, i) % 2 == 1;
                let there = if odd { 3 - here } else { here };
                let slot = state.get_mut(u as usize).ok_or(CdtError::Internal)?;
                if *slot == 0 {
                    *slot = there;
                    queue.push_back(u);
                } else if *slot != there {
                    return Err(CdtError::Open);
                }
            }
        }
        for (tri, s) in self.tris.iter_mut().zip(&state) {
            tri.inside = *s == 2;
            inside += usize::from(tri.inside);
        }
        Ok(inside)
    }
}

/// The triangles of a region given by points and closed chains of segments
/// between them (index pairs), with extra interior points. Triangles are
/// counter-clockwise index triples into the returned points, which are the
/// distinct input points followed by the interior points that were kept;
/// `map` gives the returned index of each input point.
#[cfg(test)]
pub(crate) struct Triangulation {
    pub points: Vec<P2>,
    pub triangles: Vec<[usize; 3]>,
    pub map: Vec<usize>,
}

#[cfg(test)]
pub(crate) fn triangulate(
    points: &[P2],
    segments: &[(usize, usize)],
    interior: &[P2],
) -> Result<Triangulation, CdtError> {
    let (mut min, mut max) = (
        (f64::INFINITY, f64::INFINITY),
        (f64::NEG_INFINITY, f64::NEG_INFINITY),
    );
    for p in points.iter().chain(interior) {
        if !(p.0.is_finite() && p.1.is_finite()) {
            return Err(CdtError::NonFinite);
        }
        min = (min.0.min(p.0), min.1.min(p.1));
        max = (max.0.max(p.0), max.1.max(p.1));
    }
    let mut cdt = Cdt::new(min, max, points.len() + interior.len())?;
    let mut vertex = Vec::with_capacity(points.len());
    for &p in points {
        vertex.push(cdt.insert_boundary(p)?);
    }
    for &(a, b) in segments {
        let (Some(&a), Some(&b)) = (vertex.get(a), vertex.get(b)) else {
            return Err(CdtError::Internal);
        };
        cdt.constrain(a, b)?;
    }
    if cdt.classify()? == 0 {
        return Err(CdtError::Degenerate);
    }
    for &p in interior {
        cdt.insert_interior(p, 0.0)?;
    }
    let first = FIRST_VERTEX as usize;
    let mut triangles = Vec::new();
    for t in 0..cdt.triangle_count() {
        if let Some(view) = cdt.triangle(t).filter(|view| view.inside) {
            triangles.push(view.v.map(|v| v as usize - first));
        }
    }
    Ok(Triangulation {
        points: cdt.pts.iter().skip(first).copied().collect(),
        triangles,
        map: vertex.iter().map(|v| *v as usize - first).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An interior point keeps its clearance from every constraint, not
    /// only from those of the triangle it falls in: next to a corner of
    /// a hole it falls in a triangle of the fan around that corner, none
    /// of whose own edges is a constraint. (Such a point just above a
    /// chord of a hole made a triangle that stands inside out on a curved
    /// face, which refinement then never got right.)
    #[test]
    fn interior_points_keep_their_clearance_from_every_constraint() {
        let outer = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)];
        let hole = [(4.0, 4.0), (4.0, 6.0), (6.0, 6.0), (6.0, 4.0)];
        let (points, segments) = rings(&[&outer, &hole]);
        let mut cdt = Cdt::new((0.0, 0.0), (10.0, 10.0), 100_000).unwrap();
        let ids: Vec<u32> = points
            .iter()
            .map(|p| cdt.insert_boundary(*p).unwrap())
            .collect();
        for &(i, j) in &segments {
            cdt.constrain(ids[i], ids[j]).unwrap();
        }
        assert!(cdt.classify().unwrap() > 0);
        let clearance = 0.5;
        let mut added = 0;
        for i in 0..15 {
            for j in 0..15 {
                // Off the lines through the corners.
                let p = (f64::from(i) * 0.7 + 0.013, f64::from(j) * 0.7 + 0.007);
                let nearest = segments
                    .iter()
                    .map(|&(a, b)| distance_squared_to_segment(p, points[a], points[b]).sqrt())
                    .fold(f64::INFINITY, f64::min);
                let in_the_hole = p.0 > 4.0 && p.0 < 6.0 && p.1 > 4.0 && p.1 < 6.0;
                let wanted = nearest >= clearance && !in_the_hole;
                let got = matches!(
                    cdt.insert_interior(p, clearance).unwrap(),
                    Interior::Added(_)
                );
                assert_eq!(got, wanted, "{p:?}, {nearest} from a constraint");
                added += usize::from(got);
            }
        }
        assert!(added > 100, "{added}");
    }

    /// Points and segments of closed polygons, one after the other.
    fn rings(polygons: &[&[P2]]) -> (Vec<P2>, Vec<(usize, usize)>) {
        let mut points = Vec::new();
        let mut segments = Vec::new();
        for polygon in polygons {
            let base = points.len();
            for (i, p) in polygon.iter().enumerate() {
                points.push(*p);
                segments.push((base + i, base + (i + 1) % polygon.len()));
            }
        }
        (points, segments)
    }

    fn shoelace(polygon: &[P2]) -> f64 {
        (0..polygon.len())
            .map(|i| {
                let (a, b) = (polygon[i], polygon[(i + 1) % polygon.len()]);
                a.0 * b.1 - b.0 * a.1
            })
            .sum::<f64>()
            / 2.0
    }

    fn area(t: &Triangulation) -> f64 {
        t.triangles
            .iter()
            .map(|&[a, b, c]| {
                let (a, b, c) = (t.points[a], t.points[b], t.points[c]);
                ((b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)) / 2.0
            })
            .sum()
    }

    /// Every triangle is counter-clockwise with non-zero area, every edge
    /// is used by at most one triangle per direction, and an edge used
    /// only once is one of the segments: so the triangles tile exactly
    /// the region the segments bound, without overlap.
    fn assert_tiles(t: &Triangulation, segments: &[(usize, usize)], what: &str) {
        let mut edges = std::collections::BTreeMap::new();
        for &[a, b, c] in &t.triangles {
            let (pa, pb, pc) = (t.points[a], t.points[b], t.points[c]);
            let twice = (pb.0 - pa.0) * (pc.1 - pa.1) - (pb.1 - pa.1) * (pc.0 - pa.0);
            assert!(twice > 0.0, "{what}: triangle {a} {b} {c} has area {twice}");
            for e in [(a, b), (b, c), (c, a)] {
                assert!(
                    edges.insert(e, ()).is_none(),
                    "{what}: edge {e:?} used twice"
                );
            }
        }
        let boundary: std::collections::BTreeSet<(usize, usize)> = segments
            .iter()
            .flat_map(|&(a, b)| {
                let (a, b) = (t.map[a], t.map[b]);
                [(a, b), (b, a)]
            })
            .collect();
        for &(a, b) in edges.keys() {
            if !edges.contains_key(&(b, a)) {
                assert!(
                    boundary.contains(&(a, b)),
                    "{what}: open edge {a} {b} is not on the boundary"
                );
            }
        }
        // Every segment is an edge of the result.
        for &(a, b) in segments {
            let (a, b) = (t.map[a], t.map[b]);
            assert!(
                a == b || edges.contains_key(&(a, b)) || edges.contains_key(&(b, a)),
                "{what}: segment {a} {b} is missing"
            );
        }
    }

    const ELL: [P2; 6] = [
        (0.0, 0.0),
        (2.0, 0.0),
        (2.0, 1.0),
        (1.0, 1.0),
        (1.0, 2.0),
        (0.0, 2.0),
    ];

    const COMB: [P2; 16] = [
        (0.0, 0.0),
        (7.0, 0.0),
        (7.0, 3.0),
        (6.0, 3.0),
        (6.0, 1.0),
        (5.0, 1.0),
        (5.0, 3.0),
        (4.0, 3.0),
        (4.0, 1.0),
        (3.0, 1.0),
        (3.0, 3.0),
        (2.0, 3.0),
        (2.0, 1.0),
        (1.0, 1.0),
        (1.0, 3.0),
        (0.0, 3.0),
    ];

    #[test]
    fn convex_and_concave_polygons_are_covered_exactly() {
        let square = [(0.0, 0.0), (2.0, 0.0), (2.0, 2.0), (0.0, 2.0)];
        let (points, segments) = rings(&[&square]);
        let t = triangulate(&points, &segments, &[]).unwrap();
        assert_eq!(t.triangles.len(), 2);
        assert_eq!(area(&t), 4.0);
        assert_tiles(&t, &segments, "square");

        // An L shape: concave at (1, 1).
        let (points, segments) = rings(&[&ELL]);
        let t = triangulate(&points, &segments, &[]).unwrap();
        assert_eq!(t.triangles.len(), 4);
        assert_eq!(area(&t), 3.0);
        assert_tiles(&t, &segments, "ell");

        // The direction of the boundary does not matter: the region is
        // what the segments enclose, and triangles are counter-clockwise.
        let clockwise: Vec<P2> = ELL.iter().rev().copied().collect();
        let (points, segments) = rings(&[&clockwise]);
        let t = triangulate(&points, &segments, &[]).unwrap();
        assert_eq!(t.triangles.len(), 4);
        assert_eq!(area(&t), 3.0);
        assert_tiles(&t, &segments, "clockwise ell");

        // A comb with deep notches.
        let (points, segments) = rings(&[&COMB]);
        let t = triangulate(&points, &segments, &[]).unwrap();
        assert_eq!(t.triangles.len(), COMB.len() - 2);
        assert!((area(&t) - (7.0 + 4.0 * 2.0)).abs() < 1e-12);
        assert_tiles(&t, &segments, "comb");
    }

    #[test]
    fn collinear_boundary_points_stay_corners_of_triangles() {
        // Midpoints on two sides: every boundary point must be used, or
        // the neighbouring face would see a T-junction there.
        let with_midpoints = [
            (0.0, 0.0),
            (1.0, 0.0),
            (2.0, 0.0),
            (2.0, 2.0),
            (1.0, 2.0),
            (0.0, 2.0),
        ];
        let (points, segments) = rings(&[&with_midpoints]);
        let t = triangulate(&points, &segments, &[]).unwrap();
        assert_eq!(t.triangles.len(), 4);
        assert_eq!(area(&t), 4.0);
        assert_tiles(&t, &segments, "midpoints");

        // A side cut into many collinear pieces, at awkward coordinates.
        let mut polygon: Vec<P2> = (0..=40).map(|i| (0.1 * f64::from(i) + 0.3, 0.7)).collect();
        polygon.push((4.3, 1.9));
        polygon.push((0.3, 1.9));
        let (points, segments) = rings(&[&polygon]);
        let t = triangulate(&points, &segments, &[]).unwrap();
        assert_eq!(t.triangles.len(), polygon.len() - 2);
        assert!((area(&t) - 4.8).abs() < 1e-12);
        assert_tiles(&t, &segments, "cut side");

        // A slanted line of collinear points, where rounding decides.
        let mut polygon: Vec<P2> = (0..=30)
            .map(|i| {
                let s = f64::from(i) / 30.0;
                (0.1 + 0.9 * s, 0.2 + 0.7 * s)
            })
            .collect();
        polygon.push((0.0, 1.0));
        let (points, segments) = rings(&[&polygon]);
        let t = triangulate(&points, &segments, &[]).unwrap();
        assert_eq!(t.triangles.len(), polygon.len() - 2);
        assert!((area(&t) - shoelace(&polygon)).abs() < 1e-12);
        assert_tiles(&t, &segments, "slanted");
    }

    #[test]
    fn holes_are_left_open() {
        let outer = [(0.0, 0.0), (10.0, 0.0), (10.0, 6.0), (0.0, 6.0)];
        let hole_a = [(1.0, 1.0), (1.0, 3.0), (3.0, 3.0), (3.0, 1.0)];
        let hole_b: Vec<P2> = (0..24)
            .map(|i| {
                let a = -std::f64::consts::TAU * f64::from(i) / 24.0;
                (7.0 + 1.5 * a.cos(), 3.0 + 1.5 * a.sin())
            })
            .collect();
        let (points, segments) = rings(&[&outer, &hole_a, &hole_b]);
        let t = triangulate(&points, &segments, &[]).unwrap();
        let expected = 60.0 - 4.0 + shoelace(&hole_b);
        assert!((area(&t) - expected).abs() < 1e-12, "{}", area(&t));
        // n + 2 (holes − 1) triangles for n boundary points.
        assert_eq!(t.triangles.len(), points.len() + 2);
        assert_tiles(&t, &segments, "holes");

        // An island inside a hole is part of the region again.
        let island = [(1.5, 1.5), (2.5, 1.5), (2.5, 2.5), (1.5, 2.5)];
        let (points, segments) = rings(&[&outer, &hole_a, &island]);
        let t = triangulate(&points, &segments, &[]).unwrap();
        assert!((area(&t) - (60.0 - 4.0 + 1.0)).abs() < 1e-12);
        assert_tiles(&t, &segments, "island");
    }

    #[test]
    fn interior_points_refine_without_changing_the_region() {
        let (points, segments) = rings(&[&ELL]);
        let interior = [
            (0.5, 0.5),
            (1.5, 0.5),
            (0.5, 1.5),
            (0.5, 1.0),
            (1.5, 1.5),  // outside the L: skipped
            (1.0, 0.0),  // on the boundary: skipped
            (0.5, 0.5),  // a second time: skipped
            (-3.0, 0.5), // far outside: skipped
        ];
        let t = triangulate(&points, &segments, &interior).unwrap();
        assert_eq!(t.points.len(), 6 + 4);
        assert_eq!(t.triangles.len(), 4 + 2 * 4);
        assert_eq!(area(&t), 3.0);
        assert_tiles(&t, &segments, "ell with points");
    }

    #[test]
    fn duplicate_points_share_one_vertex() {
        // Two squares that touch at a corner, given as two rings: the
        // corner appears twice.
        let a = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)];
        let b = [(1.0, 1.0), (2.0, 1.0), (2.0, 2.0), (1.0, 2.0)];
        let (points, segments) = rings(&[&a, &b]);
        let t = triangulate(&points, &segments, &[]).unwrap();
        assert_eq!(t.points.len(), 7);
        assert_eq!(t.map[2], t.map[4]);
        assert_eq!(area(&t), 2.0);
        assert_tiles(&t, &segments, "touching squares");

        // A slit: a segment walked there and back encloses nothing and
        // does not cut the region in two.
        let outer = [(0.0, 0.0), (4.0, 0.0), (4.0, 4.0), (0.0, 4.0)];
        let (mut points, mut segments) = rings(&[&outer]);
        points.extend([(1.0, 2.0), (3.0, 2.0)]);
        segments.extend([(4, 5), (5, 4)]);
        let t = triangulate(&points, &segments, &[]).unwrap();
        assert_eq!(area(&t), 16.0);
        assert_eq!(t.triangles.len(), 6);
    }

    #[test]
    fn degenerate_input_is_an_error() {
        let none: [(usize, usize); 0] = [];
        assert_eq!(
            triangulate(&[], &none, &[]).err(),
            Some(CdtError::NonFinite)
        );
        // No area at all.
        let (points, segments) = rings(&[&[(0.0, 0.0), (1.0, 0.0)]]);
        assert!(triangulate(&points, &segments, &[]).is_err());
        let (points, segments) = rings(&[&[(0.0, 0.0), (1.0, 0.0), (2.0, 0.0)]]);
        assert!(triangulate(&points, &segments, &[]).is_err(), "zero area");
        let (points, segments) = rings(&[&[(0.5, 0.5), (0.5, 0.5), (0.5, 0.5)]]);
        assert!(triangulate(&points, &segments, &[]).is_err(), "one point");
        // Not a number.
        let (points, segments) = rings(&[&[(0.0, 0.0), (f64::NAN, 0.0), (1.0, 1.0)]]);
        assert_eq!(
            triangulate(&points, &segments, &[]).err(),
            Some(CdtError::NonFinite)
        );
        let (points, segments) = rings(&[&[(0.0, 0.0), (f64::INFINITY, 0.0), (1.0, 1.0)]]);
        assert_eq!(
            triangulate(&points, &segments, &[]).err(),
            Some(CdtError::NonFinite)
        );
        // A bow tie: its two sides cross.
        let (points, segments) = rings(&[&[(0.0, 0.0), (2.0, 2.0), (2.0, 0.0), (0.0, 2.0)]]);
        assert_eq!(
            triangulate(&points, &segments, &[]).err(),
            Some(CdtError::Crossing)
        );
        // An open chain bounds nothing.
        let points = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0)];
        assert!(triangulate(&points, &[(0, 1), (1, 2)], &[]).is_err());
        // An open chain hanging off a closed ring.
        let points = [(0.0, 0.0), (4.0, 0.0), (2.0, 4.0), (2.0, 1.0), (2.0, 2.0)];
        let t = triangulate(&points, &[(0, 1), (1, 2), (2, 0), (3, 4)], &[]);
        // A dangling segment inside a region has the region on both
        // sides: that cannot be told from a hole's edge, so it is refused.
        assert_eq!(t.err(), Some(CdtError::Open));
        // A segment to a point that does not exist.
        let points = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0)];
        assert_eq!(
            triangulate(&points, &[(0, 1), (1, 2), (2, 7)], &[]).err(),
            Some(CdtError::Internal)
        );
    }

    #[test]
    fn many_cocircular_points_triangulate() {
        // The worst case for an in-circle test: every answer is "zero".
        for n in [3_usize, 4, 5, 64, 1000] {
            let circle: Vec<P2> = (0..n)
                .map(|i| {
                    let a = std::f64::consts::TAU * i as f64 / n as f64;
                    (3.0 + 2.0 * a.cos(), -1.0 + 2.0 * a.sin())
                })
                .collect();
            let (points, segments) = rings(&[&circle]);
            let t = triangulate(&points, &segments, &[]).unwrap();
            assert_eq!(t.triangles.len(), n - 2, "{n} points");
            assert!((area(&t) - shoelace(&circle)).abs() < 1e-11, "{n} points");
            assert_tiles(&t, &segments, "circle");
        }
    }

    #[test]
    fn a_regular_grid_of_interior_points_triangulates() {
        let outer = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)];
        let (points, segments) = rings(&[&outer]);
        let n = 20;
        let mut interior = Vec::new();
        for i in 1..n {
            for j in 1..n {
                interior.push((f64::from(i) / f64::from(n), f64::from(j) / f64::from(n)));
            }
        }
        let t = triangulate(&points, &segments, &interior).unwrap();
        assert_eq!(t.points.len(), 4 + interior.len());
        assert_eq!(t.triangles.len(), 2 + 2 * interior.len());
        assert!((area(&t) - 1.0).abs() < 1e-12);
        assert_tiles(&t, &segments, "grid");
    }

    /// xorshift64*: deterministic pseudo-random numbers without a crate.
    struct Random(u64);

    impl Random {
        fn next(&mut self) -> f64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            (self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1_u64 << 53) as f64
        }

        fn between(&mut self, a: f64, b: f64) -> f64 {
            a + (b - a) * self.next()
        }
    }

    /// A star-shaped (hence simple) polygon around `centre`: random radii
    /// at increasing angles. Counter-clockwise, or clockwise on request.
    fn star(
        random: &mut Random,
        centre: P2,
        radius: (f64, f64),
        n: usize,
        clockwise: bool,
    ) -> Vec<P2> {
        let mut polygon: Vec<P2> = (0..n)
            .map(|i| {
                let a = std::f64::consts::TAU * (i as f64 + random.between(0.1, 0.9)) / n as f64;
                let r = random.between(radius.0, radius.1);
                (centre.0 + r * a.cos(), centre.1 + r * a.sin())
            })
            .collect();
        if clockwise {
            polygon.reverse();
        }
        polygon
    }

    /// Random simple polygons with random holes and interior points: the
    /// triangles tile the region (no overlap, nothing missing) and their
    /// total area is the polygon's minus the holes'.
    #[test]
    fn random_polygons_with_holes_and_points_are_tiled_exactly() {
        let mut random = Random(0x9E37_79B9_7F4A_7C15);
        for case in 0..400 {
            // Sizes and positions over many orders of magnitude.
            let scale = 10.0_f64.powf(random.between(-6.0, 6.0).round());
            let offset = (
                scale * random.between(-50.0, 50.0),
                scale * random.between(-50.0, 50.0),
            );
            let n = 8 + (random.next() * 40.0) as usize;
            let outer = star(
                &mut random,
                offset,
                (4.0 * scale, 10.0 * scale),
                n,
                case % 2 == 1,
            );
            // Holes on a ring well inside the smallest outer radius, each
            // in its own sector so they cannot touch.
            let holes = (random.next() * 4.0) as usize;
            let mut polygons = vec![outer];
            for h in 0..holes {
                let a = std::f64::consts::TAU * (h as f64 + 0.5) / holes as f64;
                let centre = (
                    offset.0 + 1.5 * scale * a.cos(),
                    offset.1 + 1.5 * scale * a.sin(),
                );
                let m = 4 + (random.next() * 12.0) as usize;
                polygons.push(star(
                    &mut random,
                    centre,
                    (0.3 * scale, 0.9 * scale),
                    m,
                    case % 3 != 0,
                ));
            }
            let refs: Vec<&[P2]> = polygons.iter().map(Vec::as_slice).collect();
            let (points, segments) = rings(&refs);
            let interior: Vec<P2> = (0..(random.next() * 60.0) as usize)
                .map(|_| {
                    (
                        offset.0 + scale * random.between(-11.0, 11.0),
                        offset.1 + scale * random.between(-11.0, 11.0),
                    )
                })
                .collect();
            let what = format!("case {case} (scale {scale:e}, {n} corners, {holes} holes)");
            let t = triangulate(&points, &segments, &interior)
                .unwrap_or_else(|e| panic!("{what}: {e:?}"));
            assert_tiles(&t, &segments, &what);
            let expected = polygons[0..1]
                .iter()
                .chain(&polygons[1..])
                .enumerate()
                .map(|(k, p)| {
                    let a = shoelace(p).abs();
                    if k == 0 { a } else { -a }
                })
                .sum::<f64>();
            assert!(
                (area(&t) - expected).abs() <= 1e-9 * expected,
                "{what}: area {} vs {expected}",
                area(&t)
            );
            // Euler: triangles = 2 × interior points kept + boundary
            // points + 2 (holes − 1).
            let kept = t.points.len() - points.len();
            assert_eq!(
                t.triangles.len(),
                2 * kept + points.len() + 2 * holes - 2,
                "{what}"
            );
        }
    }

    /// Whatever is thrown at it, the triangulator answers or refuses; it
    /// does not panic, and what it answers is a valid tiling.
    #[test]
    fn random_garbage_never_panics() {
        let mut random = Random(0xD1B5_4A32_D192_ED03);
        let mut answered = 0;
        for _ in 0..600 {
            let n = (random.next() * 12.0) as usize;
            let grid = random.next() < 0.5;
            let coordinate = |random: &mut Random| {
                let x = random.between(-2.0, 2.0);
                // Snapped to a coarse grid: many collinear and equal points.
                if grid { (x * 2.0).round() / 2.0 } else { x }
            };
            let points: Vec<P2> = (0..n)
                .map(|_| (coordinate(&mut random), coordinate(&mut random)))
                .collect();
            let segments: Vec<(usize, usize)> = (0..n).map(|i| (i, (i + 1) % n.max(1))).collect();
            let interior: Vec<P2> = (0..4)
                .map(|_| (coordinate(&mut random), coordinate(&mut random)))
                .collect();
            if let Ok(t) = triangulate(&points, &segments, &interior) {
                answered += 1;
                for &[a, b, c] in &t.triangles {
                    assert!(orient(t.points[a], t.points[b], t.points[c]) > 0.0);
                }
            }
        }
        // Most random closed chains cross themselves, but not all.
        assert!(answered > 20, "only {answered} were triangulated");
    }

    /// The points of a long, thin strip (the land of a thread laid out
    /// in its parameters: forty turns side by side), in the order the
    /// tessellator inserts boundary points, which jumps along the whole
    /// strip at every step.
    fn strip(count: usize) -> Vec<P2> {
        let mut random = Random(0x9E37_79B9_7F4A_7C15);
        let points: Vec<P2> = (0..count)
            .map(|i| {
                let along = 1000.0 * i as f64 / count as f64;
                (along, random.between(0.0, 1.0) + 0.02 * along)
            })
            .collect();
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
        (0..count).map(|n| points[(n * stride) % count]).collect()
    }

    /// Where a walk runs out of steps, a walk from a vertex near the
    /// point gives the answer looking at every triangle would give, so
    /// the triangulation is the same either way; and on a long, thin
    /// strip in jumping order the work per point stays far below the
    /// work of looking at every triangle (before, every point there cost
    /// as much as all the points before it).
    #[test]
    fn a_walk_that_runs_out_of_steps_starts_again_near_its_point() {
        let points = strip(20_000);
        let (min, max) = ((0.0, 0.0), (1000.0, 21.0));
        let mut cdt = Cdt::new(min, max, points.len()).unwrap();
        let mut fallbacks = 0;
        for (n, &p) in points.iter().enumerate() {
            if n % 97 == 0 {
                // The answer the scan gives and the one from the grid
                // agree wherever the grid gives one.
                let scanned = cdt.scan(p).unwrap();
                if let Some(found) = cdt.locate_from_grid(p).unwrap() {
                    fallbacks += 1;
                    let same = match (found, scanned) {
                        (Location::Inside(a), Location::Inside(b)) => a == b,
                        (Location::OnEdge(a, i), Location::OnEdge(b, j)) => a == b && i == j,
                        (Location::Vertex(a), Location::Vertex(b)) => a == b,
                        (Location::Near, Location::Near) => true,
                        _ => false,
                    };
                    assert!(same, "point {n}");
                }
            }
            cdt.insert_boundary(p).unwrap();
        }
        assert!(fallbacks > 100, "{fallbacks}");
        let per_point = cdt.work as f64 / points.len() as f64;
        assert!(per_point < 2000.0, "{per_point} steps per point");
    }

    /// Work beyond what the points so far allow is an error, not more
    /// time: a triangulation that does not converge says so.
    #[test]
    fn work_beyond_the_budget_is_an_error() {
        let mut cdt = Cdt::new((0.0, 0.0), (1.0, 1.0), 100).unwrap();
        for p in [(0.2, 0.2), (0.8, 0.3), (0.5, 0.9)] {
            cdt.insert_boundary(p).unwrap();
        }
        let allowed = cdt.allowance();
        assert!(allowed >= WORK_BASE as u64);
        assert_eq!(cdt.spend((allowed - cdt.work) as usize), Ok(()));
        assert_eq!(cdt.spend(1), Err(CdtError::TooSlow));
        // And it stays an error for whatever comes next.
        assert_eq!(cdt.insert_boundary((0.4, 0.4)), Err(CdtError::TooSlow));
    }
}
