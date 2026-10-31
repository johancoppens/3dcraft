//! The proven least distance between two faces (docs/design/verification.md
//! › *The proven distance*): a branch and bound over the strata of the two
//! faces whose bounds close in on the least distance, with the symmetries
//! of the analytic kinds as shortcuts that are exact by construction.
//!
//! **Strata.** A face is its open interior, its edges' open curves and its
//! vertices. The least distance between two faces is reached at a pair of
//! points each in the relative interior of one stratum, and unless it is
//! 0 that pair is *critical* there: the line between them is normal to a
//! face's surface and square to an edge's tangent. So the least is the
//! least over pairs of strata of what their critical pairs reach, and a
//! piece of one stratum against a piece of another that cannot hold a
//! critical pair holds nothing the least can be at.
//!
//! **Pieces.** A stratum is cut into pieces (a face into boxes of its
//! surface's parameters, an edge into intervals): [`craft3d_geom::support`]
//! gives each piece's least along any direction and a cone around its
//! normals or tangents. A face's piece is kept in full even where it
//! reaches past the face (so it holds every point of the face it covers);
//! one that meets none of the face's edges is all in or all out of the
//! face (it is connected), told by its middle, and dropped when out.
//!
//! **Bounds on a pair of pieces**, the most of: the distance of their
//! boxes; along a direction `d`, the least of `d · q` over the one less
//! the most of `d · p` over the other (`|q − p| ≥ d · (q − p)`), for `d`
//! from one box's middle to the other's, along each cone's axis and along
//! the axis of a kind turned about one; the distance in the plane through
//! an axis of what the two pieces sweep turning about it (for any points,
//! `|p − q| ≥ |(ρ_p, z_p) − (ρ_q, z_q)|`: a cylinder, cone, torus, circle,
//! sphere or plane about that axis is a segment, circle or point there,
//! so coaxial kinds are exact); and the distance in the cross-section
//! square to a cylinder's axis (`|p − q| ≥` that of their projections: a
//! cylinder along it is a circle, a plane along it a segment). A pair is
//! dropped when it cannot hold a critical pair: with `w = q − p` boxed in
//! a frame about a cone's axis, a normal cone of half-angle `β` needs
//! `|w⊥| ≤ tan β |w∥|` somewhere, a tangent cone `|w∥| ≤ tan γ |w⊥|`.
//!
//! **The search** takes the pair with the least bound first, cuts the
//! piece that spreads most, until no pair's bound is below the least
//! found less what is close enough, or the work allowed is spent: then
//! the proof is the least bound left, honestly short. A pair of points on
//! the faces met on the way that is nearer than the least found is taken
//! as the new least found.

use crate::verify::{Part, face_edges};
use craft3d_geom::support::{
    Cone, ConeKind, CurveNets, Piece, SurfaceNets, curve_piece, surface_piece,
};
use craft3d_geom::{Curve3, Surface};
use craft3d_kernel::boolean::probe::OnFace;
use craft3d_math::{Bbox3, Vec3};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BinaryHeap};
use std::f64::consts::{FRAC_PI_2, TAU};

/// Most pairs of pieces one distance looks at.
pub(crate) const MAX_NODES: usize = 100_000;
/// Most pairs of strata of one pair of faces.
const MAX_STRATA_PAIRS: usize = 1_000_000;
/// Most pairs of strata of one distance, all pairs of faces together
/// (beyond, the pairs of faces fall back on the mesh).
const MAX_TOTAL_STRATA_PAIRS: usize = 1_000_000;

#[cfg(test)]
thread_local! {
    /// In tests, caps lower than [`MAX_NODES`] and
    /// [`MAX_TOTAL_STRATA_PAIRS`] (in that order), to see what a proof
    /// past them comes to; per thread, so tests running beside it keep
    /// the real ones.
    pub(crate) static CAPS: std::cell::Cell<Option<(usize, usize)>> =
        const { std::cell::Cell::new(None) };
    /// In tests, what the last proof on this thread spent: pairs of
    /// pieces looked at and pairs of strata made.
    pub(crate) static SPENT: std::cell::Cell<(usize, usize)> =
        const { std::cell::Cell::new((0, 0)) };
}

/// [`MAX_NODES`], or a test's lower cap.
fn max_nodes() -> usize {
    #[cfg(test)]
    if let Some((nodes, _)) = CAPS.with(std::cell::Cell::get) {
        return nodes;
    }
    MAX_NODES
}

/// [`MAX_TOTAL_STRATA_PAIRS`], or a test's lower cap.
fn max_total_strata_pairs() -> usize {
    #[cfg(test)]
    if let Some((_, pairs)) = CAPS.with(std::cell::Cell::get) {
        return pairs;
    }
    MAX_TOTAL_STRATA_PAIRS
}

/// How deep an edge is halved to tell whether it meets a piece's box.
const EDGE_DEPTH: u32 = 14;
/// What counts as one axis, or a direction along a plane, in the
/// symmetric shortcuts: the sine of the angle and the offset, relative.
const SAME_AXIS: f64 = 1e-12;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Face,
    Edge,
    Vertex,
}

/// A stratum: (side, part, kind, index in the body).
type Key = (u8, usize, Kind, usize);
/// A pair of faces: (part, face) of each side.
pub(crate) type FacePair = ((usize, usize), (usize, usize));
/// An edge's pieces: their ranges and boxes.
type Leaves = Option<Vec<((f64, f64), Bbox3)>>;

/// What a stratum's kind is turned about or drawn along.
#[derive(Clone, Copy, Debug)]
enum Sym {
    /// A cylinder (`slope` 0), a cone, a torus (`minor` > 0) or a circle
    /// about the axis through `o` along `z`.
    Axis {
        o: Vec3,
        z: Vec3,
        shape: Shape,
    },
    Sphere {
        c: Vec3,
        r: f64,
    },
    Plane {
        o: Vec3,
        n: Vec3,
    },
    Line {
        o: Vec3,
        dir: Vec3,
    },
    None,
}

#[derive(Clone, Copy, Debug)]
enum Shape {
    Cylinder { r: f64 },
    Cone { r: f64, slope: f64 },
    Torus { major: f64, minor: f64 },
    Circle { r: f64 },
}

/// A boundary edge of a face, for telling whether a piece meets it.
struct Bound<'a> {
    curve: &'a Curve3,
    nets: CurveNets,
    range: (f64, f64),
    bbox: Bbox3,
    /// The boxes of its range cut in [`LEAVES`] equal pieces, made when
    /// first asked (`None` inside: a piece could not be enclosed).
    leaves: std::cell::OnceCell<Leaves>,
}

/// How many pieces an edge's boxes are kept for at once.
const LEAVES: u32 = 32;

/// A stratum ready to be cut.
struct Prim<'a> {
    part: usize,
    side: u8,
    kind: Kind,
    /// The face (for a face stratum) or edge (edge stratum) or vertex.
    index: usize,
    surface: Option<&'a Surface>,
    curve: Option<&'a Curve3>,
    snets: SurfaceNets,
    cnets: CurveNets,
    /// How far the true boundary can be from an edge's curve or a vertex.
    slack: f64,
    /// The body's largest entity tolerance: how far a face's outline (by
    /// which `on_face` tells in from out) can be from its edges' curves.
    tolerant: f64,
    boundary: Vec<Bound<'a>>,
    sym: Sym,
    root: Param,
    /// A face's outward normal is its surface's times this (-1 for a
    /// reversed face); 1 for an edge or a vertex.
    sign: f64,
}

#[derive(Clone, Copy, Debug)]
enum Param {
    Face((f64, f64), (f64, f64)),
    Edge((f64, f64)),
    Vertex(Vec3),
}

/// A piece of a stratum.
struct Rec {
    prim: usize,
    param: Param,
    piece: Piece,
    bbox: Bbox3,
    center: Vec3,
    /// A face's piece known to lie within the face (no reach past it).
    inside: bool,
    cone: Option<Cone>,
    /// A face's piece: the cone around its outward normals, oriented
    /// (every outward normal within the half-angle of the axis itself).
    out: Option<Cone>,
    /// The pieces it was cut into, per direction cut (u or t, v).
    children: [Option<Vec<usize>>; 2],
}

/// What making a piece came to.
enum Made {
    Rec(usize),
    /// Out of its face: nothing of the face is lost by leaving it out.
    Out,
    /// It could not be enclosed (or its middle is not finite).
    Failed,
}

struct Node {
    bound: f64,
    seq: u64,
    a: usize,
    b: usize,
    pair: usize,
}

impl PartialEq for Node {
    fn eq(&self, o: &Self) -> bool {
        self.cmp(o) == Ordering::Equal
    }
}
impl Eq for Node {}
impl PartialOrd for Node {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Node {
    // A min-heap on the bound, the order of making as the tie-break.
    fn cmp(&self, o: &Self) -> Ordering {
        o.bound
            .total_cmp(&self.bound)
            .then_with(|| o.seq.cmp(&self.seq))
    }
}

/// A nearer pair of points found on the way.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Nearer {
    pub distance: f64,
    pub a: Vec3,
    pub b: Vec3,
    /// The pair of faces (part, face) it was found for.
    pub face_a: (usize, usize),
    pub face_b: (usize, usize),
}

/// What the proof came to.
pub(crate) struct Proof {
    /// Per pair of faces asked about, in order: the proven least
    /// distance, or `None` where the faces could not be cut.
    pub lower: Vec<Option<f64>>,
    pub nearer: Option<Nearer>,
}

fn grown(b: &Bbox3, by: f64) -> Bbox3 {
    match b.corners() {
        Some((lo, hi)) => {
            let d = Vec3::new(by, by, by);
            Bbox3::from_points([lo - d, hi + d])
        }
        None => *b,
    }
}

