//! The St Venant torsion of a section: its torsion constant `J`, its shear
//! centre and its warping constant `Cw`, for any planar section (the cut
//! [`below`](super::below) makes), with any number of holes and of
//! separate pieces (docs/design/sections-and-levels.md, *Torsion*).
//!
//! Theory (Timoshenko & Goodier, *Theory of Elasticity*, ch. 10; Pilkey,
//! *Analysis and Design of Elastic Beams*, ch. 6). With coordinates `x, y`
//! in the section from its centroid, a bar twisted by `θ` per length
//! warps out of its plane by `θ ψ(x, y)`, where the warping function `ψ`
//! is harmonic in the section, `Δψ = 0`, with the Neumann condition
//! `∂ψ/∂n = y nₓ − x n_y` on every boundary, outer and holes alike (so a
//! section with holes needs no extra constants). Its weak form is
//! `a(ψ, v) = f(v)` for every `v`, with `a(ψ, v) = ∫ ∇ψ · ∇v dA` and
//! `f(v) = ∫ (y ∂v/∂x − x ∂v/∂y) dA` (the boundary term moved inside by the
//! divergence theorem), and the torsion constant is
//! `J = ∫ (x² + y² + x ∂ψ/∂y − y ∂ψ/∂x) dA = I_p − a(ψ, ψ)`, with `I_p`
//! the polar moment. A Galerkin solution `ψ_h` has `a(ψ_h, ψ_h) ≤ a(ψ, ψ)`,
//! so `J_h ≥ J` on the meshed region, and `J_h − J = a(ψ − ψ_h, ψ − ψ_h)`,
//! the square of the error in the energy norm: `J` converges twice as
//! fast as the warping function.
//!
//! The shear centre is Trefftz's: the pole about which the warping
//! function `ψ_S = ψ − y_s x + x_s y + c` (a twist about `(x_s, y_s)`) is
//! orthogonal to `x` and `y`, `∫ ψ_S x dA = ∫ ψ_S y dA = 0`, two linear
//! equations in `(x_s, y_s)` with the section's second moments; it does
//! not depend on Poisson's ratio. The warping constant is
//! `Cw = ∫ ψ_S² dA` with `ψ_S` of mean zero.
//!
//! Method. Quadratic (six-node) triangles, isoparametric: the middle node
//! of an edge on the boundary lies on the exact curve (at the middle of
//! its parameter span), so a circle's or a spline's boundary is followed
//! to the fourth order rather than cut by chords. The triangles are the
//! constrained Delaunay triangulation of the boundary points and interior
//! points (`craft3d_tess::triangulate_region`, handed in by the caller as
//! [`Mesher`], since the kernel does not depend on the tessellator). Each
//! element is integrated by Dunavant's 12-point rule (exact to degree 6);
//! the linear system, with one node of every connected piece held at zero
//! (the constant `ψ` is free), by conjugate gradients with a Jacobi
//! preconditioner. Refinement is adaptive: the residual error indicator
//! (the jumps of `∂ψ_h/∂n` across interior edges and the residual of the
//! Neumann condition on boundary edges, each weighted by the edge's
//! length; Verfürth, *A Review of A Posteriori Error Estimation*, 1996)
//! marks the elements that carry half of the estimated error (Dörfler
//! marking), every edge of a marked element is halved (a boundary edge at
//! the middle of its parameter span, on the curve), and the points are
//! triangulated again. It stops when `J` and the shear centre change by
//! less than the relative tolerance from one round to the next, and the
//! meshed area agrees with the exact one to it; or, with `converged`
//! false and what was reached, when the element cap or the round cap is
//! reached.

use super::{BelowPlane, CutError, CutPlane, Part, below_parts, prepare};
use craft3d_geom::Curve3;
use craft3d_math::{Tolerance, Vec3};
use craft3d_topo::Body;
use std::collections::{BTreeMap, BTreeSet};
use std::f64::consts::PI;

/// A point of the section's plane, in its axes from its centroid.
type P2 = (f64, f64);

/// A triangulated region of the plane, as a [`Mesher`] gives it.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct RegionMesh {
    /// The distinct boundary points, then the interior points kept.
    pub points: Vec<P2>,
    /// Counter-clockwise index triples into `points`.
    pub triangles: Vec<[usize; 3]>,
    /// The index in `points` of each boundary point given.
    pub boundary: Vec<usize>,
    /// The index in `points` of each interior point given, `None` for one
    /// that was skipped.
    pub interior: Vec<Option<usize>>,
}

/// Triangulates a region of the plane: boundary points, closed chains of
/// segments between them (the region by the even–odd rule), interior
/// points and the clearance each keeps from every segment. The boundary
/// must be kept as given (no point moved, no segment split) and every
/// triangle counter-clockwise; `craft3d_tess::triangulate_region` is one.
pub type Mesher<'m> =
    dyn Fn(&[P2], &[(usize, usize)], &[P2], &[f64]) -> Result<RegionMesh, String> + 'm;

/// The relative change of `J` (and of the shear centre, relative to the
/// section's radius of gyration) between two rounds at which the
/// refinement stops, by default.
pub const DEFAULT_TOLERANCE: f64 = 1e-4;
/// The most elements, by default.
pub const DEFAULT_MAX_ELEMENTS: usize = 60_000;
/// The most elements a caller may allow.
pub const MAX_ELEMENTS: usize = 400_000;
/// The most rounds of meshing and solving.
pub const MAX_ROUNDS: usize = 40;
/// The share of the estimated error the marked elements carry.
const MARKED_SHARE: f64 = 0.5;
/// The most boundary points the first mesh takes, and the number of
/// elements its spacing aims at.
const FIRST_BOUNDARY: f64 = 4000.0;
const FIRST_ELEMENTS: f64 = 256.0;
/// An edge piece of the first boundary is halved while the curve's middle
/// lies further than this share of its chord from the chord's middle.
const BOW: f64 = 0.05;
/// An interior point keeps this share of the edge it halves from every
/// boundary segment.
const CLEARANCE: f64 = 0.1;
/// The least Jacobian of an element, relative to that of its straight
/// triangle, below which a curved element is taken as folded.
const LEAST_JACOBIAN: f64 = 0.05;
/// The most halvings of one piece of an edge for the first boundary.
const MAX_DEPTH: u32 = 24;
/// The most boundary points.
const MAX_BOUNDARY: usize = 200_000;

