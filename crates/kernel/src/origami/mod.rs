//! Origami: a crease pattern on a flat sheet, its rigid folding, and the
//! two local conditions for folding flat (Kawasaki, Maekawa).
//!
//! The sheet is a set of closed profiles of straight lines in a plane; the
//! creases are straight segments inside it, each a mountain, a valley or a
//! cut. [`arrange`] divides the sheet along the creases into *facets* (the
//! faces of the planar arrangement of the boundary and the creases);
//! [`fold`] turns each facet as a rigid body about the creases it shares
//! with its neighbours, by fold angles that are given or solved from the
//! loop-closure conditions; [`folded_sheet`] builds the folded sheet as a
//! body of planar faces; [`vertex_checks`] reads Kawasaki's and Maekawa's
//! conditions at every interior vertex. docs/design/origami.md has the
//! method, the sources and the limits.
//!
//! Sources: E. D. Demaine and J. O'Rourke, *Geometric Folding Algorithms:
//! Linkages, Origami, Polyhedra* (Cambridge University Press, 2007),
//! chapters 11–13 (crease patterns, local flat foldability, rigid
//! origami); T. C. Hull, *Origametry* (Cambridge University Press, 2020);
//! D. A. Huffman, "Curvature and creases: a primer on paper", IEEE
//! Transactions on Computers C-25 (1976) 1010–1019; T. Tachi,
//! "Generalization of rigid-foldable quadrilateral-mesh origami", Journal
//! of the IASS 50 (2009) 173–179; M. Bern and B. Hayes, "The complexity of
//! flat origami", Proc. 7th ACM-SIAM SODA (1996) 175–183.

mod arrange;
mod checks;
mod collide;
mod flat;
mod layers;
mod sheet;
mod solve;
mod sparse;
mod stack;
mod thick;

pub use arrange::{Arrangement, Edge, EdgeKind, Facet, Hinge, arrange};
pub use checks::{VertexCheck, vertex_checks};
pub use collide::{FacetMeeting, Meeting, first_meeting, first_meeting_stacked};
pub use flat::{fold_flat, folds_flat};
pub use layers::{Layers, MAX_LAYER_CONSTRAINTS, MAX_LAYER_WORK, layer_order};
pub use sheet::folded_sheet;
pub use solve::{Angle, Folded, Joint, Linkage, MAX_WORK, fold_linkage, fold_linkage_with};
pub use sparse::MAX_PROFILE;
pub use stack::{Stacked, fold_stacked, folds_stacked};
pub use thick::{PanelSide, Panels, thick_panels};

pub(crate) use crate::profile::{P2, cross, distance as dist, dot, sub};
use craft3d_math::{Bbox3, Transform, Vec3};

use std::fmt;

/// The most creases one pattern may have.
pub const MAX_CREASES: usize = 4096;
/// The most straight boundary segments one sheet may have.
pub const MAX_BOUNDARY: usize = 4096;
/// The most fold angles one fold solves for at once (the work budget and
/// the skyline's size bound the solve further: docs/design/origami.md ›
/// *A sparse solve*).
pub const MAX_FREE: usize = 16_384;
/// Up to this many fold angles the normal equations are solved dense,
/// with full pivoting in the rank test; past it, sparse.
pub const MAX_DENSE: usize = 400;

/// How a crease folds. Seen from the side the sheet's normal points to, a
/// valley folds the paper towards the viewer (the crease sinks), a
/// mountain away from the viewer (the crease rises).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CreaseKind {
    Mountain,
    Valley,
    /// The sheet is cut through along the crease: the facets on its two
    /// sides are not joined there.
    Cut,
}

impl CreaseKind {
    /// The sign of the fold angle: valleys fold by a positive angle,
    /// mountains by a negative one (Tachi's convention); 0 for a cut.
    pub fn sign(self) -> f64 {
        match self {
            CreaseKind::Valley => 1.0,
            CreaseKind::Mountain => -1.0,
            CreaseKind::Cut => 0.0,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            CreaseKind::Mountain => "mountain",
            CreaseKind::Valley => "valley",
            CreaseKind::Cut => "cut",
        }
    }
}

/// A straight crease, in the sheet's plane coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct Crease {
    /// The id of the sketch curve it is (unique among the creases and the
    /// boundary).
    pub id: u64,
    pub start: [f64; 2],
    pub end: [f64; 2],
    pub kind: CreaseKind,
}