fn boxes_distance(a: &Bbox3, b: &Bbox3) -> f64 {
    let (Some((alo, ahi)), Some((blo, bhi))) = (a.corners(), b.corners()) else {
        return f64::INFINITY;
    };
    let d = |al: f64, ah: f64, bl: f64, bh: f64| (bl - ah).max(0.0).max(al - bh);
    Vec3::new(
        d(alo.x, ahi.x, blo.x, bhi.x),
        d(alo.y, ahi.y, blo.y, bhi.y),
        d(alo.z, ahi.z, blo.z, bhi.z),
    )
    .length()
}

fn center(b: &Bbox3) -> Vec3 {
    b.center().unwrap_or(Vec3::ZERO)
}

/// Two unit vectors square to `z` and to each other.
fn square_to(z: Vec3) -> Option<(Vec3, Vec3)> {
    let hint = if z.x.abs() <= z.y.abs() && z.x.abs() <= z.z.abs() {
        Vec3::X
    } else if z.y.abs() <= z.z.abs() {
        Vec3::Y
    } else {
        Vec3::Z
    };
    let e1 = z.cross(hint).normalized()?;
    let e2 = z.cross(e1).normalized()?;
    Some((e1, e2))
}

// ------------------------------------------------------- 2D distances

/// What a piece is in a plane through an axis or across one: a point, a
/// segment, a circle (the curve) or a filled box `(x0, x1, y0, y1)`.
#[derive(Clone, Copy, Debug)]
enum Img {
    Seg((f64, f64), (f64, f64)),
    Circle((f64, f64), f64),
    Rect(f64, f64, f64, f64),
}

fn d2(a: (f64, f64), b: (f64, f64)) -> f64 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

fn pt_seg(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    d2(p, (a.0 + t * dx, a.1 + t * dy))
}

fn cross2(o: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0)
}

fn seg_seg(a: (f64, f64), b: (f64, f64), c: (f64, f64), d: (f64, f64)) -> f64 {
    let (d1, d2_, d3, d4) = (
        cross2(c, d, a),
        cross2(c, d, b),
        cross2(a, b, c),
        cross2(a, b, d),
    );
    if ((d1 > 0.0 && d2_ < 0.0) || (d1 < 0.0 && d2_ > 0.0))
        && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0))
    {
        return 0.0;
    }
    pt_seg(a, c, d)
        .min(pt_seg(b, c, d))
        .min(pt_seg(c, a, b))
        .min(pt_seg(d, a, b))
}

fn pt_rect(p: (f64, f64), (x0, x1, y0, y1): (f64, f64, f64, f64)) -> f64 {
    let dx = (x0 - p.0).max(0.0).max(p.0 - x1);
    let dy = (y0 - p.1).max(0.0).max(p.1 - y1);
    dx.hypot(dy)
}

fn corners(r: (f64, f64, f64, f64)) -> [(f64, f64); 4] {
    let (x0, x1, y0, y1) = r;
    [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
}

fn seg_rect(a: (f64, f64), b: (f64, f64), r: (f64, f64, f64, f64)) -> f64 {
    if pt_rect(a, r) == 0.0 || pt_rect(b, r) == 0.0 {
        return 0.0;
    }
    let c = corners(r);
    let mut least = f64::INFINITY;
    for k in 0..4 {
        let (Some(&p), Some(&q)) = (c.get(k), c.get((k + 1) % 4)) else {
            continue;
        };
        least = least.min(seg_seg(a, b, p, q));
    }
    least
}

/// The distance from a circle (the curve) to a connected set whose
/// distances from the centre run over `[near, far]`.
fn circle_to(r: f64, near: f64, far: f64) -> f64 {
    if near >= r {
        near - r
    } else if far <= r {
        r - far
    } else {
        0.0
    }
}

fn img_distance(a: &Img, b: &Img) -> f64 {
    match (a, b) {
        (Img::Seg(p, q), Img::Seg(s, t)) => seg_seg(*p, *q, *s, *t),
        (Img::Seg(p, q), Img::Rect(x0, x1, y0, y1))
        | (Img::Rect(x0, x1, y0, y1), Img::Seg(p, q)) => seg_rect(*p, *q, (*x0, *x1, *y0, *y1)),
        (Img::Rect(a0, a1, b0, b1), Img::Rect(c0, c1, e0, e1)) => {
            let dx = (c0 - a1).max(0.0).max(a0 - c1);
            let dy = (e0 - b1).max(0.0).max(b0 - e1);
            dx.hypot(dy)
        }
        (Img::Circle(c, r), Img::Seg(p, q)) | (Img::Seg(p, q), Img::Circle(c, r)) => {
            circle_to(*r, pt_seg(*c, *p, *q), d2(*c, *p).max(d2(*c, *q)))
        }
        (Img::Circle(c, r), Img::Rect(x0, x1, y0, y1))
        | (Img::Rect(x0, x1, y0, y1), Img::Circle(c, r)) => {
            let rect = (*x0, *x1, *y0, *y1);
            let far = corners(rect).iter().map(|k| d2(*c, *k)).fold(0.0, f64::max);
            circle_to(*r, pt_rect(*c, rect), far)
        }
        (Img::Circle(c1, r1), Img::Circle(c2, r2)) => {
            let d = d2(*c1, *c2);
            if d >= r1 + r2 {
                d - r1 - r2
            } else if d <= (r1 - r2).abs() {
                (r1 - r2).abs() - d
            } else {
                0.0
            }
        }
    }
}

// ------------------------------------------------------------ strata

/// The parameters of a face's surface that hold the whole face.
fn face_root(part: &Part<'_>, face: usize, surface: &Surface, slack: f64) -> Option<Param> {
    let body = part.body();
    let edges: Vec<Piece> = face_edges(body, face)
        .filter_map(|e| curve_piece(&e.curve, &CurveNets::of(&e.curve), e.range))
        .collect();
    let range = |d: Vec3, o: Vec3| -> Option<(f64, f64)> {
        if edges.is_empty() {
            return None;
        }
        let lo = edges
            .iter()
            .map(|p| p.least(d))
            .fold(f64::INFINITY, f64::min);
        let hi = edges
            .iter()
            .map(|p| p.most(d))
            .fold(f64::NEG_INFINITY, f64::max);
        let at = d.dot(o);
        let grow = slack + 1e-9 * (hi - lo).abs().max(at.abs()).max(1.0);
        (lo.is_finite() && hi.is_finite()).then_some((lo - at - grow, hi - at + grow))
    };
    let turn = (0.0, TAU);
    // A linear function over a plane, and the height along the axis over
    // a cylinder or a cone, is least and most on the face's boundary.
    Some(match surface {
        Surface::Plane(p) => {
            let f = p.frame();
            Param::Face(range(f.x(), f.origin())?, range(f.y(), f.origin())?)
        }
        Surface::Cylinder(c) => Param::Face(turn, range(c.frame().z(), c.frame().origin())?),
        Surface::Cone(c) => Param::Face(turn, range(c.frame().z(), c.frame().origin())?),
        Surface::Sphere(_) => Param::Face(turn, (-FRAC_PI_2, FRAC_PI_2)),
        Surface::Torus(_) => Param::Face(turn, turn),
        Surface::Bezier(_) | Surface::Nurbs(_) => {
            let (u, v) = surface.domain();
            Param::Face(u, v)
        }
    })
}

fn surface_sym(s: &Surface) -> Sym {
    match s {
        Surface::Plane(p) => Sym::Plane {
            o: p.frame().origin(),
            n: p.frame().z(),
        },
        Surface::Cylinder(c) => Sym::Axis {
            o: c.frame().origin(),
            z: c.frame().z(),
            shape: Shape::Cylinder { r: c.radius() },
        },
        Surface::Cone(c) => Sym::Axis {
            o: c.frame().origin(),
            z: c.frame().z(),
            shape: Shape::Cone {
                r: c.radius(),
                slope: c.half_angle().tan(),
            },
        },
        Surface::Sphere(s) => Sym::Sphere {
            c: s.frame().origin(),
            r: s.radius(),
        },
        Surface::Torus(t) => {
            let (major, minor) = t.radii();
            Sym::Axis {
                o: t.frame().origin(),
                z: t.frame().z(),
                shape: Shape::Torus { major, minor },
            }
        }
        Surface::Bezier(_) | Surface::Nurbs(_) => Sym::None,
    }
}

fn curve_sym(c: &Curve3) -> Sym {
    match c {
        Curve3::Line(l) => Sym::Line {
            o: l.origin(),
            dir: l.direction(),
        },
        Curve3::Circle(c) => Sym::Axis {
            o: c.frame().origin(),
            z: c.frame().z(),
            shape: Shape::Circle { r: c.radius() },
        },
        Curve3::Ellipse(_) | Curve3::Nurbs(_) => Sym::None,
    }
}

/// The search over one question.
struct Search<'p, 'a> {
    parts: [&'p [Part<'a>]; 2],
    prims: Vec<Prim<'a>>,
    keys: BTreeMap<Key, Option<usize>>,
    recs: Vec<Rec>,
    tol: f64,
    /// Strata a piece of which could not be enclosed.
    failed: std::collections::BTreeSet<usize>,
    /// Bounding lines into the material (a thickness) rather than
    /// distances: side 0's pieces are where a line leaves, side 1's where
    /// it comes out ([`Search::ray_bound`]).
    rays: bool,
    /// [`Search::floor`] per face and axis.
    floors: std::cell::RefCell<BTreeMap<(usize, [u64; 6]), f64>>,
}