/// How finely the torsion is solved for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TorsionOptions {
    /// The relative change between rounds at which refinement stops.
    pub tolerance: f64,
    /// The most elements.
    pub max_elements: usize,
}

impl Default for TorsionOptions {
    fn default() -> Self {
        TorsionOptions {
            tolerance: DEFAULT_TOLERANCE,
            max_elements: DEFAULT_MAX_ELEMENTS,
        }
    }
}

/// The torsion of a section.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Torsion {
    /// The St Venant torsion constant `J` (length⁴): an upper bound on
    /// the meshed region, `change` from the round before.
    pub constant: f64,
    /// `|J − J of the round before|`.
    pub change: f64,
    /// The shear centre (Trefftz), in space.
    pub shear_centre: Vec3,
    /// The shear centre from the section's centroid, along its x and y
    /// axes.
    pub shear_offset: (f64, f64),
    /// The warping constant `Cw` (length⁶) about the shear centre.
    pub warping: f64,
    /// Whether the refinement stopped by the tolerance; false when it
    /// stopped at the element cap or the round cap, the numbers then
    /// those of the last round.
    pub converged: bool,
    /// The elements of the last mesh, and the rounds solved.
    pub elements: usize,
    pub rounds: usize,
}

/// The section of `bodies` by the plane `{p : p · side.normal = offset}`
/// as [`below`](super::below) measures it, and its torsion. A plane that
/// cuts nothing is an error, and so are sections whose faces overlap.
pub fn section_torsion(
    bodies: &[&Body],
    side: &CutPlane,
    offset: f64,
    tolerance: &Tolerance,
    options: &TorsionOptions,
    mesher: &Mesher<'_>,
) -> Result<(BelowPlane, Torsion), CutError> {
    if !offset.is_finite() {
        return Err(CutError::NotFinite("the offset"));
    }
    if !(options.tolerance.is_finite() && (1e-9..=0.1).contains(&options.tolerance)) {
        return Err(CutError::Torsion(
            "the tolerance must be between 1e-9 and 0.1".into(),
        ));
    }
    if !(1..=MAX_ELEMENTS).contains(&options.max_elements) {
        return Err(CutError::Torsion(format!(
            "the most elements must be between 1 and {MAX_ELEMENTS}"
        )));
    }
    let prepared = prepare(bodies, side.normal)?;
    let (found, parts) = below_parts(&prepared, side, offset, tolerance)?;
    let section = found.section;
    let Some(centroid) = section.centroid.filter(|_| section.area > 0.0) else {
        return Err(CutError::Torsion(
            "the plane cuts nothing: there is no section".into(),
        ));
    };
    let axes = Axes {
        origin: centroid,
        x: side.x_axis,
        y: side.y_axis(),
    };
    let boundary = Boundary::of(&parts, &axes, section.area)?;
    let torsion = solve(boundary, &axes, section.area, options, mesher)?;
    Ok((found, torsion))
}

/// The section's axes: its centroid and its x and y directions.
struct Axes {
    origin: Vec3,
    x: Vec3,
    y: Vec3,
}

impl Axes {
    fn of(&self, p: Vec3) -> P2 {
        let r = p - self.origin;
        (r.dot(self.x), r.dot(self.y))
    }
}

/// A piece of the boundary: from point `a` to point `b` (indices into the
/// boundary's points) along curve `curve` from `ta` to `tb`.
#[derive(Clone, Copy, Debug)]
struct Seg {
    a: usize,
    b: usize,
    curve: usize,
    ta: f64,
    tb: f64,
}

/// The boundary of the section: its curves, points and pieces.
struct Boundary<'a> {
    curves: Vec<&'a Curve3>,
    points: Vec<P2>,
    segs: Vec<Seg>,
}

fn dist(a: P2, b: P2) -> f64 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

fn mid(a: P2, b: P2) -> P2 {
    (0.5 * (a.0 + b.0), 0.5 * (a.1 + b.1))
}

/// Where a curve's pieces start before they are halved for the mesh: the
/// knots of a spline, and stretches of at most π/8 of a circle's or an
/// ellipse's angle.
fn breaks(curve: &Curve3, (t0, t1): (f64, f64)) -> Vec<f64> {
    let mut at = vec![t0];
    match curve {
        Curve3::Line(_) => {}
        Curve3::Circle(_) | Curve3::Ellipse(_) => {
            let n = ((t1 - t0) / (PI / 8.0)).ceil().clamp(1.0, 1024.0) as usize;
            for k in 1..n {
                at.push(t0 + (t1 - t0) * (k as f64 / n as f64));
            }
        }
        Curve3::Nurbs(spline) => {
            for &knot in spline.knots() {
                if knot > t0 && knot < t1 && at.last().is_some_and(|&last| knot > last) {
                    at.push(knot);
                }
            }
        }
    }
    at.push(t1);
    at
}