/// One straight piece of the sheet's boundary, in the direction the loop
/// runs (outer loops counter-clockwise, holes clockwise: the sheet lies to
/// the left), with the curve it comes from.
#[derive(Clone, Debug, PartialEq)]
pub struct BoundarySegment {
    pub id: u64,
    pub start: [f64; 2],
    pub end: [f64; 2],
    /// True when the loop runs against the curve's own direction.
    pub reversed: bool,
}

/// Why a crease pattern cannot be divided, folded or built. Curves are
/// named by their ids; facets by their index in the [`Arrangement`].
#[derive(Clone, Debug, PartialEq)]
pub enum FoldError {
    /// The tolerance is not positive and finite.
    Tolerance,
    /// More creases or boundary pieces than [`MAX_CREASES`] or
    /// [`MAX_BOUNDARY`].
    TooMany { what: &'static str, limit: usize },
    /// A coordinate is not finite.
    NotFinite { id: u64 },
    /// A crease no longer than the tolerance.
    TooShort { id: u64 },
    /// Two creases, or a crease and the boundary, lie along each other.
    Overlap { a: u64, b: u64 },
    /// Points closer than the tolerance chain into a cluster wider than
    /// it: the pattern is too fine for the tolerance.
    Crowded { id: u64 },
    /// A crease reaches outside the sheet, or over a hole of it.
    Outside { id: u64 },
    /// A crease that ends inside a facet without dividing it (it would
    /// have the same facet on both sides).
    Dangling { id: u64 },
    /// A facet thinner than the tolerance, or one that encloses no area.
    Sliver { facet: usize },
    /// The pattern could not be divided into facets (a bug, or input at
    /// the edge of the tolerance).
    Broken { reason: &'static str },
    /// The point that says which facet stays in place lies in none of
    /// them, or on a crease.
    Fixed,
    /// A fold angle is outside 0 to 180 degrees, or not finite.
    AngleRange { id: u64 },
    /// The angle of a crease is solved for but nothing determines it: it
    /// lies on no closed loop of creases, or the given angles leave it
    /// free together with others.
    Undetermined { ids: Vec<u64> },
    /// More fold angles to solve for than [`MAX_FREE`].
    TooManyFree { count: usize },
    /// The given angles cannot all hold at once: the facets around the
    /// loop through `id` do not close, by `gap` (a length).
    NotClosing { id: u64, gap: f64 },
    /// The solve did not converge within its iterations; `gap` is how far
    /// the facets were from closing at the end.
    NotConverged { gap: f64 },
    /// The solved fold of crease `id` turns the other way from its kind
    /// (or past 180 degrees), by `angle` radians (signed, valley
    /// positive).
    WrongWay { id: u64, angle: f64 },
    /// The continuation from the flat sheet could not follow the fold
    /// past `at` radians of the largest given angle: no step, however
    /// small, found a solution that follows on from the last one.
    NotFollowed { at: f64 },
    /// The fold needs more work than [`MAX_WORK`].
    TooMuchWork,
    /// From the flat sheet two different branches keep every kind: the
    /// solved creases `ids` go one way on one and another way on the other.
    Ambiguous { ids: Vec<u64> },
    /// Two facets of the folded sheet pass through each other or touch
    /// (other than along the creases and at the vertices they share).
    Meets(Meeting),
    /// Thick panels: the clearance does not fit facet `facet`.
    Clearance { facet: usize, reason: &'static str },
    /// Thick panels: a thickness or a clearance that cannot be used.
    Panels { reason: &'static str },
    /// Thick panels: the panels of two facets pass through each other or
    /// touch.
    PanelsMeet(Meeting),
    /// The body could not be built: see the reason.
    Build(String),
    /// Folded flat (every crease by 180 degrees), the facets round the
    /// loop through crease `id` do not meet again, by `gap` (a length):
    /// Kawasaki's condition fails at a vertex of that loop.
    NotFlat { id: u64, gap: f64 },
    /// Folded flat, no order of the layers keeps these facets (by index)
    /// from passing through each other: the mountain-valley assignment
    /// does not fold flat.
    NoLayers { facets: Vec<usize> },
    /// Whether an order of the layers exists was not decided within
    /// [`MAX_LAYER_WORK`] or [`MAX_LAYER_CONSTRAINTS`]: `count` of `what`
    /// were too many (facets on one another in one place, rules, pairs).
    LayersUndecided { what: &'static str, count: usize },
    /// The folded edges could not be divided into cells: see the reason.
    Overlay(String),
    /// Crease `id` folds by 180 degrees while another folds by less: a
    /// sheet folded onto itself is layered only when it is folded flat.
    MixedFlat { id: u64 },
}

impl fmt::Display for FoldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FoldError::Tolerance => write!(f, "the tolerance is not positive and finite"),
            FoldError::TooMany { what, limit } => write!(f, "more than {limit} {what}"),
            FoldError::NotFinite { id } => {
                write!(f, "curve {id} has a coordinate that is not finite")
            }
            FoldError::TooShort { id } => write!(f, "crease {id} is no longer than the tolerance"),
            FoldError::Overlap { a, b } => write!(f, "curves {a} and {b} lie along each other"),
            FoldError::Crowded { id } => write!(
                f,
                "the points near curve {id} lie too close together for the tolerance"
            ),
            FoldError::Outside { id } => write!(f, "crease {id} reaches outside the sheet"),
            FoldError::Dangling { id } => {
                write!(f, "crease {id} ends inside a facet without dividing it")
            }
            FoldError::Sliver { facet } => write!(f, "facet {facet} is thinner than the tolerance"),
            FoldError::Broken { reason } => write!(
                f,
                "the crease pattern could not be divided into facets: {reason}"
            ),
            FoldError::Fixed => write!(
                f,
                "the fixed point lies in no facet (outside the sheet or on a crease)"
            ),
            FoldError::AngleRange { id } => write!(
                f,
                "the fold angle of crease {id} is not between 0 and 180 degrees"
            ),
            FoldError::Undetermined { ids } => {
                write!(f, "nothing determines the fold angles of creases {ids:?}")
            }
            FoldError::TooManyFree { count } => {
                write!(f, "{count} fold angles to solve for, more than {MAX_FREE}")
            }
            FoldError::NotClosing { id, gap } => write!(
                f,
                "the facets around the loop through crease {id} do not close (by {gap:e})"
            ),
            FoldError::NotConverged { gap } => write!(
                f,
                "the fold did not converge (the facets stay {gap:e} apart)"
            ),
            FoldError::WrongWay { id, angle } => {
                write!(f, "crease {id} would fold the other way ({angle} rad)")
            }
            FoldError::Meets(c) => write!(
                f,
                "facets {} and {} {}",
                c.facets[0],
                c.facets[1],
                c.kind.verb()
            ),
            FoldError::NotFollowed { at } => write!(
                f,
                "the fold could not be followed from the flat sheet past {at} rad"
            ),
            FoldError::TooMuchWork => write!(f, "the fold needs more work than {MAX_WORK} steps"),
            FoldError::Ambiguous { ids } => write!(
                f,
                "two branches from the flat sheet keep every kind; creases {ids:?} differ"
            ),
            FoldError::Clearance { facet, reason } => {
                write!(f, "the panel of facet {facet} does not fit: {reason}")
            }
            FoldError::Panels { reason } => write!(f, "{reason}"),
            FoldError::PanelsMeet(c) => write!(
                f,
                "the panels of facets {} and {} {}",
                c.facets[0],
                c.facets[1],
                c.kind.verb()
            ),
            FoldError::Build(reason) => write!(f, "{reason}"),
            FoldError::NotFlat { id, gap } => write!(
                f,
                "folded flat, the facets round the loop through crease {id} do not close (by {gap:e})"
            ),
            FoldError::NoLayers { facets } => {
                write!(f, "no order of the layers keeps facets {facets:?} apart")
            }
            FoldError::LayersUndecided { what, count } => write!(
                f,
                "the order of the layers was not decided within the work allowed ({count} {what})"
            ),
            FoldError::Overlay(reason) => {
                write!(
                    f,
                    "the folded layers could not be divided into cells: {reason}"
                )
            }
            FoldError::MixedFlat { id } => {
                write!(f, "crease {id} folds by 180 degrees while others fold less")
            }
        }
    }
}

impl std::error::Error for FoldError {}

/// The distance from `p` to the segment `a`–`b`, and the parameter of the
/// nearest point (0 at `a`, 1 at `b`).
pub(crate) fn to_segment(p: P2, a: P2, b: P2) -> (f64, f64) {
    let d = sub(b, a);
    let len2 = dot(d, d);
    if len2.is_nan() || len2 <= 0.0 {
        return (dist(p, a), 0.0);
    }
    let t = (dot(sub(p, a), d) / len2).clamp(0.0, 1.0);
    let q = [a[0] + t * d[0], a[1] + t * d[1]];
    (dist(p, q), t)
}

/// Whether `p` lies inside the closed polygon loops (even-odd rule).
/// Points on a loop give either answer; callers test points away from the
/// loops.
pub(crate) fn inside_loops<'a>(p: P2, loops: impl IntoIterator<Item = &'a [P2]>) -> bool {
    let mut inside = false;
    for ring in loops {
        let n = ring.len();
        for i in 0..n {
            let (Some(&a), Some(&b)) = (ring.get(i), ring.get((i + 1) % n)) else {
                continue;
            };
            if (a[1] > p[1]) != (b[1] > p[1]) {
                let x = a[0] + (p[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
                if p[0] < x {
                    inside = !inside;
                }
            }
        }
    }
    inside
}

/// The distance from `p` to the nearest edge of the closed `rings`.
pub(crate) fn ring_distance(rings: &[Vec<P2>], p: P2) -> f64 {
    let mut best = f64::INFINITY;
    for ring in rings {
        let n = ring.len();
        for i in 0..n {
            if let (Some(&a), Some(&b)) = (ring.get(i), ring.get((i + 1) % n)) {
                best = best.min(to_segment(p, a, b).0);
            }
        }
    }
    best
}

pub(crate) fn lift(p: P2) -> Vec3 {
    Vec3::new(p[0], p[1], 0.0)
}

/// Folds the arrangement by `angles` (one per hinge), keeping facet
/// `fixed` in place: the facets as the bodies of a [`Linkage`] whose
/// joints are the creases (axes in the sheet's plane, z = 0), solved
/// about the middle of the sheet's box (so a sheet far from the sketch's
/// origin loses no digits) and returned in the sheet's coordinates.
pub fn fold(
    arr: &Arrangement,
    angles: &[Angle],
    fixed: usize,
    tol: f64,
    angular: f64,
) -> Result<Folded, FoldError> {
    fold_with(arr, angles, fixed, tol, angular, MAX_DENSE)
}

/// [`fold`], with the dense solve up to `dense_up_to` fold angles and the
/// sparse one past it ([`fold_linkage_with`]).
pub fn fold_with(
    arr: &Arrangement,
    angles: &[Angle],
    fixed: usize,
    tol: f64,
    angular: f64,
    dense_up_to: usize,
) -> Result<Folded, FoldError> {
    let box_ = Bbox3::from_points(arr.vertices.iter().map(|&p| lift(p)));
    let centre = box_.center().unwrap_or(Vec3::ZERO);
    let mut joints = Vec::with_capacity(arr.hinges.len());
    for h in &arr.hinges {
        let e = arr.edges.get(h.edge).ok_or(FoldError::Broken {
            reason: "a hinge has no edge",
        })?;
        let (Some(a), Some(b)) = (arr.point(e.from), arr.point(e.to)) else {
            return Err(FoldError::Broken {
                reason: "an edge has no vertex",
            });
        };
        let (a, b) = (lift(a) - centre, lift(b) - centre);
        let direction = (b - a)
            .normalized()
            .ok_or(FoldError::TooShort { id: e.source })?;
        joints.push(Joint::crease(
            h.left,
            h.right,
            a,
            direction,
            Vec3::Z.cross(direction),
            a.distance(b),
            e.source,
        ));
    }
    let link = Linkage {
        bodies: arr.facets.len(),
        joints,
        size: box_.diagonal(),
    };
    let mut folded = fold_linkage_with(&link, angles, fixed, tol, angular, dense_up_to)?;
    let (from, to) = (
        Transform::translation(-centre),
        Transform::translation(centre),
    );
    for t in &mut folded.transforms {
        *t = from.then(t).then(&to);
    }
    Ok(folded)
}