impl<'p, 'a> Search<'p, 'a>
where
    'a: 'p,
{
    fn part(&self, side: u8, part: usize) -> Option<&'p Part<'a>> {
        self.parts.get(usize::from(side)).and_then(|p| p.get(part))
    }

    /// The stratum `key`, made ready once (with its first piece).
    fn prim(&mut self, key: Key) -> Option<usize> {
        if let Some(found) = self.keys.get(&key) {
            return *found;
        }
        let made = self.make_prim(key);
        let id = made.map(|prim| {
            self.prims.push(prim);
            self.prims.len() - 1
        });
        let id = id.and_then(|p| match self.make_rec(p, None) {
            Made::Rec(_) => Some(p),
            Made::Out | Made::Failed => None,
        });
        self.keys.insert(key, id);
        id
    }

    fn make_prim(&self, (side, part_index, kind, index): Key) -> Option<Prim<'a>> {
        let part = self.part(side, part_index)?;
        let body: &'a craft3d_topo::Body = part.body();
        let slack = body.tolerances.largest().unwrap_or(0.0);
        match kind {
            Kind::Face => {
                let face = body.faces.get(index)?;
                let surface = &face.surface;
                let snets = SurfaceNets::of(surface).ok()?;
                let mut boundary = Vec::new();
                let mut seen = std::collections::BTreeSet::new();
                for coedge in face
                    .loops
                    .iter()
                    .filter_map(|l| body.loops.get(l.0 as usize))
                    .flat_map(|l| l.coedges.iter())
                {
                    if !seen.insert(coedge.edge.0) {
                        continue;
                    }
                    let edge = body.edges.get(coedge.edge.0 as usize)?;
                    let nets = CurveNets::of(&edge.curve);
                    let piece = curve_piece(&edge.curve, &nets, edge.range)?;
                    boundary.push(Bound {
                        curve: &edge.curve,
                        nets,
                        range: edge.range,
                        bbox: piece.bbox(),
                        leaves: std::cell::OnceCell::new(),
                    });
                }
                let root = face_root(part, index, surface, slack)?;
                Some(Prim {
                    part: part_index,
                    side,
                    kind,
                    index,
                    surface: Some(surface),
                    curve: None,
                    snets,
                    cnets: CurveNets::default(),
                    slack: 0.0,
                    tolerant: slack,
                    boundary,
                    sym: surface_sym(surface),
                    root,
                    sign: if face.reversed { -1.0 } else { 1.0 },
                })
            }
            Kind::Edge => {
                let edge = body.edges.get(index)?;
                let (t0, t1) = edge.range;
                if !(t0.is_finite() && t1.is_finite() && t0 < t1) {
                    return None;
                }
                Some(Prim {
                    part: part_index,
                    side,
                    kind,
                    index,
                    surface: None,
                    curve: Some(&edge.curve),
                    snets: SurfaceNets::default(),
                    cnets: CurveNets::of(&edge.curve),
                    slack,
                    tolerant: slack,
                    boundary: Vec::new(),
                    sym: curve_sym(&edge.curve),
                    root: Param::Edge(edge.range),
                    sign: 1.0,
                })
            }
            Kind::Vertex => {
                let v = body.vertices.get(index)?;
                v.point.is_finite().then_some(Prim {
                    part: part_index,
                    side,
                    kind,
                    index,
                    surface: None,
                    curve: None,
                    snets: SurfaceNets::default(),
                    cnets: CurveNets::default(),
                    slack,
                    tolerant: slack,
                    boundary: Vec::new(),
                    sym: Sym::None,
                    root: Param::Vertex(v.point),
                    sign: 1.0,
                })
            }
        }
    }

    /// A piece of `prim` over `param` (its root when `None`), or that it
    /// lies out of its face, or that it cannot be enclosed.
    fn make_rec(&mut self, prim_id: usize, param: Option<(Param, bool)>) -> Made {
        match self.try_rec(prim_id, param) {
            Some(made) => made,
            None => Made::Failed,
        }
    }

    fn try_rec(&mut self, prim_id: usize, param: Option<(Param, bool)>) -> Option<Made> {
        let prim = self.prims.get(prim_id)?;
        let (param, inside) = param.unwrap_or((prim.root, false));
        let mut normal = None;
        let (piece, center) = match param {
            Param::Face(u, v) => {
                let s = prim.surface?;
                let piece = surface_piece(s, &prim.snets, u, v)?;
                let (um, vm) = (0.5 * (u.0 + u.1), 0.5 * (v.0 + v.1));
                normal = s.normal(um, vm).map(|n| n * prim.sign);
                (piece, s.eval(um, vm))
            }
            Param::Edge(t) => {
                let c = prim.curve?;
                let piece = curve_piece(c, &prim.cnets, t)?;
                (piece, c.eval(0.5 * (t.0 + t.1)))
            }
            Param::Vertex(p) => (Piece::Point(p), p),
        };
        if !center.is_finite() {
            return None;
        }
        let bbox = piece.bbox();
        let cone = piece.cone();
        let out = oriented(cone, normal);
        let mut inside = inside || prim.kind != Kind::Face;
        if !inside {
            match self.membership(prim_id, &bbox, center) {
                Some(true) => inside = true,
                Some(false) => return Some(Made::Out),
                None => {}
            }
        }
        self.recs.push(Rec {
            prim: prim_id,
            param,
            piece,
            bbox,
            center,
            inside,
            cone,
            out,
            children: [None, None],
        });
        Some(Made::Rec(self.recs.len() - 1))
    }

    /// Whether a face's piece (its box, its middle on the surface) is in
    /// the face (`Some(true)`), out of it, or may reach across its
    /// boundary (`None`).
    fn membership(&self, prim_id: usize, bbox: &Bbox3, middle: Vec3) -> Option<bool> {
        let prim = self.prims.get(prim_id)?;
        let part = self.part(prim.side, prim.part)?;
        let reach = grown(bbox, 2.0 * self.tol + prim.tolerant);
        for edge in &prim.boundary {
            if edge_near(edge, &reach) {
                return None;
            }
        }
        match part.probe.on_face(prim.index, middle) {
            Ok(OnFace::In) => Some(true),
            Ok(OnFace::Out) => Some(false),
            _ => None,
        }
    }

    /// The pieces `rec` is cut into along `dir` (made once).
    fn children(&mut self, rec_id: usize, dir: usize) -> Vec<usize> {
        let Some(rec) = self.recs.get(rec_id) else {
            return Vec::new();
        };
        if let Some(Some(done)) = rec.children.get(dir) {
            return done.clone();
        }
        let (prim, inside) = (rec.prim, rec.inside);
        let halves: Vec<Param> = match rec.param {
            Param::Face(u, v) => {
                if dir == 0 {
                    let m = 0.5 * (u.0 + u.1);
                    vec![Param::Face((u.0, m), v), Param::Face((m, u.1), v)]
                } else {
                    let m = 0.5 * (v.0 + v.1);
                    vec![Param::Face(u, (v.0, m)), Param::Face(u, (m, v.1))]
                }
            }
            Param::Edge(t) => {
                let m = 0.5 * (t.0 + t.1);
                vec![Param::Edge((t.0, m)), Param::Edge((m, t.1))]
            }
            Param::Vertex(_) => Vec::new(),
        };
        let mut made = Vec::new();
        for half in halves {
            match self.make_rec(prim, Some((half, inside))) {
                Made::Rec(id) => made.push(id),
                Made::Out => {}
                // A half that cannot be enclosed: what it holds is not
                // bounded, so the pairs of faces of this stratum are not
                // proven.
                Made::Failed => {
                    self.failed.insert(prim);
                }
            }
        }
        if let Some(slot) = self
            .recs
            .get_mut(rec_id)
            .and_then(|r| r.children.get_mut(dir))
        {
            *slot = Some(made.clone());
        }
        made
    }

    /// How much cutting `rec` along each direction would gain against
    /// direction `d`; `None` for a direction that cannot be cut further.
    fn spread(&self, rec: &Rec, d: Option<Vec3>, meet: bool) -> [Option<f64>; 2] {
        let narrow = |r: (f64, f64)| r.1 - r.0 <= 1e-12 * (1.0 + r.0.abs().max(r.1.abs()));
        // How much `d · x` spreads along the direction (its change end to
        // end and the bow at the middle); a millionth of the length breaks
        // ties where it does not spread at all (a flat piece square to d).
        // Where the boxes meet, a twentieth of the length as well; without
        // a direction, the length.
        let score = |p0: Vec3, p1: Vec3, pm: Vec3| {
            let len = p1.distance(p0);
            let bend = pm.distance((p0 + p1) * 0.5);
            let weight = if meet { 0.05 } else { 1e-6 };
            match d {
                Some(d) => d.dot(p1 - p0).abs() + 4.0 * bend + weight * len,
                None => len + 4.0 * bend,
            }
        };
        let Some(prim) = self.prims.get(rec.prim) else {
            return [None, None];
        };
        match rec.param {
            Param::Face(u, v) => {
                let Some(s) = prim.surface else {
                    return [None, None];
                };
                let (um, vm) = (0.5 * (u.0 + u.1), 0.5 * (v.0 + v.1));
                let su = (!narrow(u))
                    .then(|| score(s.eval(u.0, vm), s.eval(u.1, vm), rec.center))
                    .filter(|x| x.is_finite());
                let sv = (!narrow(v))
                    .then(|| score(s.eval(um, v.0), s.eval(um, v.1), rec.center))
                    .filter(|x| x.is_finite());
                [su, sv]
            }
            Param::Edge(t) => {
                let Some(c) = prim.curve else {
                    return [None, None];
                };
                let st = (!narrow(t))
                    .then(|| score(c.eval(t.0), c.eval(t.1), rec.center))
                    .filter(|x| x.is_finite());
                [st, None]
            }
            Param::Vertex(_) => [None, None],
        }
    }

    /// The proven bound on a pair of pieces; `None` when the pair cannot
    /// hold a critical pair (nothing the least can be at).
    fn bound(&self, a: &Rec, b: &Rec, enough: f64) -> Option<f64> {
        if self.rays {
            return self.ray_bound(a, b, enough);
        }
        let (pa, pb) = (self.prims.get(a.prim)?, self.prims.get(b.prim)?);
        let slack = pa.slack + pb.slack;
        let lb = self.lower(pa, a, pb, b, enough + slack) - slack;
        if lb < enough && excluded(a, b) {
            return None;
        }
        Some(lb.max(0.0))
    }

    /// A proven bound below the distance of any point of piece `a` from
    /// any of `b` (the strata's slack not taken off); it stops at the
    /// boxes' distance when that is `enough`.
    fn lower(&self, pa: &Prim<'_>, a: &Rec, pb: &Prim<'_>, b: &Rec, enough: f64) -> f64 {
        let mut lb = boxes_distance(&a.bbox, &b.bbox);
        if lb >= enough {
            return lb;
        }
        let along = |d: Vec3| b.piece.least(d) - a.piece.most(d);
        let c = center(&b.bbox) - center(&a.bbox);
        let mut dirs: Vec<Vec3> = Vec::new();
        dirs.extend(c.normalized());
        for rec in [a, b] {
            if let Some(cone) = rec.cone {
                match rec.piece.cone_kind() {
                    Some(ConeKind::Normal) => dirs.push(cone.axis),
                    Some(ConeKind::Tangent) => {
                        dirs.extend((c - cone.axis * c.dot(cone.axis)).normalized());
                    }
                    None => {}
                }
            }
        }
        for prim in [pa, pb] {
            match prim.sym {
                Sym::Axis { z, .. } => dirs.push(z),
                Sym::Plane { n, .. } => dirs.push(n),
                _ => {}
            }
        }
        for d in dirs {
            let d = if d.dot(c) < 0.0 { -d } else { d };
            lb = lb.max(along(d));
        }
        // About an axis of either, and across a cylinder's.
        for prim in [pa, pb] {
            if let Sym::Axis { o, z, shape } = prim.sym {
                lb = lb.max(self.meridian(pa, a, pb, b, o, z));
                if matches!(shape, Shape::Cylinder { .. }) {
                    lb = lb.max(self.section(pa, a, pb, b, o, z));
                }
            }
            // About a sphere's centre `c`: `|p − q| ≥ ||p − c| − |q − c||`,
            // and a sphere about `c` is at its radius exactly.
            if let Sym::Sphere { c, .. } = prim.sym {
                let (sa, ma) = radii(pa, a, c);
                let (sb, mb) = radii(pb, b, c);
                lb = lb.max((sb.0 - sa.1).max(sa.0 - sb.1) - ma - mb);
            }
        }
        lb
    }

    /// The proven bound below the length of a line that leaves piece `a`
    /// (of a face of side 0) into the material along the inward normal
    /// and comes out of the body through piece `b` (of a face of side
    /// 1); `None` when no such line can come out there. Every line is
    /// `w = q − p = −t n_a(p)`, `t > 0`, and comes out where it does not
    /// point into `b`: `n_b(q) · w ≥ 0`.
    fn ray_bound(&self, a: &Rec, b: &Rec, enough: f64) -> Option<f64> {
        let (pa, pb) = (self.prims.get(a.prim)?, self.prims.get(b.prim)?);
        let mut lb = self.lower(pa, a, pb, b, enough);
        let range = |e: Vec3| {
            (
                b.piece.least(e) - a.piece.most(e),
                b.piece.most(e) - a.piece.least(e),
            )
        };
        // Coming out: some n_a and n_b more than a right angle apart.
        if let (Some(ca), Some(cb)) = (a.out, b.out) {
            let apart = ca.axis.dot(cb.axis).clamp(-1.0, 1.0).acos();
            if apart + ca.half_angle + cb.half_angle < FRAC_PI_2 - ANGLE_MARGIN {
                return None;
            }
        }
        // Along the inward normals: `w` within the cone about `e = −c_a`,
        // so `w · e ≥ |w⊥| / tan β` and `|w| ≥ |(w · e, |w⊥|)|`.
        if let Some(ca) = a.out
            && let Some((e1, e2)) = square_to(-ca.axis)
        {
            let (lo, hi) = range(-ca.axis);
            if hi < 0.0 {
                return None;
            }
            let across = least_abs(range(e1)).hypot(least_abs(range(e2)));
            let tan = ca.half_angle.tan();
            if across > tan * hi {
                return None;
            }
            let mut along = lo.max(0.0);
            if tan > 0.0 {
                along = along.max(across / tan);
            }
            lb = lb.max(along.hypot(across));
        }
        // A surface extruded along `z` (a cylinder, a spline extrusion)
        // has its normals square to `z`: so has `w`.
        if pa.kind == Kind::Face
            && let Some(z) = self
                .part(pa.side, pa.part)
                .and_then(|part| crate::verify::extruded(part.body(), pa.index))
        {
            let (lo, hi) = range(z);
            if lo > 0.0 || hi < 0.0 {
                return None;
            }
        }
        match pa.sym {
            // A convex cylinder's lines pass through its axis.
            Sym::Axis {
                o,
                z,
                shape: Shape::Cylinder { r },
            } if pa.kind == Kind::Face => {
                let radial = |p: Vec3| (p - o) - z * (p - o).dot(z);
                lb = lb.max(focal(a, b, r, o, |p| from_axis(p, o, z), radial));
            }
            Sym::Sphere { c, r } => {
                lb = lb.max(focal(a, b, r, c, |p| p.distance(c), |p| p - c));
            }
            _ => {}
        }
        Some(lb.max(0.0))
    }

    /// Whether two face strata lie in one plane (normals parallel to
    /// 1e-12, apart along them by no more than the tolerance and a quarter
    /// of `enough`) and do not overlap: then their interiors hold no
    /// critical pair. Points of two faces of one plane are apart within it,
    /// never along its normal, so only coincident points (an overlap)
    /// could be critical. The faces overlap only if one boundary meets the
    /// other face (then a pair of their strata is at 0, which bounds the
    /// pair of faces anyway) or one face lies inside the other: told by a
    /// vertex of each, put on the other's plane, out of the other.
    fn one_plane_apart(&self, pa: usize, pb: usize, enough: f64) -> bool {
        let (Some(a), Some(b)) = (self.prims.get(pa), self.prims.get(pb)) else {
            return false;
        };
        if a.kind != Kind::Face || b.kind != Kind::Face {
            return false;
        }
        let (Some(Surface::Plane(x)), Some(Surface::Plane(y))) = (a.surface, b.surface) else {
            return false;
        };
        let (na, nb) = (x.frame().z(), y.frame().z());
        let offset = (y.frame().origin() - x.frame().origin()).dot(na).abs();
        if na.cross(nb).length() > SAME_AXIS
            || offset.is_nan()
            || offset > self.tol.min(0.25 * enough)
        {
            return false;
        }
        let out_of = |inner: &Prim<'_>, outer: &Prim<'_>, n: Vec3, o: Vec3| -> bool {
            let (Some(pi), Some(po)) = (
                self.part(inner.side, inner.part),
                self.part(outer.side, outer.part),
            ) else {
                return false;
            };
            let Some(v) = pi.body().faces.get(inner.index).and_then(|f| {
                f.loops
                    .iter()
                    .filter_map(|l| pi.body().loops.get(l.0 as usize))
                    .flat_map(|l| l.coedges.iter())
                    .filter_map(|c| pi.body().edges.get(c.edge.0 as usize))
                    .map(|e| e.curve.eval(e.range.0))
                    .next()
            }) else {
                return false;
            };
            let on = v - n * (v - o).dot(n);
            matches!(po.probe.on_face(outer.index, on), Ok(OnFace::Out))
        };
        out_of(a, b, nb, y.frame().origin()) && out_of(b, a, na, x.frame().origin())
    }

    /// Whether the pair of strata `f` (a flat face) and `x` (an edge, a
    /// vertex or another face; `rx` its first piece) holds no
    /// critical pair the least can be at, by one of two rules:
    ///
    /// - **In its plane, out of it.** `x` lies in `f`'s plane (an edge or
    ///   a vertex within the tolerance and a quarter of `enough` of it,
    ///   by its piece's extent along the normal) and a point of it put on
    ///   the plane is out of `f`. Points of one plane are apart within
    ///   it, never along its normal, so only coincident points could be
    ///   critical; `x` meets `f` only if it crosses `f`'s boundary (a
    ///   pair of their strata is then at 0, which bounds the pair of faces
    ///   anyway) or lies in `f`, which the point rules out.
    /// - **On one side, never square to it.** `x` (a face or an edge)
    ///   lies on one side of `f`'s plane (its first piece's extent along
    ///   the normal, to rounding) and its cone holds no normal along `f`'s
    ///   (a face) or no tangent square to it (an edge). A critical pair
    ///   needs the line between the points along both normals (square to
    ///   the tangent), and a point of `x`'s open interior on the plane
    ///   would be a touch from one side, where its normal is along `f`'s
    ///   (its tangent square to it): neither can be.
    fn plane_rules(&self, f: usize, x: usize, rx: usize, enough: f64) -> bool {
        let (Some(pf), Some(px), Some(rec_x)) =
            (self.prims.get(f), self.prims.get(x), self.recs.get(rx))
        else {
            return false;
        };
        let (Kind::Face, Some(Surface::Plane(plane))) = (pf.kind, pf.surface) else {
            return false;
        };
        let (n, o) = (plane.frame().z(), plane.frame().origin());
        let h = n.dot(o);
        let (lo, hi) = (rec_x.piece.least(n) - h, rec_x.piece.most(n) - h);
        if !(lo.is_finite() && hi.is_finite()) {
            return false;
        }
        let near = self.tol.min(0.25 * enough);
        if px.kind != Kind::Face && lo >= -near && hi <= near {
            let point = match (px.curve, px.root) {
                (Some(c), Param::Edge(t)) => c.eval(t.0),
                (_, Param::Vertex(p)) => p,
                _ => return false,
            };
            let on = point - n * (point - o).dot(n);
            let Some(part) = self.part(pf.side, pf.part) else {
                return false;
            };
            if on.is_finite() && matches!(part.probe.on_face(pf.index, on), Ok(OnFace::Out)) {
                return true;
            }
        }
        let rounding =
            SAME_AXIS * (h.abs() + rec_x.bbox.diagonal() + center(&rec_x.bbox).length() + 1.0);
        let one_side = lo >= -rounding || hi <= rounding;
        let (Some(cone), Some(kind)) = (rec_x.cone, rec_x.piece.cone_kind()) else {
            return false;
        };
        if !one_side {
            return false;
        }
        // The angle between the lines of `n` and of the cone's axis.
        let between = cone.axis.dot(n).abs().clamp(0.0, 1.0).acos();
        match kind {
            ConeKind::Normal => between > cone.half_angle + ANGLE_MARGIN,
            ConeKind::Tangent => between + cone.half_angle < FRAC_PI_2 - ANGLE_MARGIN,
        }
    }

    /// The distance in the plane through the axis `(o, z)` of what the
    /// two pieces sweep turning about it.
    fn meridian(&self, pa: &Prim<'_>, a: &Rec, pb: &Prim<'_>, b: &Rec, o: Vec3, z: Vec3) -> f64 {
        let (ia, ma) = meridian_image(pa, a, o, z, self.floor(a.prim, o, z));
        let (ib, mb) = meridian_image(pb, b, o, z, self.floor(b.prim, o, z));
        img_distance(&ia, &ib) - ma - mb
    }

    /// How near the axis `(o, z)` the points of a flat face square to it
    /// can be (0 where not known): when the axis's foot on its plane is
    /// out of the face, the distance from the foot, which is convex and
    /// least at the foot alone, is least over the face on its boundary:
    /// so no less than the least over its edges (a line's segment and a
    /// circle in closed form, the boxes of the others' pieces), less the
    /// body's tolerance. Made once per face and axis.
    fn floor(&self, prim_id: usize, o: Vec3, z: Vec3) -> f64 {
        let Some(prim) = self.prims.get(prim_id) else {
            return 0.0;
        };
        let (Kind::Face, Sym::Plane { o: o2, n }) = (prim.kind, prim.sym) else {
            return 0.0;
        };
        if n.cross(z).length() > SAME_AXIS {
            return 0.0;
        }
        let bits = |v: Vec3| [v.x.to_bits(), v.y.to_bits(), v.z.to_bits()];
        let (bo, bz) = (bits(o), bits(z));
        let key = (prim_id, [bo[0], bo[1], bo[2], bz[0], bz[1], bz[2]]);
        if let Some(known) = self.floors.borrow().get(&key) {
            return *known;
        }
        let foot = o + z * (o2 - o).dot(z);
        let out = self
            .part(prim.side, prim.part)
            .is_some_and(|part| matches!(part.probe.on_face(prim.index, foot), Ok(OnFace::Out)));
        let mut least = if out { f64::INFINITY } else { 0.0 };
        for edge in &prim.boundary {
            if least <= 0.0 {
                break;
            }
            least = least.min(edge_from(edge, foot));
        }
        let rounding = 4e-15 * (foot.length() + least.min(1e300) + 1.0);
        let floor = if least.is_finite() {
            (least - prim.tolerant - rounding).max(0.0)
        } else {
            0.0
        };
        self.floors.borrow_mut().insert(key, floor);
        floor
    }

    /// The distance of the two pieces' projections across the axis.
    fn section(&self, pa: &Prim<'_>, a: &Rec, pb: &Prim<'_>, b: &Rec, o: Vec3, z: Vec3) -> f64 {
        let Some((e1, e2)) = square_to(z) else {
            return 0.0;
        };
        let (ia, ma) = section_image(pa, a, o, z, e1, e2);
        let (ib, mb) = section_image(pb, b, o, z, e1, e2);
        img_distance(&ia, &ib) - ma - mb
    }
}