impl<'a> Boundary<'a> {
    /// The edges of every section face of `parts`, each used once (an
    /// edge between two section faces of one body is inside the section
    /// and left out), cut into pieces no longer than the first spacing
    /// and bowing at most [`BOW`] of their chord.
    fn of(parts: &'a [Part], axes: &Axes, area: f64) -> Result<Self, CutError> {
        let lost = || CutError::Torsion("a face, loop or edge that is not in its body".into());
        // Every edge of the section faces, with how often they use it.
        let mut uses: BTreeMap<(usize, u32), usize> = BTreeMap::new();
        for (index, part) in parts.iter().enumerate() {
            let Some((body, caps)) = &part.cut else {
                continue;
            };
            for &face in caps {
                let face = body.faces.get(face).ok_or_else(lost)?;
                for &loop_id in &face.loops {
                    for coedge in &body.loop_(loop_id).ok_or_else(lost)?.coedges {
                        *uses.entry((index, coedge.edge.0)).or_insert(0) += 1;
                    }
                }
            }
        }
        let mut edges = Vec::new();
        for (&(index, edge), &count) in &uses {
            if count % 2 == 0 {
                continue;
            }
            let (body, _) = parts
                .get(index)
                .and_then(|p| p.cut.as_ref())
                .ok_or_else(lost)?;
            let found = body.edge(craft3d_topo::EdgeId(edge)).ok_or_else(lost)?;
            let (t0, t1) = found.range;
            if !(t0.is_finite() && t1.is_finite() && t0 <= t1) {
                return Err(CutError::Torsion(
                    "an edge whose parameter range is not finite".into(),
                ));
            }
            let start = body.vertex(found.start).ok_or_else(lost)?.point;
            let end = body.vertex(found.end).ok_or_else(lost)?.point;
            edges.push((index, found, (found.start.0, start), (found.end.0, end)));
        }
        // The first spacing: about FIRST_ELEMENTS elements over the area,
        // at most FIRST_BOUNDARY points along the boundary.
        let mut perimeter = 0.0;
        for (_, edge, _, _) in &edges {
            let (t0, t1) = edge.range;
            let mut last = axes.of(edge.curve.eval(t0));
            for k in 1..=32 {
                let p = axes.of(edge.curve.eval(t0 + (t1 - t0) * (k as f64 / 32.0)));
                perimeter += dist(last, p);
                last = p;
            }
        }
        let spacing = (area / FIRST_ELEMENTS)
            .sqrt()
            .max(perimeter / FIRST_BOUNDARY);
        if !(spacing.is_finite() && spacing > 0.0) {
            return Err(CutError::Torsion(
                "the section's boundary is not finite".into(),
            ));
        }
        let mut out = Boundary {
            curves: Vec::new(),
            points: Vec::new(),
            segs: Vec::new(),
        };
        let mut vertices: BTreeMap<(usize, u32), usize> = BTreeMap::new();
        for (index, edge, (sid, start), (eid, end)) in edges {
            let curve = out.curves.len();
            out.curves.push(&edge.curve);
            let mut vertex = |out: &mut Boundary<'a>, id: u32, p: Vec3| {
                *vertices.entry((index, id)).or_insert_with(|| {
                    out.points.push(axes.of(p));
                    out.points.len() - 1
                })
            };
            let first = vertex(&mut out, sid, start);
            let last = vertex(&mut out, eid, end);
            let at = breaks(&edge.curve, edge.range);
            let n = at.len();
            let mut prev = first;
            let mut prev_t = edge.range.0;
            for (k, &t) in at.iter().enumerate().skip(1) {
                let next = if k + 1 == n {
                    last
                } else {
                    out.points.push(axes.of(edge.curve.eval(t)));
                    out.points.len() - 1
                };
                out.halve_into(axes, curve, (prev, prev_t), (next, t), spacing)?;
                prev = next;
                prev_t = t;
            }
        }
        if out.segs.len() < 3 {
            return Err(CutError::Torsion(
                "the section's boundary has fewer than three pieces".into(),
            ));
        }
        Ok(out)
    }

    /// The piece from `a` to `b`, halved until it is no longer than
    /// `spacing` and bows less than [`BOW`] of its chord.
    fn halve_into(
        &mut self,
        axes: &Axes,
        curve: usize,
        a: (usize, f64),
        b: (usize, f64),
        spacing: f64,
    ) -> Result<(), CutError> {
        let mut open = vec![(a, b, 0u32)];
        while let Some(((ia, ta), (ib, tb), depth)) = open.pop() {
            let (Some(&pa), Some(&pb), Some(c)) = (
                self.points.get(ia),
                self.points.get(ib),
                self.curves.get(curve),
            ) else {
                return Err(CutError::Torsion(
                    "a boundary point that is not there".into(),
                ));
            };
            let tm = 0.5 * (ta + tb);
            let pm = axes.of(c.eval(tm));
            let chord = dist(pa, pb);
            let bow = dist(pm, mid(pa, pb));
            let split =
                depth < MAX_DEPTH && tm > ta && tm < tb && (chord > spacing || bow > BOW * chord);
            if !split {
                if ia == ib {
                    return Err(CutError::Torsion(
                        "a closed edge too small to be meshed".into(),
                    ));
                }
                self.segs.push(Seg {
                    a: ia,
                    b: ib,
                    curve,
                    ta,
                    tb,
                });
                continue;
            }
            if self.points.len() >= MAX_BOUNDARY {
                return Err(CutError::Torsion(
                    "the section's boundary takes more points than it may".into(),
                ));
            }
            self.points.push(pm);
            let im = self.points.len() - 1;
            // The first half is taken next, so the pieces stay in order.
            open.push(((im, tm), (ib, tb), depth + 1));
            open.push(((ia, ta), (im, tm), depth + 1));
        }
        Ok(())
    }

    /// The point at the middle of `seg`'s parameter span.
    fn middle(&self, axes: &Axes, seg: &Seg) -> Option<P2> {
        let c = self.curves.get(seg.curve)?;
        let p = axes.of(c.eval(0.5 * (seg.ta + seg.tb)));
        (p.0.is_finite() && p.1.is_finite()).then_some(p)
    }

    /// How far the quadratic through `seg`'s ends and middle strays from
    /// the curve, at a quarter and three quarters of the span.
    fn stray(&self, axes: &Axes, seg: &Seg) -> f64 {
        let (Some(&pa), Some(&pb), Some(pm), Some(c)) = (
            self.points.get(seg.a),
            self.points.get(seg.b),
            self.middle(axes, seg),
            self.curves.get(seg.curve),
        ) else {
            return f64::INFINITY;
        };
        let mut most: f64 = 0.0;
        for (s, wa, wb) in [(0.25, 0.375, -0.125), (0.75, -0.125, 0.375)] {
            let q = axes.of(c.eval(seg.ta + s * (seg.tb - seg.ta)));
            let quad = (
                wa * pa.0 + wb * pb.0 + 0.75 * pm.0,
                wa * pa.1 + wb * pb.1 + 0.75 * pm.1,
            );
            most = most.max(dist(q, quad));
        }
        if most.is_nan() { f64::INFINITY } else { most }
    }
}

/// Dunavant's 12-point rule on the triangle, exact to degree 6: the
/// barycentric coordinates of each point and its weight (summing to 1).
/// D. A. Dunavant, "High degree efficient symmetrical Gaussian quadrature
/// rules for the triangle", IJNME 21 (1985).
const RULE: [(f64, f64, f64, f64); 12] = {
    const A1: f64 = 0.249_286_745_170_910;
    const B1: f64 = 0.501_426_509_658_179;
    const W1: f64 = 0.116_786_275_726_379;
    const A2: f64 = 0.063_089_014_491_502;
    const B2: f64 = 0.873_821_971_016_996;
    const W2: f64 = 0.050_844_906_370_207;
    const A3: f64 = 0.053_145_049_844_817;
    const B3: f64 = 0.310_352_451_033_784;
    const C3: f64 = 0.636_502_499_121_399;
    const W3: f64 = 0.082_851_075_618_374;
    [
        (B1, A1, A1, W1),
        (A1, B1, A1, W1),
        (A1, A1, B1, W1),
        (B2, A2, A2, W2),
        (A2, B2, A2, W2),
        (A2, A2, B2, W2),
        (A3, B3, C3, W3),
        (A3, C3, B3, W3),
        (B3, A3, C3, W3),
        (B3, C3, A3, W3),
        (C3, A3, B3, W3),
        (C3, B3, A3, W3),
    ]
};

/// Three-point Gauss–Legendre on [0, 1]: points and weights.
const EDGE_RULE: [(f64, f64); 3] = [
    (0.112_701_665_379_258_3, 5.0 / 18.0),
    (0.5, 8.0 / 18.0),
    (0.887_298_334_620_741_7, 5.0 / 18.0),
];

/// The six shape functions of the quadratic triangle at reference point
/// `(ξ, η)` (vertices 0, 1, 2 at (0, 0), (1, 0), (0, 1); nodes 3, 4, 5 at
/// the middles of edges 0–1, 1–2, 2–0), and their derivatives in ξ and η.
fn shape(xi: f64, eta: f64) -> ([f64; 6], [f64; 6], [f64; 6]) {
    let (l1, l2, l3) = (1.0 - xi - eta, xi, eta);
    let n = [
        l1 * (2.0 * l1 - 1.0),
        l2 * (2.0 * l2 - 1.0),
        l3 * (2.0 * l3 - 1.0),
        4.0 * l1 * l2,
        4.0 * l2 * l3,
        4.0 * l3 * l1,
    ];
    let dxi = [
        -(4.0 * l1 - 1.0),
        4.0 * l2 - 1.0,
        0.0,
        4.0 * (l1 - l2),
        4.0 * l3,
        -4.0 * l3,
    ];
    let deta = [
        -(4.0 * l1 - 1.0),
        0.0,
        4.0 * l3 - 1.0,
        -4.0 * l2,
        4.0 * l2,
        4.0 * (l1 - l3),
    ];
    (n, dxi, deta)
}

/// An element's map at a reference point: the point, the Jacobian's
/// determinant, its columns `∂X/∂ξ` and `∂X/∂η`, the shape functions and
/// their gradients in x and y.
struct At {
    p: P2,
    det: f64,
    dxi: P2,
    deta: P2,
    n: [f64; 6],
    gx: [f64; 6],
    gy: [f64; 6],
}

fn at(nodes: &[P2; 6], xi: f64, eta: f64) -> At {
    let (n, dxi, deta) = shape(xi, eta);
    let (mut p, mut a, mut b) = ((0.0, 0.0), (0.0, 0.0), (0.0, 0.0));
    for (((q, ni), di), ei) in nodes.iter().zip(n).zip(dxi).zip(deta) {
        p.0 += ni * q.0;
        p.1 += ni * q.1;
        a.0 += di * q.0;
        a.1 += di * q.1;
        b.0 += ei * q.0;
        b.1 += ei * q.1;
    }
    let det = a.0 * b.1 - b.0 * a.1;
    let (mut gx, mut gy) = ([0.0; 6], [0.0; 6]);
    if det != 0.0 {
        for ((x, y), (di, ei)) in gx.iter_mut().zip(gy.iter_mut()).zip(dxi.iter().zip(deta)) {
            *x = (b.1 * di - a.1 * ei) / det;
            *y = (-b.0 * di + a.0 * ei) / det;
        }
    }
    At {
        p,
        det,
        dxi: a,
        deta: b,
        n,
        gx,
        gy,
    }
}

/// Where on the reference triangle a point at `s` along edge `k` (from
/// vertex k to vertex k + 1) lies, and the derivative of that along s.
fn on_edge(k: usize, s: f64) -> ((f64, f64), (f64, f64)) {
    match k {
        0 => ((s, 0.0), (1.0, 0.0)),
        1 => ((1.0 - s, s), (-1.0, 1.0)),
        _ => ((0.0, 1.0 - s), (0.0, -1.0)),
    }
}

/// One element: its six nodes (unknowns) and where they are.
struct Element {
    nodes: [usize; 6],
    at: [P2; 6],
    /// Its three edges by their vertices (smaller first).
    edges: [(usize, usize); 3],
}

/// An edge of the mesh: the elements beside it (with its place in each)
/// and the boundary piece it is, if any.
struct EdgeInfo {
    sides: Vec<(usize, usize)>,
    seg: Option<usize>,
}

/// A mesh of six-node elements.
struct Mesh {
    elements: Vec<Element>,
    edges: BTreeMap<(usize, usize), EdgeInfo>,
    /// The mesh's vertices (the first unknowns), then the edges' middles.
    nodes: usize,
    vertices: usize,
}

/// The sparse symmetric matrix of the system, by rows.
struct Csr {
    start: Vec<usize>,
    col: Vec<usize>,
    val: Vec<f64>,
    diag: Vec<f64>,
}