/// What the angle between two cones' axes may be off by in rounding
/// (`acos` near 1 turns an error of 1e-16 in the cosine into 1.5e-8).
const ANGLE_MARGIN: f64 = 1e-7;

/// The least `|x|` over `[lo, hi]`.
fn least_abs((lo, hi): (f64, f64)) -> f64 {
    if lo > 0.0 {
        lo
    } else if hi < 0.0 {
        -hi
    } else {
        0.0
    }
}

/// A piece's cone oriented by its outward normal at the middle: every
/// normal of a connected regular piece is within the half-angle (below a
/// right angle) of the axis or of its opposite, and turning continuously
/// it cannot pass from one to the other, so all are on the side of the
/// middle's. `None` without a cone, a normal, or where the two disagree.
fn oriented(cone: Option<Cone>, normal: Option<Vec3>) -> Option<Cone> {
    let (cone, normal) = (cone?, normal?);
    let dot = cone.axis.dot(normal.normalized()?);
    if dot.abs() < 0.5 * cone.half_angle.cos() {
        return None;
    }
    Some(Cone {
        axis: if dot < 0.0 { -cone.axis } else { cone.axis },
        half_angle: cone.half_angle,
    })
}

/// What a line along the inward normal of a convex cylinder or sphere
/// must travel to reach piece `b`: the line passes through the axis (the
/// centre) at the radius `r`, so a point of it at `t` is `|t − r|` from
/// the axis; a point of `b` at most `d` from it is reached after at least
/// `r − min(d, r)`. 0 where `a` is not convex (normals pointing away from
/// the axis) or has no oriented cone.
fn focal(
    a: &Rec,
    b: &Rec,
    r: f64,
    o: Vec3,
    from: impl Fn(Vec3) -> f64,
    radial: impl Fn(Vec3) -> Vec3,
) -> f64 {
    let Some(ca) = a.out else {
        return 0.0;
    };
    if ca.axis.dot(radial(a.center)).partial_cmp(&0.0) != Some(Ordering::Greater) {
        return 0.0;
    }
    let Some((l, h)) = b.bbox.corners() else {
        return 0.0;
    };
    let mut far = 0.0_f64;
    for x in [l.x, h.x] {
        for y in [l.y, h.y] {
            for z in [l.z, h.z] {
                far = far.max(from(Vec3::new(x, y, z)));
            }
        }
    }
    // The distance from an axis or a centre is convex: most at a corner.
    let margin = 1e-14 * (o.length() + r + far + 1.0);
    (r - far.min(r) - margin).max(0.0)
}