impl Csr {
    fn from_triplets(n: usize, mut t: Vec<(usize, usize, f64)>) -> Csr {
        t.sort_by_key(|a| (a.0, a.1));
        let mut start = vec![0; n + 1];
        let (mut col, mut val) = (
            Vec::with_capacity(t.len() / 2),
            Vec::with_capacity(t.len() / 2),
        );
        let mut diag = vec![0.0; n];
        let mut last: Option<(usize, usize)> = None;
        for (r, c, v) in t {
            if last == Some((r, c)) {
                if let Some(x) = val.last_mut() {
                    *x += v;
                }
            } else {
                col.push(c);
                val.push(v);
                if let Some(s) = start.get_mut(r + 1) {
                    *s += 1;
                }
                last = Some((r, c));
            }
        }
        for k in 1..=n {
            let before = start.get(k - 1).copied().unwrap_or(0);
            if let Some(s) = start.get_mut(k) {
                *s += before;
            }
        }
        for (r, d) in diag.iter_mut().enumerate() {
            let (a, b) = (
                start.get(r).copied().unwrap_or(0),
                start.get(r + 1).copied().unwrap_or(0),
            );
            for (c, v) in col
                .get(a..b)
                .unwrap_or(&[])
                .iter()
                .zip(val.get(a..b).unwrap_or(&[]))
            {
                if *c == r {
                    *d += v;
                }
            }
        }
        Csr {
            start,
            col,
            val,
            diag,
        }
    }

    fn times(&self, x: &[f64], y: &mut [f64]) {
        for (r, out) in y.iter_mut().enumerate() {
            let (a, b) = (
                self.start.get(r).copied().unwrap_or(0),
                self.start.get(r + 1).copied().unwrap_or(0),
            );
            let mut sum = 0.0;
            for (c, v) in self
                .col
                .get(a..b)
                .unwrap_or(&[])
                .iter()
                .zip(self.val.get(a..b).unwrap_or(&[]))
            {
                sum += v * x.get(*c).copied().unwrap_or(0.0);
            }
            *out = sum;
        }
    }
}

fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// Conjugate gradients with a Jacobi preconditioner, from zero, until the
/// residual is at most `floor`; `None` when it does not get there within
/// its steps.
fn conjugate_gradients(a: &Csr, f: &[f64], floor: f64) -> Option<Vec<f64>> {
    let n = f.len();
    let mut x = vec![0.0; n];
    let mut r = f.to_vec();
    if dot(&r, &r).sqrt() <= floor {
        return Some(x);
    }
    let inv: Vec<f64> = a
        .diag
        .iter()
        .map(|d| if *d > 0.0 { 1.0 / d } else { 1.0 })
        .collect();
    let mut z: Vec<f64> = r.iter().zip(&inv).map(|(r, i)| r * i).collect();
    let mut p = z.clone();
    let mut rz = dot(&r, &z);
    let mut ap = vec![0.0; n];
    let steps = (2 * n + 1000).min(200_000);
    for _ in 0..steps {
        a.times(&p, &mut ap);
        let pap = dot(&p, &ap);
        if !(pap.is_finite() && pap > 0.0) {
            return None;
        }
        let alpha = rz / pap;
        for ((x, r), (p, ap)) in x.iter_mut().zip(r.iter_mut()).zip(p.iter().zip(&ap)) {
            *x += alpha * p;
            *r -= alpha * ap;
        }
        let norm = dot(&r, &r).sqrt();
        if !norm.is_finite() {
            return None;
        }
        if norm <= floor {
            return Some(x);
        }
        for ((z, r), i) in z.iter_mut().zip(&r).zip(&inv) {
            *z = r * i;
        }
        let next = dot(&r, &z);
        let beta = next / rz;
        rz = next;
        for (p, z) in p.iter_mut().zip(&z) {
            *p = z + beta * *p;
        }
    }
    None
}

/// What one round measures.
#[derive(Clone, Copy, Debug)]
struct Round {
    j: f64,
    shear: P2,
    warping: f64,
    area: f64,
}

/// What one solve leaves: the round's numbers and each element's share
/// of the estimated error.
struct Solved {
    round: Round,
    indicator: Vec<f64>,
}

fn solve(
    mut boundary: Boundary<'_>,
    axes: &Axes,
    exact_area: f64,
    options: &TorsionOptions,
    mesher: &Mesher<'_>,
) -> Result<Torsion, CutError> {
    let tol = options.tolerance;
    // Interior points and the clearance each keeps.
    let mut interior: Vec<(P2, f64)> = Vec::new();
    let mut last: Option<(Round, usize)> = None;
    let mut change = f64::INFINITY;
    let mut rounds = 0;
    for _ in 0..MAX_ROUNDS {
        let segments: Vec<(usize, usize)> = boundary.segs.iter().map(|s| (s.a, s.b)).collect();
        let inner: Vec<P2> = interior.iter().map(|(p, _)| *p).collect();
        let clear: Vec<f64> = interior.iter().map(|(_, c)| *c).collect();
        let region = mesher(&boundary.points, &segments, &inner, &clear)
            .map_err(|why| CutError::Torsion(format!("the section cannot be meshed: {why}")))?;
        // The interior points kept, and which of them each vertex is.
        let mut of_vertex: BTreeMap<usize, usize> = BTreeMap::new();
        let mut kept = Vec::with_capacity(interior.len());
        for (point, v) in interior.iter().zip(&region.interior) {
            if let Some(v) = v {
                of_vertex.insert(*v, kept.len());
                kept.push(*point);
            }
        }
        interior = kept;
        let elements = region.triangles.len();
        if elements > options.max_elements {
            break;
        }
        let mesh = build(&region, &boundary, axes)?;
        // Curved elements that fold: their boundary pieces halved, their
        // interior points dropped, and meshed again.
        let folded: Vec<usize> = mesh
            .elements
            .iter()
            .enumerate()
            .filter(|(_, e)| folds(e))
            .map(|(k, _)| k)
            .collect();
        if !folded.is_empty() {
            let mut halve = BTreeSet::new();
            let mut drop = BTreeSet::new();
            for &k in &folded {
                let Some(e) = mesh.elements.get(k) else {
                    continue;
                };
                for key in e.edges {
                    if let Some(seg) = mesh.edges.get(&key).and_then(|i| i.seg) {
                        halve.insert(seg);
                    }
                }
                for v in e.nodes.iter().take(3) {
                    if let Some(&i) = of_vertex.get(v) {
                        drop.insert(i);
                    }
                }
            }
            if halve.is_empty() && drop.is_empty() {
                return Err(CutError::Torsion(
                    "an element folds and cannot be mended".into(),
                ));
            }
            interior = interior
                .into_iter()
                .enumerate()
                .filter(|(i, _)| !drop.contains(i))
                .map(|(_, p)| p)
                .collect();
            split_segs(&mut boundary, axes, &halve)?;
            continue;
        }
        let Some(solved) = solve_once(&mesh) else {
            break;
        };
        rounds += 1;
        let round = solved.round;
        let size = round.area.sqrt();
        let mut done = false;
        if let Some((before, _)) = last {
            change = (round.j - before.j).abs();
            let moved = dist(round.shear, before.shear);
            done = change <= tol * round.j.abs() && moved <= tol * size;
        }
        let area_ok = (round.area - exact_area).abs() <= tol * exact_area;
        last = Some((round, elements));
        if done && area_ok {
            return Ok(answer(round, change, true, elements, rounds, axes));
        }
        // Refine: halve every edge of the marked elements, and where the
        // area is off, the boundary pieces that stray most from the curve.
        let marked = mark(&solved.indicator);
        let mut halve = BTreeSet::new();
        let mut added = 0;
        let mut inner_edges = BTreeSet::new();
        for k in marked {
            let Some(e) = mesh.elements.get(k) else {
                continue;
            };
            for key in e.edges {
                match mesh.edges.get(&key).and_then(|i| i.seg) {
                    Some(seg) => {
                        halve.insert(seg);
                    }
                    None => {
                        inner_edges.insert(key);
                    }
                }
            }
        }
        if !area_ok {
            let strays: Vec<f64> = boundary
                .segs
                .iter()
                .map(|s| boundary.stray(axes, s))
                .collect();
            let most = strays.iter().copied().fold(0.0, f64::max);
            for (k, s) in strays.iter().enumerate() {
                if *s >= 0.25 * most {
                    halve.insert(k);
                }
            }
        }
        for (a, b) in inner_edges {
            let (Some(&pa), Some(&pb)) = (region.points.get(a), region.points.get(b)) else {
                continue;
            };
            interior.push((mid(pa, pb), CLEARANCE * dist(pa, pb)));
            added += 1;
        }
        added += halve.len();
        if added == 0 {
            break;
        }
        split_segs(&mut boundary, axes, &halve)?;
    }
    match last {
        Some((round, elements)) => Ok(answer(round, change, false, elements, rounds, axes)),
        None => Err(CutError::Torsion(format!(
            "no mesh of the section within {} elements",
            options.max_elements
        ))),
    }
}

fn answer(
    round: Round,
    change: f64,
    converged: bool,
    elements: usize,
    rounds: usize,
    axes: &Axes,
) -> Torsion {
    let (sx, sy) = round.shear;
    Torsion {
        constant: round.j,
        change,
        shear_centre: axes.origin + axes.x * sx + axes.y * sy,
        shear_offset: (sx, sy),
        warping: round.warping,
        converged,
        elements,
        rounds,
    }
}

/// Halves the boundary pieces `which` at the middle of their spans.
fn split_segs(
    boundary: &mut Boundary<'_>,
    axes: &Axes,
    which: &BTreeSet<usize>,
) -> Result<(), CutError> {
    let mut segs = Vec::with_capacity(boundary.segs.len() + which.len());
    let old = std::mem::take(&mut boundary.segs);
    for (k, seg) in old.into_iter().enumerate() {
        let tm = 0.5 * (seg.ta + seg.tb);
        let middle = boundary.middle(axes, &seg);
        match middle {
            Some(pm) if which.contains(&k) && tm > seg.ta && tm < seg.tb => {
                if boundary.points.len() >= MAX_BOUNDARY {
                    return Err(CutError::Torsion(
                        "the section's boundary takes more points than it may".into(),
                    ));
                }
                boundary.points.push(pm);
                let m = boundary.points.len() - 1;
                segs.push(Seg {
                    b: m,
                    tb: tm,
                    ..seg
                });
                segs.push(Seg {
                    a: m,
                    ta: tm,
                    ..seg
                });
            }
            _ => segs.push(seg),
        }
    }
    boundary.segs = segs;
    Ok(())
}