/// No more than the least distance from `p` of the edge's points: a
/// line's segment and a circle (its whole circle) in closed form, the
/// others by the boxes of their kept pieces (0 where those are not made).
fn edge_from(edge: &Bound<'_>, p: Vec3) -> f64 {
    match edge.curve {
        Curve3::Line(_) => {
            let (a, b) = (edge.curve.eval(edge.range.0), edge.curve.eval(edge.range.1));
            let d = b - a;
            let len2 = d.dot(d);
            let t = if len2 > 0.0 {
                ((p - a).dot(d) / len2).clamp(0.0, 1.0)
            } else {
                0.0
            };
            (p - (a + d * t)).length()
        }
        Curve3::Circle(c) => {
            let (m, r) = (c.frame().z(), c.radius());
            let w = p - c.frame().origin();
            let along = w.dot(m);
            let across = (w - m * along).length();
            along.hypot(across - r)
        }
        _ => {
            let leaves = edge.leaves.get_or_init(|| leaves_of(edge));
            leaves.as_ref().map_or(0.0, |list| {
                list.iter()
                    .map(|(_, b)| crate::verify::box_distance(b, p))
                    .fold(f64::INFINITY, f64::min)
            })
        }
    }
}

/// The boxes of an edge's range cut in [`LEAVES`] equal pieces.
fn leaves_of(edge: &Bound<'_>) -> Leaves {
    let (t0, t1) = edge.range;
    let n = f64::from(LEAVES);
    (0..LEAVES)
        .map(|k| {
            let a = t0 + (t1 - t0) * f64::from(k) / n;
            let z = if k + 1 == LEAVES {
                t1
            } else {
                t0 + (t1 - t0) * f64::from(k + 1) / n
            };
            curve_piece(edge.curve, &edge.nets, (a, z)).map(|p| ((a, z), p.bbox()))
        })
        .collect()
}

/// The least and most distance from `c` of a piece's points, and a
/// margin: a sphere's about the same centre (to rounding) its radius, else
/// those of its box.
fn radii(prim: &Prim<'_>, rec: &Rec, c: Vec3) -> ((f64, f64), f64) {
    if let Sym::Sphere { c: c2, r } = prim.sym {
        let off = c2.distance(c);
        if off <= SAME_AXIS * (r + c.length()).max(1.0) {
            return ((r, r), off + 2e-15 * (r + c.length() + c2.length()));
        }
    }
    let near = crate::verify::box_distance(&rec.bbox, c);
    ((near, reach_from(rec, c)), 0.0)
}

/// The most distance from `q` of a piece's box (its farthest corner).
fn reach_from(rec: &Rec, q: Vec3) -> f64 {
    rec.bbox.corners().map_or(f64::INFINITY, |(l, h)| {
        let d = |a: f64, b: f64, c: f64| (a - c).abs().max((b - c).abs());
        Vec3::new(d(l.x, h.x, q.x), d(l.y, h.y, q.y), d(l.z, h.z, q.z)).length()
    })
}

/// What an axis or a plane that is one with `(o, z)` only to rounding
/// (`tilt`, the sine of the angle) can move a piece's image by: a point
/// `p` of the piece is placed from its own frame's origin `o2`, so its
/// error is `tilt |p − o2|`, and its place about `(o, z)` errs by
/// `tilt |p − o|` again; a cone's radius follows its height by `slope`.
fn tilt_margin(rec: &Rec, tilt: f64, o: Vec3, o2: Vec3, slope: f64) -> f64 {
    let reach = reach_from(rec, o2) + reach_from(rec, o);
    // With the rounding of the image's coordinates, a few units in the
    // last place of what they are made from.
    2.0 * tilt * reach * (1.0 + slope.abs()) + 2e-15 * (o.length() + o2.length() + reach)
}

/// How far a point is from the axis through `o` along unit `z`.
fn from_axis(p: Vec3, o: Vec3, z: Vec3) -> f64 {
    let w = p - o;
    (w - z * w.dot(z)).length()
}

/// The sine of the angle between two unit directions, and how far `p`
/// is from the axis `(o, z)`: what tells one axis.
fn off_axis(p: Vec3, dir: Vec3, o: Vec3, z: Vec3) -> (f64, f64) {
    (dir.cross(z).length(), from_axis(p, o, z))
}