/// The six-node mesh of a triangulation: the vertices first, then a node
/// at the middle of every edge, on the curve for a boundary piece.
fn build(region: &RegionMesh, boundary: &Boundary<'_>, axes: &Axes) -> Result<Mesh, CutError> {
    let bad = |why: &str| CutError::Torsion(format!("the mesher's triangles: {why}"));
    let vertices = region.points.len();
    // The boundary pieces by their vertices in the mesh.
    let mut pieces: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    for (k, seg) in boundary.segs.iter().enumerate() {
        let (Some(&a), Some(&b)) = (region.boundary.get(seg.a), region.boundary.get(seg.b)) else {
            return Err(bad("a boundary point is missing"));
        };
        pieces.insert((a.min(b), a.max(b)), k);
    }
    let mut edges: BTreeMap<(usize, usize), EdgeInfo> = BTreeMap::new();
    for (t, tri) in region.triangles.iter().enumerate() {
        for k in 0..3 {
            let (a, b) = match k {
                0 => (tri[0], tri[1]),
                1 => (tri[1], tri[2]),
                _ => (tri[2], tri[0]),
            };
            if a >= vertices || b >= vertices || a == b {
                return Err(bad("a triangle with a corner that is not a point"));
            }
            let key = (a.min(b), a.max(b));
            let info = edges.entry(key).or_insert_with(|| EdgeInfo {
                sides: Vec::new(),
                seg: pieces.get(&key).copied(),
            });
            if info.sides.len() >= 2 {
                return Err(bad("an edge with more than two triangles"));
            }
            info.sides.push((t, k));
        }
    }
    // The middle nodes, numbered in the edges' order.
    let mut middle: BTreeMap<(usize, usize), (usize, P2)> = BTreeMap::new();
    for (n, (key, info)) in edges.iter().enumerate() {
        let (Some(&pa), Some(&pb)) = (region.points.get(key.0), region.points.get(key.1)) else {
            return Err(bad("an edge with a corner that is not a point"));
        };
        let at = match info.seg.and_then(|s| boundary.segs.get(s)) {
            Some(seg) => boundary
                .middle(axes, seg)
                .ok_or_else(|| bad("a boundary curve that is not finite"))?,
            None => mid(pa, pb),
        };
        middle.insert(*key, (vertices + n, at));
    }
    let mut elements = Vec::with_capacity(region.triangles.len());
    for tri in &region.triangles {
        let [a, b, c] = *tri;
        let (Some(&pa), Some(&pb), Some(&pc)) = (
            region.points.get(a),
            region.points.get(b),
            region.points.get(c),
        ) else {
            return Err(bad("a corner that is not a point"));
        };
        let keys = [
            (a.min(b), a.max(b)),
            (b.min(c), b.max(c)),
            (c.min(a), c.max(a)),
        ];
        let [m0, m1, m2] = keys.map(|k| middle.get(&k).copied());
        let (Some(m0), Some(m1), Some(m2)) = (m0, m1, m2) else {
            return Err(bad("an edge without its middle"));
        };
        elements.push(Element {
            nodes: [a, b, c, m0.0, m1.0, m2.0],
            at: [pa, pb, pc, m0.1, m1.1, m2.1],
            edges: keys,
        });
    }
    Ok(Mesh {
        elements,
        nodes: vertices + edges.len(),
        edges,
        vertices,
    })
}

/// Whether a curved element folds: its Jacobian somewhere below
/// [`LEAST_JACOBIAN`] of its straight triangle's.
fn folds(e: &Element) -> bool {
    let [a, b, c, ..] = e.at;
    let straight = (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0);
    let points =
        RULE.iter()
            .map(|(_, l2, l3, _)| (*l2, *l3))
            .chain([(0.0, 0.0), (1.0, 0.0), (0.0, 1.0)]);
    for (xi, eta) in points {
        let det = at(&e.at, xi, eta).det;
        if !(det.is_finite() && det >= LEAST_JACOBIAN * straight) {
            return true;
        }
    }
    false
}

/// Index of the root of `v` in a union-find forest.
fn root(parent: &mut [usize], mut v: usize) -> usize {
    let mut steps = 0;
    while let Some(&p) = parent.get(v) {
        if p == v || steps > parent.len() {
            break;
        }
        let grand = parent.get(p).copied().unwrap_or(p);
        if let Some(slot) = parent.get_mut(v) {
            *slot = grand;
        }
        v = p;
        steps += 1;
    }
    v
}

/// Assembles and solves the warping problem on `mesh`, and measures it.
/// `None` when the linear solver does not converge.
fn solve_once(mesh: &Mesh) -> Option<Solved> {
    let n = mesh.nodes;
    // One vertex of every connected piece is held at zero.
    let mut parent: Vec<usize> = (0..mesh.vertices).collect();
    for e in &mesh.elements {
        let [a, b, c, ..] = e.nodes;
        for (x, y) in [(a, b), (b, c)] {
            let (rx, ry) = (root(&mut parent, x), root(&mut parent, y));
            if rx != ry
                && let Some(slot) = parent.get_mut(rx.max(ry))
            {
                *slot = rx.min(ry);
            }
        }
    }
    let mut held = vec![false; n];
    let mut seen = BTreeSet::new();
    for e in &mesh.elements {
        let v = e.nodes[0];
        if seen.insert(root(&mut parent, v))
            && let Some(h) = held.get_mut(v)
        {
            *h = true;
        }
    }
    let is_held = |i: usize| held.get(i).copied().unwrap_or(false);
    let mut triplets = Vec::with_capacity(mesh.elements.len() * 36);
    let mut f = vec![0.0; n];
    let mut magnitude = 0.0;
    // The section's integrals on the mesh: area, ∫x, ∫y, ∫x², ∫y², ∫xy.
    let mut m = [0.0; 6];
    for e in &mesh.elements {
        let mut ke = [[0.0; 6]; 6];
        let mut fe = [0.0; 6];
        for &(_, l2, l3, w) in &RULE {
            let q = at(&e.at, l2, l3);
            let wd = 0.5 * w * q.det;
            let (x, y) = q.p;
            for (k, v) in m.iter_mut().zip([1.0, x, y, x * x, y * y, x * y]) {
                *k += wd * v;
            }
            for (i, row) in ke.iter_mut().enumerate() {
                let (Some(gxi), Some(gyi)) = (q.gx.get(i), q.gy.get(i)) else {
                    continue;
                };
                for (kij, (gxj, gyj)) in row.iter_mut().zip(q.gx.iter().zip(&q.gy)) {
                    *kij += wd * (gxi * gxj + gyi * gyj);
                }
            }
            for (fi, (gx, gy)) in fe.iter_mut().zip(q.gx.iter().zip(&q.gy)) {
                *fi += wd * (y * gx - x * gy);
                magnitude += wd.abs() * ((y * gx).abs() + (x * gy).abs());
            }
        }
        for (row, (&i, fi)) in ke.iter().zip(e.nodes.iter().zip(fe)) {
            if !is_held(i)
                && let Some(slot) = f.get_mut(i)
            {
                *slot += fi;
            }
            for (&j, kij) in e.nodes.iter().zip(row) {
                if !is_held(i) && !is_held(j) {
                    triplets.push((i, j, *kij));
                }
            }
        }
    }
    for (i, h) in held.iter().enumerate() {
        if *h {
            triplets.push((i, i, 1.0));
        }
    }
    let a = Csr::from_triplets(n, triplets);
    let psi = conjugate_gradients(&a, &f, 1e-11 * magnitude.max(f64::MIN_POSITIVE))?;
    let [area, sx, sy, ixx, iyy, ixy] = m;
    if !(area.is_finite() && area > 0.0) {
        return None;
    }
    let polar = ixx + iyy;
    let j = polar - dot(&psi, &f);
    // Moments about the mesh's own centroid.
    let (cx, cy) = (sx / area, sy / area);
    let i_x = iyy - area * cy * cy;
    let i_y = ixx - area * cx * cx;
    let product = ixy - area * cx * cy;
    let value = |e: &Element, q: &At| -> f64 {
        e.nodes
            .iter()
            .zip(q.n)
            .map(|(&i, ni)| ni * psi.get(i).copied().unwrap_or(0.0))
            .sum()
    };
    let (mut s, mut spx, mut spy) = (0.0, 0.0, 0.0);
    for e in &mesh.elements {
        for &(_, l2, l3, w) in &RULE {
            let q = at(&e.at, l2, l3);
            let wd = 0.5 * w * q.det;
            let v = value(e, &q);
            s += wd * v;
            spx += wd * v * q.p.0;
            spy += wd * v * q.p.1;
        }
    }
    let ipx = spx - cx * s;
    let ipy = spy - cy * s;
    let det = i_x * i_y - product * product;
    if !(det.is_finite() && det > 0.0) {
        return None;
    }
    // ∫ψ_S x' = ipx − y_s i_y + x_s product = 0 and
    // ∫ψ_S y' = ipy − y_s product + x_s i_x = 0.
    let xs = (product * ipx - i_y * ipy) / det;
    let ys = (i_x * ipx - product * ipy) / det;
    let mean = (s - ys * sx + xs * sy) / area;
    let mut warping = 0.0;
    for e in &mesh.elements {
        for &(_, l2, l3, w) in &RULE {
            let q = at(&e.at, l2, l3);
            let wd = 0.5 * w * q.det;
            let v = value(e, &q) - ys * q.p.0 + xs * q.p.1 - mean;
            warping += wd * v * v;
        }
    }
    let indicator = indicators(mesh, &psi);
    let round = Round {
        j,
        shear: (xs, ys),
        warping,
        area,
    };
    let finite = j.is_finite() && xs.is_finite() && ys.is_finite() && warping.is_finite();
    finite.then_some(Solved { round, indicator })
}