fn same_axis(tilt: f64, offset: f64, scale: f64) -> bool {
    tilt <= SAME_AXIS && offset <= SAME_AXIS * scale.max(1.0)
}

/// A piece's image in the half-plane `(ρ, ζ)` about the axis `(o, z)`,
/// and a margin for an axis that is one only to rounding.
fn meridian_image(prim: &Prim<'_>, rec: &Rec, o: Vec3, z: Vec3, floor: f64) -> (Img, f64) {
    let lo = rec.piece.least(z) - o.dot(z);
    let hi = rec.piece.most(z) - o.dot(z);
    let corners = rec
        .bbox
        .corners()
        .map(|(l, h)| {
            [
                Vec3::new(l.x, l.y, l.z),
                Vec3::new(h.x, l.y, l.z),
                Vec3::new(l.x, h.y, l.z),
                Vec3::new(h.x, h.y, l.z),
                Vec3::new(l.x, l.y, h.z),
                Vec3::new(h.x, l.y, h.z),
                Vec3::new(l.x, h.y, h.z),
                Vec3::new(h.x, h.y, h.z),
            ]
        })
        .unwrap_or([Vec3::ZERO; 8]);
    let far = corners
        .iter()
        .map(|p| from_axis(*p, o, z))
        .fold(0.0, f64::max);
    let size = rec.bbox.diagonal() + far + (lo.abs()).max(hi.abs());
    let boxed = (Img::Rect(0.0, far, lo, hi), 0.0);
    match prim.sym {
        Sym::Axis {
            o: o2,
            z: z2,
            shape,
        } => {
            let (tilt, offset) = off_axis(o2, z2, o, z);
            if !same_axis(tilt, offset, size) {
                return boxed;
            }
            let slope = match shape {
                Shape::Cone { slope, .. } => slope,
                _ => 0.0,
            };
            let margin = offset * (1.0 + slope.abs()) + tilt_margin(rec, tilt, o, o2, slope);
            let at = (o2 - o).dot(z);
            let image = match shape {
                Shape::Cylinder { r } => Img::Seg((r, lo), (r, hi)),
                Shape::Cone { r, slope } => {
                    let sign = if z2.dot(z) < 0.0 { -1.0 } else { 1.0 };
                    let rho = |zeta: f64| (r + slope * sign * (zeta - at)).max(0.0);
                    Img::Seg((rho(lo), lo), (rho(hi), hi))
                }
                Shape::Torus { major, minor } => Img::Circle((major, at), minor),
                Shape::Circle { r } => Img::Seg((r, at), (r, at)),
            };
            (image, margin)
        }
        Sym::Sphere { c, r } => {
            let offset = from_axis(c, o, z);
            if offset > SAME_AXIS * size.max(1.0) {
                return boxed;
            }
            (Img::Circle((0.0, (c - o).dot(z)), r), offset)
        }
        Sym::Plane { o: o2, n } => {
            let tilt = n.cross(z).length();
            if tilt > SAME_AXIS {
                return boxed;
            }
            let h = (o2 - o).dot(z);
            // A plane's piece square to the axis: its distances from the
            // axis's foot, exactly for the box of its parameters.
            let near = match &rec.piece {
                Piece::Plane {
                    o: po, x, y, u, v, ..
                } => {
                    let foot = o + z * (o2 - o).dot(z) - *po;
                    pt_rect((foot.dot(*x), foot.dot(*y)), (u.0, u.1, v.0, v.1))
                }
                _ => 0.0,
            }
            .max(floor);
            let far = far.max(near);
            (
                Img::Seg((near, h), (far, h)),
                tilt_margin(rec, tilt, o, o2, 0.0),
            )
        }
        Sym::Line { o: o2, dir } => {
            let (tilt, _) = off_axis(o2, dir, o, z);
            if tilt > SAME_AXIS {
                return boxed;
            }
            let rho = from_axis(o2, o, z);
            (
                Img::Seg((rho, lo), (rho, hi)),
                tilt_margin(rec, tilt, o, o2, 0.0),
            )
        }
        Sym::None => boxed,
    }
}

/// A piece's image in the plane across the axis `(o, z)` (coordinates
/// along `e1`, `e2`), and a margin.
fn section_image(prim: &Prim<'_>, rec: &Rec, o: Vec3, z: Vec3, e1: Vec3, e2: Vec3) -> (Img, f64) {
    let span = |e: Vec3| (rec.piece.least(e) - o.dot(e), rec.piece.most(e) - o.dot(e));
    let (x, y) = (span(e1), span(e2));
    let boxed = (Img::Rect(x.0, x.1, y.0, y.1), 0.0);
    let at = |p: Vec3| ((p - o).dot(e1), (p - o).dot(e2));
    match prim.sym {
        Sym::Axis {
            o: o2,
            z: z2,
            shape: Shape::Cylinder { r },
        } => {
            let tilt = z2.cross(z).length();
            if tilt > SAME_AXIS {
                return boxed;
            }
            (Img::Circle(at(o2), r), tilt_margin(rec, tilt, o, o2, 0.0))
        }
        Sym::Plane { o: o2, n } => {
            let tilt = n.dot(z).abs();
            if tilt > SAME_AXIS {
                return boxed;
            }
            let Some(t) = z.cross(n).normalized() else {
                return boxed;
            };
            // The plane's trace: the foot of `o2` and the extent along it.
            let (s0, s1) = (
                rec.piece.least(t) - o2.dot(t),
                rec.piece.most(t) - o2.dot(t),
            );
            let (p0, p1) = (o2 + t * s0, o2 + t * s1);
            (Img::Seg(at(p0), at(p1)), tilt_margin(rec, tilt, o, o2, 0.0))
        }
        Sym::Line { o: o2, dir } => {
            let tilt = dir.cross(z).length();
            if tilt > SAME_AXIS {
                return boxed;
            }
            let p = at(o2);
            (Img::Seg(p, p), tilt_margin(rec, tilt, o, o2, 0.0))
        }
        _ => boxed,
    }
}

/// Whether a pair of pieces cannot hold a critical pair: `w = q − p`
/// boxed in a frame about one piece's cone is never along its normals (a
/// surface) or never square to its tangents (a curve).
fn excluded(a: &Rec, b: &Rec) -> bool {
    let range = |e: Vec3| {
        (
            b.piece.least(e) - a.piece.most(e),
            b.piece.most(e) - a.piece.least(e),
        )
    };
    let most_abs = |(lo, hi): (f64, f64)| lo.abs().max(hi.abs());
    for rec in [a, b] {
        let (Some(cone), Some(kind)) = (rec.cone, rec.piece.cone_kind()) else {
            continue;
        };
        let Some((e1, e2)) = square_to(cone.axis) else {
            continue;
        };
        let (w1, w2, wk) = (range(e1), range(e2), range(cone.axis));
        let across_least = least_abs(w1).hypot(least_abs(w2));
        let across_most = most_abs(w1).hypot(most_abs(w2));
        let tan = cone.half_angle.tan();
        let out = match kind {
            ConeKind::Normal => across_least > tan * most_abs(wk),
            ConeKind::Tangent => least_abs(wk) > tan * across_most,
        };
        if out {
            return true;
        }
    }
    false
}

/// Whether the edge may meet the box `b`: its kept pieces' boxes first,
/// then those that meet `b` halved further.
fn edge_near(edge: &Bound<'_>, b: &Bbox3) -> bool {
    if boxes_distance(&edge.bbox, b) > 0.0 {
        return false;
    }
    let leaves = edge.leaves.get_or_init(|| leaves_of(edge));
    let Some(leaves) = leaves else {
        return edge_meets(edge, edge.range, b, EDGE_DEPTH);
    };
    leaves.iter().any(|(t, bbox)| {
        boxes_distance(bbox, b) <= 0.0 && edge_meets(edge, *t, b, EDGE_DEPTH.saturating_sub(5))
    })
}

/// Whether the edge's piece over `t` may meet the box `b`.
fn edge_meets(edge: &Bound<'_>, t: (f64, f64), b: &Bbox3, depth: u32) -> bool {
    if boxes_distance(&edge.bbox, b) > 0.0 && t == edge.range {
        return false;
    }
    let Some(piece) = curve_piece(edge.curve, &edge.nets, t) else {
        return true;
    };
    if boxes_distance(&piece.bbox(), b) > 0.0 {
        return false;
    }
    let m = 0.5 * (t.0 + t.1);
    if depth == 0 || b.contains(edge.curve.eval(m), 0.0) || m <= t.0 || m >= t.1 {
        return true;
    }
    edge_meets(edge, (t.0, m), b, depth - 1) || edge_meets(edge, (m, t.1), b, depth - 1)
}

/// The strata of a face: itself, its edges and its vertices.
fn strata(part: &Part<'_>, side: u8, part_index: usize, face: usize) -> Vec<Key> {
    let body = part.body();
    let mut out = vec![(side, part_index, Kind::Face, face)];
    let mut edges = std::collections::BTreeSet::new();
    let mut vertices = std::collections::BTreeSet::new();
    for coedge in body
        .faces
        .get(face)
        .into_iter()
        .flat_map(|f| f.loops.iter())
        .filter_map(|l| body.loops.get(l.0 as usize))
        .flat_map(|l| l.coedges.iter())
    {
        if edges.insert(coedge.edge.0 as usize)
            && let Some(edge) = body.edges.get(coedge.edge.0 as usize)
        {
            vertices.insert(edge.start.0 as usize);
            vertices.insert(edge.end.0 as usize);
        }
    }
    out.extend(edges.into_iter().map(|e| (side, part_index, Kind::Edge, e)));
    out.extend(
        vertices
            .into_iter()
            .map(|v| (side, part_index, Kind::Vertex, v)),
    );
    out
}