/// The gradient of the solution on element `e` at `s` along its edge `k`,
/// with the point, the edge's outward normal and its length element.
fn edge_gradient(e: &Element, psi: &[f64], k: usize, s: f64) -> (P2, P2, P2, f64) {
    let ((xi, eta), (dxi, deta)) = on_edge(k, s);
    let q = at(&e.at, xi, eta);
    let (mut gx, mut gy) = (0.0, 0.0);
    for ((&i, a), b) in e.nodes.iter().zip(q.gx).zip(q.gy) {
        let v = psi.get(i).copied().unwrap_or(0.0);
        gx += a * v;
        gy += b * v;
    }
    // ∂X/∂s from the columns ∂X/∂ξ and ∂X/∂η.
    let t = (
        q.dxi.0 * dxi + q.deta.0 * deta,
        q.dxi.1 * dxi + q.deta.1 * deta,
    );
    let length = t.0.hypot(t.1);
    let normal = if length > 0.0 {
        (t.1 / length, -t.0 / length)
    } else {
        (0.0, 0.0)
    };
    ((gx, gy), q.p, normal, length)
}

/// Each element's squared error indicator: the jumps of the normal
/// derivative across its interior edges (half to each side) and the
/// residual of the Neumann condition on its boundary edges, weighted by
/// the edge's length.
fn indicators(mesh: &Mesh, psi: &[f64]) -> Vec<f64> {
    let mut eta = vec![0.0; mesh.elements.len()];
    for info in mesh.edges.values() {
        match info.sides.as_slice() {
            [(t1, k1), (t2, k2)] => {
                let (Some(e1), Some(e2)) = (mesh.elements.get(*t1), mesh.elements.get(*t2)) else {
                    continue;
                };
                let mut sum = 0.0;
                let mut h = 0.0;
                for (s, w) in EDGE_RULE {
                    let (g1, _, n1, len) = edge_gradient(e1, psi, *k1, s);
                    let (g2, _, _, _) = edge_gradient(e2, psi, *k2, 1.0 - s);
                    let jump = (g1.0 - g2.0) * n1.0 + (g1.1 - g2.1) * n1.1;
                    sum += w * len * jump * jump;
                    h += w * len;
                }
                for t in [t1, t2] {
                    if let Some(slot) = eta.get_mut(*t) {
                        *slot += 0.5 * h * sum;
                    }
                }
            }
            [(t1, k1)] => {
                let Some(e1) = mesh.elements.get(*t1) else {
                    continue;
                };
                let mut sum = 0.0;
                let mut h = 0.0;
                for (s, w) in EDGE_RULE {
                    let (g, p, n, len) = edge_gradient(e1, psi, *k1, s);
                    let wanted = p.1 * n.0 - p.0 * n.1;
                    let r = wanted - (g.0 * n.0 + g.1 * n.1);
                    sum += w * len * r * r;
                    h += w * len;
                }
                if let Some(slot) = eta.get_mut(*t1) {
                    *slot += h * sum;
                }
            }
            _ => {}
        }
    }
    eta
}

/// The elements that carry [`MARKED_SHARE`] of the estimated error, the
/// largest first (ties by index).
fn mark(indicator: &[f64]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..indicator.len()).collect();
    let value = |k: usize| {
        indicator
            .get(k)
            .copied()
            .filter(|v| v.is_finite())
            .unwrap_or(0.0)
    };
    order.sort_by(|&a, &b| value(b).total_cmp(&value(a)).then(a.cmp(&b)));
    let total: f64 = order.iter().map(|&k| value(k)).sum();
    let mut sum = 0.0;
    let mut out = Vec::new();
    for k in order {
        if sum >= MARKED_SHARE * total && !out.is_empty() {
            break;
        }
        sum += value(k);
        out.push(k);
    }
    out
}

#[cfg(test)]
#[path = "torsion_tests.rs"]
mod tests;