/// Proves the least distance of each pair of faces `pairs` (each `(part,
/// face)` of `a`, then of `b`) as close to `found` as `close` (or as the
/// work allows).
pub(crate) fn prove(
    a: &[Part<'_>],
    b: &[Part<'_>],
    pairs: &[FacePair],
    found: f64,
    close: f64,
    tol: f64,
) -> Proof {
    let mut search = Search {
        parts: [a, b],
        prims: Vec::new(),
        keys: BTreeMap::new(),
        recs: Vec::new(),
        tol,
        failed: std::collections::BTreeSet::new(),
        rays: false,
        floors: Default::default(),
    };
    let mut best = found;
    let mut nearer: Option<Nearer> = None;
    // Pairs of strata, once each, with the pairs of faces they belong to.
    let mut stratum_pairs: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    let mut pair_faces: Vec<FacePair> = Vec::new();
    let mut per_face: Vec<Option<Vec<usize>>> = Vec::new();
    for &(fa, fb) in pairs {
        let (Some(part_a), Some(part_b)) = (a.get(fa.0), b.get(fb.0)) else {
            per_face.push(None);
            continue;
        };
        let (sa, sb) = (strata(part_a, 0, fa.0, fa.1), strata(part_b, 1, fb.0, fb.1));
        let count = sa.len().saturating_mul(sb.len());
        if count > MAX_STRATA_PAIRS
            || stratum_pairs.len().saturating_add(count) > max_total_strata_pairs()
        {
            per_face.push(None);
            continue;
        }
        let mut list = Vec::new();
        let mut ok = true;
        for ka in &sa {
            let Some(pa) = search.prim(*ka) else {
                ok = false;
                break;
            };
            for kb in &sb {
                let Some(pb) = search.prim(*kb) else {
                    ok = false;
                    break;
                };
                let next = pair_faces.len();
                let id = *stratum_pairs.entry((pa, pb)).or_insert(next);
                if id == next {
                    pair_faces.push((fa, fb));
                }
                list.push(id);
            }
            if !ok {
                break;
            }
        }
        per_face.push(ok.then_some(list));
    }
    // The first piece of each stratum is the first record made for it.
    let mut first: BTreeMap<usize, usize> = BTreeMap::new();
    for (i, rec) in search.recs.iter().enumerate() {
        first.entry(rec.prim).or_insert(i);
    }
    let mut least = vec![f64::INFINITY; pair_faces.len()];
    let mut heap = BinaryHeap::new();
    let mut seq = 0_u64;
    let mut work = 0_usize;
    let enough = |best: f64| best - close;
    for (&(pa, pb), &id) in &stratum_pairs {
        let (Some(&ra), Some(&rb)) = (first.get(&pa), first.get(&pb)) else {
            continue;
        };
        // Two faces of one plane: their interiors hold no critical pair
        // apart from where they overlap, which they do not (below).
        if search.one_plane_apart(pa, pb, enough(best))
            || search.plane_rules(pa, pb, rb, enough(best))
            || search.plane_rules(pb, pa, ra, enough(best))
        {
            continue;
        }
        // Strata far apart (most of a face's edges from most of the
        // other's) by their boxes alone, without counting as work.
        if let (Some(x), Some(y), Some(px), Some(py)) = (
            search.recs.get(ra),
            search.recs.get(rb),
            search.prims.get(pa),
            search.prims.get(pb),
        ) {
            let apart = boxes_distance(&x.bbox, &y.bbox) - px.slack - py.slack;
            // Past the work allowed, the rest are bounded by their boxes
            // alone: nothing more is kept for them.
            if apart >= enough(best) || work >= max_nodes() {
                if let Some(slot) = least.get_mut(id) {
                    *slot = slot.min(apart.max(0.0));
                }
                continue;
            }
        }
        work += 1;
        consider(
            &search,
            ra,
            rb,
            id,
            &pair_faces,
            &mut best,
            &mut nearer,
            &mut least,
            &mut heap,
            &mut seq,
            enough,
        );
    }
    while let Some(node) = heap.pop() {
        if node.bound >= enough(best) || work >= max_nodes() {
            if let Some(slot) = least.get_mut(node.pair) {
                *slot = slot.min(node.bound);
            }
            // What is left counts as it stands.
            for rest in heap.drain() {
                if let Some(slot) = least.get_mut(rest.pair) {
                    *slot = slot.min(rest.bound);
                }
            }
            break;
        }
        let (Some(ra), Some(rb)) = (search.recs.get(node.a), search.recs.get(node.b)) else {
            continue;
        };
        // Cut the piece that spreads most along the line between them;
        // where nothing keeps them apart yet (a bound of 0), the length
        // counts too (pieces that lie in one another are cut down to
        // size), and without a line (one box's middle at the other's) the
        // length alone.
        let c = center(&rb.bbox) - center(&ra.bbox);
        let meet = node.bound <= 0.0;
        let size = ra.bbox.diagonal() + rb.bbox.diagonal();
        let d = c.normalized().filter(|_| c.length() > 1e-9 * size);
        let cut = cut_apart(&search, ra, rb, d, meet);
        let Some((which, dir)) = cut else {
            // Nothing left to cut: this bound is final.
            if let Some(slot) = least.get_mut(node.pair) {
                *slot = slot.min(node.bound);
            }
            continue;
        };
        let (keep, split) = if which == 0 {
            (node.b, node.a)
        } else {
            (node.a, node.b)
        };
        let pieces = search.children(split, dir);
        for piece in pieces {
            let (x, y) = if which == 0 {
                (piece, keep)
            } else {
                (keep, piece)
            };
            work += 1;
            consider(
                &search,
                x,
                y,
                node.pair,
                &pair_faces,
                &mut best,
                &mut nearer,
                &mut least,
                &mut heap,
                &mut seq,
                enough,
            );
        }
    }
    #[cfg(test)]
    SPENT.with(|spent| spent.set((work, stratum_pairs.len())));
    // A pair of strata with a piece that could not be enclosed proves
    // nothing: its pairs of faces fall back on the mesh.
    let mut unproven = vec![false; pair_faces.len()];
    for (&(pa, pb), &id) in &stratum_pairs {
        if (search.failed.contains(&pa) || search.failed.contains(&pb))
            && let Some(slot) = unproven.get_mut(id)
        {
            *slot = true;
        }
    }
    let lower = per_face
        .into_iter()
        .map(|list| {
            list.filter(|ids| !ids.iter().any(|id| unproven.get(*id) == Some(&true)))
                .map(|ids| {
                    ids.iter()
                        .filter_map(|id| least.get(*id).copied())
                        .fold(f64::INFINITY, f64::min)
                })
        })
        .collect();
    Proof { lower, nearer }
}

/// Proves, for each pair of faces `(a, b)` of `part` in `pairs`, a bound
/// below the length of every line that leaves face `a` into the material
/// along its inward normal and comes out of the body through face `b`
/// (docs/design/verification.md › *The proven thickness*), as close to
/// `found` as `close` or as the work in `budget` (pairs of pieces, spent
/// from it) allows: then the least bound left, honestly short. `None`
/// for a pair whose faces could not be cut into pieces. The same search
/// as [`prove`] over the faces alone (a line's ends need not be critical
/// for a distance, so no strata), with [`Search::ray_bound`] for the
/// bound of a pair of pieces.
pub(crate) fn prove_rays(
    part: &Part<'_>,
    pairs: &[(usize, usize)],
    found: f64,
    close: f64,
    budget: &mut usize,
) -> Vec<Option<f64>> {
    let parts = std::slice::from_ref(part);
    let mut search = Search {
        parts: [parts, parts],
        prims: Vec::new(),
        keys: BTreeMap::new(),
        recs: Vec::new(),
        tol: part.linear,
        failed: std::collections::BTreeSet::new(),
        rays: true,
        floors: Default::default(),
    };
    let enough = |_: f64| found - close;
    let mut best = found;
    let mut nearer = None;
    let mut least = vec![f64::INFINITY; pairs.len()];
    let mut made: Vec<Option<(usize, usize)>> = Vec::new();
    for &(a, b) in pairs {
        let pa = search.prim((0, 0, Kind::Face, a));
        let pb = search.prim((1, 0, Kind::Face, b));
        made.push(pa.zip(pb));
    }
    let mut first: BTreeMap<usize, usize> = BTreeMap::new();
    for (i, rec) in search.recs.iter().enumerate() {
        first.entry(rec.prim).or_insert(i);
    }
    let mut heap = BinaryHeap::new();
    let mut seq = 0_u64;
    for (id, prims) in made.iter().enumerate() {
        let Some((pa, pb)) = prims else {
            continue;
        };
        let (Some(&ra), Some(&rb)) = (first.get(pa), first.get(pb)) else {
            continue;
        };
        *budget = budget.saturating_sub(1);
        consider(
            &search,
            ra,
            rb,
            id,
            &[],
            &mut best,
            &mut nearer,
            &mut least,
            &mut heap,
            &mut seq,
            enough,
        );
    }
    while let Some(node) = heap.pop() {
        if node.bound >= enough(best) || *budget == 0 {
            if let Some(slot) = least.get_mut(node.pair) {
                *slot = slot.min(node.bound);
            }
            for rest in heap.drain() {
                if let Some(slot) = least.get_mut(rest.pair) {
                    *slot = slot.min(rest.bound);
                }
            }
            break;
        }
        let (Some(ra), Some(rb)) = (search.recs.get(node.a), search.recs.get(node.b)) else {
            continue;
        };
        let c = center(&rb.bbox) - center(&ra.bbox);
        let size = ra.bbox.diagonal() + rb.bbox.diagonal();
        let d = c.normalized().filter(|_| c.length() > 1e-9 * size);
        let Some((which, dir)) = cut_apart(&search, ra, rb, d, node.bound <= 0.0) else {
            if let Some(slot) = least.get_mut(node.pair) {
                *slot = slot.min(node.bound);
            }
            continue;
        };
        let (keep, split) = if which == 0 {
            (node.b, node.a)
        } else {
            (node.a, node.b)
        };
        for piece in search.children(split, dir) {
            let (x, y) = if which == 0 {
                (piece, keep)
            } else {
                (keep, piece)
            };
            *budget = budget.saturating_sub(1);
            consider(
                &search,
                x,
                y,
                node.pair,
                &[],
                &mut best,
                &mut nearer,
                &mut least,
                &mut heap,
                &mut seq,
                enough,
            );
        }
    }
    made.iter()
        .zip(least)
        .map(|(prims, least)| {
            let (pa, pb) = (*prims)?;
            if search.failed.contains(&pa) || search.failed.contains(&pb) {
                return None;
            }
            Some(least)
        })
        .collect()
}

/// The piece and direction to cut: the one that spreads most along `d`
/// (the larger piece where neither spreads).
fn cut_apart(
    search: &Search<'_, '_>,
    ra: &Rec,
    rb: &Rec,
    d: Option<Vec3>,
    meet: bool,
) -> Option<(usize, usize)> {
    let (mut sa, mut sb) = (search.spread(ra, d, meet), search.spread(rb, d, meet));
    let flat =
        |s: &[Option<f64>; 2], r: &Rec| s.iter().flatten().all(|x| *x <= 1e-6 * r.bbox.diagonal());
    if flat(&sa, ra) && flat(&sb, rb) {
        let (da, db) = (ra.bbox.diagonal(), rb.bbox.diagonal());
        for x in sa.iter_mut().flatten() {
            *x += da;
        }
        for x in sb.iter_mut().flatten() {
            *x += db;
        }
    }
    let pick = |s: [Option<f64>; 2]| -> Option<(usize, f64)> {
        match s {
            [Some(x), Some(y)] => Some(if y > x { (1, y) } else { (0, x) }),
            [Some(x), None] => Some((0, x)),
            [None, Some(y)] => Some((1, y)),
            [None, None] => None,
        }
    };
    match (pick(sa), pick(sb)) {
        (Some((da, x)), Some((db, y))) => Some(if y > x { (1, db) } else { (0, da) }),
        (Some((da, _)), None) => Some((0, da)),
        (None, Some((db, _))) => Some((1, db)),
        (None, None) => None,
    }
}

/// Bounds a pair of pieces: kept for cutting when below what is enough,
/// else counted for its pair of strata; a nearer pair of points on the
/// faces taken on the way.
#[allow(clippy::too_many_arguments)]
fn consider(
    search: &Search<'_, '_>,
    a: usize,
    b: usize,
    pair: usize,
    pair_faces: &[FacePair],
    best: &mut f64,
    nearer: &mut Option<Nearer>,
    least: &mut [f64],
    heap: &mut BinaryHeap<Node>,
    seq: &mut u64,
    enough: impl Fn(f64) -> f64,
) {
    let (Some(ra), Some(rb)) = (search.recs.get(a), search.recs.get(b)) else {
        return;
    };
    if ra.inside && rb.inside && !search.rays {
        let d = ra.center.distance(rb.center);
        if d < *best
            && let Some(&(fa, fb)) = pair_faces.get(pair)
        {
            *best = d;
            *nearer = Some(Nearer {
                distance: d,
                a: ra.center,
                b: rb.center,
                face_a: fa,
                face_b: fb,
            });
        }
    }
    let Some(bound) = search.bound(ra, rb, enough(*best)) else {
        return;
    };
    if bound >= enough(*best) {
        if let Some(slot) = least.get_mut(pair) {
            *slot = slot.min(bound);
        }
        return;
    }
    *seq += 1;
    heap.push(Node {
        bound,
        seq: *seq,
        a,
        b,
        pair,
    });
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use crate::verify::Part;
    use crate::{Session, Workspace};
    use craft3d_geom::Surface;
    use serde_json::json;
    use std::sync::Arc;

    /// A STEP block 10 × 20 × 30 whose corner (10, 20, 30) is moved 1e-4
    /// along x, imported loose (as `proven_distance.rs` makes it).
    fn loose_block() -> Arc<craft3d_topo::Body> {
        let dir = std::env::temp_dir().join(format!("craft3d-loose-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let run = |s: &mut Session, id: &str, params: serde_json::Value| {
            s.execute(id, &params)
                .unwrap_or_else(|e| panic!("{id}: {e}"))
        };
        let mut s = Session::new(Workspace::unrestricted());
        run(&mut s, "file.new", json!({"units": "mm"}));
        run(&mut s, "feature.block", json!({"size": [10, 20, 30]}));
        let path = dir.join("block.step");
        run(
            &mut s,
            "file.export",
            json!({"path": path.to_string_lossy()}),
        );
        let text = std::fs::read_to_string(&path).unwrap();
        let vertex_points: Vec<String> = text
            .lines()
            .filter_map(|line| {
                let (_, rest) = line.split_once("=VERTEX_POINT('',#")?;
                Some(format!("#{}=", rest.strip_suffix(");")?))
            })
            .collect();
        let corner = "=CARTESIAN_POINT('',(10.,20.,30.));";
        let moved: String = text
            .lines()
            .map(|line| {
                if line.ends_with(corner) && vertex_points.iter().any(|id| line.starts_with(id)) {
                    line.replace(corner, "=CARTESIAN_POINT('',(10.0001,20.,30.));")
                } else {
                    line.to_string()
                }
            })
            .map(|line| line + "\n")
            .collect();
        assert_ne!(moved, text);
        let loose = dir.join("loose.step");
        std::fs::write(&loose, moved).unwrap();
        let mut s = Session::new(Workspace::unrestricted());
        run(&mut s, "file.new", json!({"units": "mm"}));
        run(
            &mut s,
            "file.import",
            json!({"path": loose.to_string_lossy()}),
        );
        let _ = std::fs::remove_dir_all(&dir);
        let regen = s.document_mut().unwrap().regen();
        let (_, body) = regen.bodies().next().unwrap();
        Arc::clone(body)
    }

    /// Telling a face's piece in from out by its middle needs the piece
    /// clear of the face's edges by the body's tolerance as well as twice
    /// the linear one: a tolerant body's outline (by which the middle is
    /// told) may lie that far from the edges' curves. A piece of the loose
    /// block's top half the tolerance from an edge is not told (before,
    /// only twice the linear tolerance was kept clear, and it was); one
    /// further in or out is.
    #[test]
    fn a_piece_within_a_bodys_tolerance_of_an_edge_is_not_told_by_its_middle() {
        let body = loose_block();
        let tolerance = craft3d_math::Tolerance::new(1e-6, 1e-9).unwrap();
        let part = Part::new(&body, None, &tolerance).unwrap();
        let parts = [part];
        let mut search = Search {
            parts: [&parts, &parts],
            prims: Vec::new(),
            keys: BTreeMap::new(),
            recs: Vec::new(),
            tol: tolerance.linear,
            failed: std::collections::BTreeSet::new(),
            rays: false,
            floors: Default::default(),
        };
        let top = body
            .faces
            .iter()
            .position(|f| match &f.surface {
                Surface::Plane(p) => {
                    p.frame().z().cross(Vec3::Z).length() < 1e-12
                        && (p.frame().origin().z - 30.0).abs() < 1e-9
                }
                _ => false,
            })
            .unwrap();
        let prim = search.prim((0, 0, Kind::Face, top)).unwrap();
        let tolerant = search.prims[prim].tolerant;
        assert!((5e-5..5e-4).contains(&tolerant), "{tolerant}");
        let told = |search: &Search<'_, '_>, x: f64, y: f64| {
            let p = Vec3::new(x, y, 30.0);
            let half = Vec3::new(1e-7, 1e-7, 1e-7);
            search.membership(prim, &Bbox3::from_points([p - half, p + half]), p)
        };
        // Half the tolerance in from the edge along y (x = 10), and from
        // the one along x (y = 20), away from the corner.
        assert_eq!(told(&search, 10.0 - 0.5 * tolerant, 10.0), None);
        assert_eq!(told(&search, 5.0, 20.0 - 0.5 * tolerant), None);
        // Ten tolerances in, and out.
        assert_eq!(told(&search, 10.0 - 10.0 * tolerant, 10.0), Some(true));
        assert_eq!(told(&search, 10.0 + 10.0 * tolerant, 10.0), Some(false));
    }

    #[test]
    fn plane_images_measure_as_drawn() {
        let seg = |a: (f64, f64), b: (f64, f64)| Img::Seg(a, b);
        assert_eq!(
            img_distance(&seg((0.0, 0.0), (1.0, 0.0)), &seg((0.0, 2.0), (1.0, 2.0))),
            2.0
        );
        assert_eq!(
            img_distance(&seg((0.0, 0.0), (2.0, 2.0)), &seg((0.0, 2.0), (2.0, 0.0))),
            0.0
        );
        // A pin of radius 4 in a hole of 10: the segments 6 apart.
        assert_eq!(
            img_distance(
                &seg((4.0, -5.0), (4.0, 5.0)),
                &seg((10.0, 0.0), (10.0, 3.0))
            ),
            6.0
        );
        // Nested circles and a circle round a box.
        let c = |x: f64, y: f64, r: f64| Img::Circle((x, y), r);
        assert!((img_distance(&c(0.0, 0.0, 10.0), &c(1.0, 0.0, 4.0)) - 5.0).abs() < 1e-15);
        assert!((img_distance(&c(0.0, 0.0, 1.0), &c(3.0, 0.0, 1.0)) - 1.0).abs() < 1e-15);
        assert!(
            (img_distance(&c(0.0, 0.0, 10.0), &Img::Rect(-1.0, 1.0, -1.0, 1.0))
                - (10.0 - 2f64.sqrt()))
            .abs()
                < 1e-12
        );
        assert_eq!(
            img_distance(&c(0.0, 0.0, 1.0), &Img::Rect(-2.0, 2.0, -0.1, 0.1)),
            0.0
        );
        assert_eq!(
            img_distance(
                &Img::Rect(0.0, 1.0, 0.0, 1.0),
                &Img::Rect(4.0, 5.0, 5.0, 6.0)
            ),
            5.0
        );
    }
}
