//! The validity checker: what must hold for a body to be a body.

use crate::{Body, BodyKind, Coedge, EdgeId, Face, FaceId, LoopId, ShellId, VertexId};
use craft3d_geom::Surface;
use craft3d_math::{Bits, Tolerance};
use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Mutex, PoisonError};

/// One thing wrong with a body.
#[derive(Clone, Debug, PartialEq)]
pub enum Defect {
    /// An entity refers to an index that does not exist.
    DanglingReference {
        from: String,
        to: String,
    },
    /// A coordinate or parameter is NaN or infinite.
    NonFinite {
        what: String,
    },
    /// An edge's parameter range is empty or backwards.
    EdgeRange {
        edge: EdgeId,
    },
    /// A vertex is not at the end of its edge's curve.
    VertexOffEdge {
        edge: EdgeId,
        vertex: VertexId,
        distance: f64,
    },
    LoopEmpty {
        loop_: LoopId,
    },
    /// Coedge `at` does not start where the one before it ends.
    LoopNotClosed {
        loop_: LoopId,
        at: usize,
    },
    /// A loop belongs to no face, or to more than one.
    LoopUses {
        loop_: LoopId,
        uses: usize,
    },
    /// A face belongs to no shell, or to more than one.
    FaceUses {
        face: FaceId,
        uses: usize,
    },
    /// An edge of a face's boundary does not lie on the face's surface.
    EdgeOffSurface {
        face: FaceId,
        edge: EdgeId,
        distance: f64,
    },
    /// In a solid every edge is used exactly twice; in a sheet once (an
    /// edge of its boundary) or twice.
    EdgeUses {
        edge: EdgeId,
        uses: usize,
    },
    /// The two uses of an edge run the same way: the faces on either side
    /// disagree about which side is outside.
    EdgeUsedTwiceSameWay {
        edge: EdgeId,
    },
    /// Two different loops of one face use the same edge: the face has no
    /// width there. (A seam, used twice by one loop, is fine.)
    EdgeInTwoLoopsOfFace {
        face: FaceId,
        edge: EdgeId,
    },
    /// A loop of a face lies outside the region its other loops bound (a
    /// hole left outside the face's outer loop, a loop inside a hole).
    /// Held for faces on a plane (`crate::stranded_loops`); one point of
    /// each loop is tested, so two loops that cross are found only where
    /// that point lies on the wrong side.
    StrandedLoop {
        face: FaceId,
        loop_: LoopId,
    },
    /// V − E + F − R, less 2 for every face that is a torus or a torus
    /// with holes in it, is not 2 (shells − genus) for any whole genus ≥ 0.
    Euler {
        vertices: usize,
        edges: usize,
        faces: usize,
        rings: i64,
        shells: usize,
    },
    /// The faces of a shell do not hang together through edges: they are
    /// `parts` pieces, which are shells of their own.
    ShellApart {
        shell: ShellId,
        parts: usize,
    },
    /// An edge is used by faces of two shells, which are then one.
    EdgeInTwoShells {
        edge: EdgeId,
        shells: [ShellId; 2],
    },
    /// One shell's V − E + F − R, less 2 for every face that keeps a
    /// torus's handle, is not 2 (1 − genus) for any whole genus ≥ 0
    /// (checked per shell when a solid has more than one, which the count
    /// over the whole body does not catch where two shells make up for
    /// each other).
    ShellEuler {
        shell: ShellId,
        vertices: usize,
        edges: usize,
        faces: usize,
        rings: i64,
    },
    /// More than two edges of a sheet's boundary end at a vertex: two
    /// pieces of the boundary touch there, and the sheet is pinched to a
    /// point (not a manifold with boundary). `ends` counts the boundary
    /// edge ends at the vertex.
    BoundaryPinch {
        vertex: VertexId,
        ends: usize,
    },
    /// The shells of a solid do not lie inside each other as the outsides
    /// of lumps and their cavities do (found by `craft3d_kernel::lumps`,
    /// which tells it from geometry: a cavity outside every lump, a lump
    /// in the material of another, a shell that encloses no volume).
    ShellNesting {
        shell: ShellId,
        reason: String,
    },
}

impl fmt::Display for Defect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Defect::DanglingReference { from, to } => {
                write!(f, "{from} refers to {to}, which does not exist")
            }
            Defect::NonFinite { what } => write!(f, "{what} is not finite (NaN or infinity)"),
            Defect::EdgeRange { edge } => write!(
                f,
                "edge {} has an empty or reversed parameter range",
                edge.0
            ),
            Defect::VertexOffEdge {
                edge,
                vertex,
                distance,
            } => write!(
                f,
                "vertex {} is {distance:e} away from the end of edge {}",
                vertex.0, edge.0
            ),
            Defect::LoopEmpty { loop_ } => write!(f, "loop {} has no coedges", loop_.0),
            Defect::LoopNotClosed { loop_, at } => write!(
                f,
                "loop {} is not closed: coedge {at} does not start where the previous one ends",
                loop_.0
            ),
            Defect::LoopUses { loop_, uses } => {
                write!(
                    f,
                    "loop {} is used by {uses} faces; it must be used by exactly one",
                    loop_.0
                )
            }
            Defect::FaceUses { face, uses } => {
                write!(
                    f,
                    "face {} is in {uses} shells; it must be in exactly one",
                    face.0
                )
            }
            Defect::EdgeOffSurface {
                face,
                edge,
                distance,
            } => write!(
                f,
                "edge {} is {distance:e} away from the surface of face {}",
                edge.0, face.0
            ),
            Defect::EdgeUses { edge, uses } => write!(
                f,
                "edge {} is used {uses} times; in a solid every edge is shared by exactly two faces, in a sheet by one or two",
                edge.0
            ),
            Defect::EdgeUsedTwiceSameWay { edge } => write!(
                f,
                "edge {} is used twice in the same direction: its two faces are oriented inconsistently",
                edge.0
            ),
            Defect::EdgeInTwoLoopsOfFace { face, edge } => write!(
                f,
                "edge {} is in two different loops of face {}, which leaves the face no width there",
                edge.0, face.0
            ),
            Defect::StrandedLoop { face, loop_ } => write!(
                f,
                "loop {} of face {} does not lie where the face's other loops leave it room: it is outside the region they bound",
                loop_.0, face.0
            ),
            Defect::Euler {
                vertices,
                edges,
                faces,
                rings,
                shells,
            } => write!(
                f,
                "Euler-Poincaré fails: V - E + F - R = {vertices} - {edges} + {faces} - {rings} is not \
                 2 (S - G) for {shells} shells and a whole genus G >= 0"
            ),
            Defect::ShellApart { shell, parts } => write!(
                f,
                "the faces of shell {} fall into {parts} pieces that share no edge; each must be a shell of its own",
                shell.0
            ),
            Defect::EdgeInTwoShells { edge, shells } => write!(
                f,
                "edge {} is used by faces of shells {} and {}, which share no edge in a valid body",
                edge.0, shells[0].0, shells[1].0
            ),
            Defect::ShellEuler {
                shell,
                vertices,
                edges,
                faces,
                rings,
            } => write!(
                f,
                "Euler-Poincaré fails for shell {}: V - E + F - R = {vertices} - {edges} + {faces} - {rings} is not \
                 2 (1 - G) - B for a whole genus G >= 0 and its B loops of boundary edges",
                shell.0
            ),
            Defect::BoundaryPinch { vertex, ends } => write!(
                f,
                "{ends} edges of the sheet's boundary end at vertex {}: the boundary touches itself there, so the sheet is pinched to a point",
                vertex.0
            ),
            Defect::ShellNesting { shell, reason } => {
                write!(f, "shell {}: {reason}", shell.0)
            }
        }
    }
}

/// Points along each edge at which a loop on a surface closed both ways
/// is followed to see whether it goes around.
const HANDLE_STEPS: u32 = 32;

/// Whether a face on a surface closed both ways (a torus) still has the
/// surface's handle: it is the whole surface, or the surface less the
/// discs inside its loops.
///
/// It has not when a loop goes around the axis or around the tube (the
/// face is then a band between two such loops, cut open along them), nor
/// when the face is the patch inside a loop rather than everything
/// outside it. Counts cannot tell the last two apart: the hole a drill
/// leaves in a torus and the plug it takes out have the same loops, walked
/// the other way. Which way is read off in parameter space, where a face
/// is to the left of its loops: the loops of a patch enclose a positive
/// area in total, those of the surface less some discs a negative one.
///
/// `false` for anything that cannot be followed (a dangling reference, a
/// number that is not finite): `check` reports those as what they are.
pub(crate) fn keeps_handle(body: &Body, face: &Face) -> bool {
    let (Some(period_u), Some(period_v)) = (face.surface.period_u(), face.surface.period_v())
    else {
        return false;
    };
    if face.loops.is_empty() {
        return true;
    }
    let near =
        |value: f64, target: f64, period: f64| value + ((target - value) / period).round() * period;
    let mut enclosed = 0.0;
    for loop_id in &face.loops {
        let Some(loop_) = body.loop_(*loop_id) else {
            return false;
        };
        // The loop in parameter space, unrolled: each point as near to
        // the one before it as whole periods bring it.
        let mut polygon: Vec<(f64, f64)> = Vec::new();
        for coedge in &loop_.coedges {
            let Some(edge) = body.edge(coedge.edge) else {
                return false;
            };
            let (t0, t1) = edge.range;
            for k in 0..HANDLE_STEPS {
                let along = (f64::from(k) + 0.5) / f64::from(HANDLE_STEPS);
                let t = if coedge.reversed {
                    t1 + (t0 - t1) * along
                } else {
                    t0 + (t1 - t0) * along
                };
                let Ok((u, v)) = face.surface.project(edge.curve.eval(t)) else {
                    return false;
                };
                polygon.push(match polygon.last() {
                    Some(&(pu, pv)) => (near(u, pu, period_u), near(v, pv, period_v)),
                    None => (u, v),
                });
            }
        }
        let (Some(&(first_u, first_v)), Some(&(last_u, last_v))) =
            (polygon.first(), polygon.last())
        else {
            return false;
        };
        // Back at its start a loop that went around is a period off.
        let around = (near(first_u, last_u, period_u) - first_u).abs() > 0.5 * period_u
            || (near(first_v, last_v, period_v) - first_v).abs() > 0.5 * period_v;
        // Not `around ||`: a NaN has not come back either.
        if around || !(last_u.is_finite() && last_v.is_finite()) {
            return false;
        }
        // Twice the area inside the polygon, counter-clockwise positive,
        // about its first point so that the size of u and v does not
        // enter.
        let after = polygon.iter().skip(1);
        for (&(u0, v0), &(u1, v1)) in polygon.iter().zip(after) {
            enclosed += (u0 - first_u) * (v1 - first_v) - (u1 - first_u) * (v0 - first_v);
        }
    }
    // Loops run the other way in parameter space on a reversed face.
    if face.reversed {
        enclosed = -enclosed;
    }
    enclosed < 0.0
}

/// `(V, E, F, R, S)` and the Euler characteristic `V − E + F − R − 2H`.
///
/// `V − E + F − R = 2 (S − G)` counts every face as a disc with holes:
/// `R` is, per face, its loops beyond the first. A face that is not one
/// enters with what it is instead. A face without loops is a whole closed
/// surface, a sphere, and counts −1 in `R`, which makes a ball come out
/// at 2. A face on a torus that still has the torus's handle (see
/// [`keeps_handle`]: the whole torus, or one with holes in it) is a
/// sphere with that handle added, and each such face takes 2 off (`H`).
fn euler(body: &Body) -> (usize, usize, usize, i64, usize, i64) {
    let (v, e, f) = (body.vertices.len(), body.edges.len(), body.faces.len());
    let mut rings = 0_i64;
    let mut handles = 0_i64;
    for face in &body.faces {
        rings += face.loops.len() as i64 - 1;
        if keeps_handle(body, face) {
            handles += 1;
        }
    }
    let chi = v as i64 - e as i64 + f as i64 - rings - 2 * handles;
    (v, e, f, rings, body.shells.len(), chi)
}

/// The total genus (number of handles) of a solid, when its counts satisfy
/// Euler–Poincaré: `V − E + F − R − 2H = 2 (S − G)`, with `H` the faces
/// that are a torus or a torus with holes in it. `None` when no whole
/// `G ≥ 0` fits.
///
/// Faces are taken for what their surface and loops say they are: a
/// spline surface that closes on itself both ways is not known to, and a
/// face on one counts as a disc with holes.
pub fn genus(body: &Body) -> Option<u32> {
    let (.., shells, chi) = euler(body);
    let twice = 2 * shells as i64 - chi;
    (twice >= 0 && twice % 2 == 0)
        .then(|| u32::try_from(twice / 2).ok())
        .flatten()
}

/// Distance from `p` to `surface`, or `None` when it cannot be computed.
fn distance_to(surface: &Surface, p: craft3d_math::Vec3) -> Option<f64> {
    let (u, v) = surface.project(p).ok()?;
    let d = surface.eval(u, v).distance(p);
    d.is_finite().then_some(d)
}

/// Bodies that passed [`check`] lately, each written out in
/// full with the tolerance it was checked with, every bit of every number
/// (`craft3d_math::Bits`: two are the same only when every number of them
/// is, bit for bit), the most recent last.
static PASSED: Mutex<Vec<Vec<u64>>> = Mutex::new(Vec::new());
/// How many are remembered, and how many words all of them are at most.
const PASSED_BODIES: usize = 8;
const PASSED_WORDS: usize = 1 << 22;

/// [`check`] of `body`, remembering the bodies that pass. In a row of
/// booleans each result is the first operand of the next, and is checked
/// as a result, again by regeneration (which checks every feature's
/// body) and again as an operand; written out, a body is
/// compared at a small part of what checking it costs. Written out as its
/// bits, not as text: formatting every number of a large body cost a
/// quarter of a long row of cuts.
pub fn check_remembered(body: &Body, tolerance: &Tolerance) -> Vec<Defect> {
    let mut key = craft3d_math::bits_of(tolerance);
    body.push_bits(&mut key);
    {
        let mut passed = PASSED.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(at) = passed.iter().position(|k| *k == key) {
            let entry = passed.remove(at);
            passed.push(entry);
            return Vec::new();
        }
    }
    let defects = check(body, tolerance);
    if defects.is_empty() && key.len() <= PASSED_WORDS {
        let mut passed = PASSED.lock().unwrap_or_else(PoisonError::into_inner);
        if !passed.contains(&key) {
            passed.push(key);
        }
        while passed.len() > PASSED_BODIES
            || passed.iter().map(Vec::len).sum::<usize>() > PASSED_WORDS
        {
            let _forgotten = passed.remove(0);
        }
    }
    defects
}

/// Everything wrong with `body`; empty when it is valid.
///
/// Checked: references, finite numbers, edge ranges, vertices on their
/// edges, closed loops, each loop in one face and each face in one shell,
/// boundary edges on their face's surface, and for solids that every edge
/// is used once in each direction and that the counts satisfy
/// Euler–Poincaré. For sheets: every edge is used once (an edge of the
/// boundary) or once each way, no more than two boundary edges end at a
/// vertex, and each shell satisfies Euler–Poincaré with its boundary,
/// `V − E + F − R − 2H = 2 (1 − G) − B` with `B` its loops of boundary
/// edges ([`crate::boundary_loops`]). Also that a face's loops lie inside each
/// other as a face's do, for faces on a plane (`stranded_loops`). Not
/// checked yet: self-intersection, the same for loops on curved faces,
/// and that each face is bounded by its
/// loops (`mass_properties` reports an unbounded face as an error).
///
/// **Allowed: a skin that touches itself in a point.** A solid whose
/// boundary is pinched (two conical dents that meet tip to tip, which is
/// what a profile standing on the axis of a revolution with one corner
/// sweeps in a full turn) passes. Nothing in how such a body hangs
/// together shows the pinch: there is no vertex at the tips, every edge is
/// used twice, and the counts are those of the surface with the two tips
/// told apart. It is contact without overlap, which is geometry, and is
/// to `tess::body_self_intersections` a marginal finding at most. Two lumps
/// that share a point are two shells; in one shell their counts fit no
/// genus and are a [`Defect::Euler`].
pub fn check(body: &Body, tol: &Tolerance) -> Vec<Defect> {
    let mut defects = Vec::new();

    for (i, vertex) in body.vertices.iter().enumerate() {
        if !vertex.point.is_finite() {
            defects.push(Defect::NonFinite {
                what: format!("vertex {i}"),
            });
        }
    }

    for (i, edge) in body.edges.iter().enumerate() {
        let id = EdgeId(i as u32);
        let (t0, t1) = edge.range;
        if !(t0.is_finite() && t1.is_finite()) {
            defects.push(Defect::NonFinite {
                what: format!("the parameter range of edge {i}"),
            });
            continue;
        }
        if t0 >= t1 {
            defects.push(Defect::EdgeRange { edge: id });
        }
        for (vertex, t) in [(edge.start, t0), (edge.end, t1)] {
            match body.vertex(vertex) {
                None => defects.push(Defect::DanglingReference {
                    from: format!("edge {i}"),
                    to: format!("vertex {}", vertex.0),
                }),
                Some(v) => {
                    let distance = edge.curve.eval(t).distance(v.point);
                    // NaN (a non-finite vertex, reported above) is not "off".
                    if distance > tol.linear {
                        defects.push(Defect::VertexOffEdge {
                            edge: id,
                            vertex,
                            distance,
                        });
                    }
                }
            }
        }
    }

    // Loops: closed chains, each used by exactly one face.
    for (i, loop_) in body.loops.iter().enumerate() {
        let id = LoopId(i as u32);
        if loop_.coedges.is_empty() {
            defects.push(Defect::LoopEmpty { loop_: id });
            continue;
        }
        let mut ends = Vec::with_capacity(loop_.coedges.len());
        for coedge in &loop_.coedges {
            match body.coedge_ends(*coedge) {
                Some(pair) => ends.push(pair),
                None => defects.push(Defect::DanglingReference {
                    from: format!("loop {i}"),
                    to: format!("edge {}", coedge.edge.0),
                }),
            }
        }
        if ends.len() == loop_.coedges.len() {
            // Each coedge starts where the previous one (cyclically) ends.
            let previous = ends.iter().cycle().skip(ends.len() - 1);
            for (at, (prev, current)) in previous.zip(&ends).enumerate() {
                if prev.1 != current.0 {
                    defects.push(Defect::LoopNotClosed { loop_: id, at });
                }
            }
        }
    }

    let mut loop_uses = vec![0_usize; body.loops.len()];
    let mut edge_uses: BTreeMap<EdgeId, Vec<bool>> = BTreeMap::new();
    for (i, face) in body.faces.iter().enumerate() {
        let id = FaceId(i as u32);
        // Which loop of this face first used each edge.
        let mut seen_in: BTreeMap<EdgeId, LoopId> = BTreeMap::new();
        for loop_id in &face.loops {
            let (Some(loop_), Some(uses)) =
                (body.loop_(*loop_id), loop_uses.get_mut(loop_id.0 as usize))
            else {
                defects.push(Defect::DanglingReference {
                    from: format!("face {i}"),
                    to: format!("loop {}", loop_id.0),
                });
                continue;
            };
            *uses += 1;
            for Coedge { edge, reversed } in &loop_.coedges {
                edge_uses.entry(*edge).or_default().push(*reversed);
                if *seen_in.entry(*edge).or_insert(*loop_id) != *loop_id {
                    defects.push(Defect::EdgeInTwoLoopsOfFace {
                        face: id,
                        edge: *edge,
                    });
                }
                let Some(e) = body.edge(*edge) else {
                    continue; // Reported with the loop.
                };
                let (t0, t1) = e.range;
                if !(t0.is_finite() && t1.is_finite()) {
                    continue;
                }
                // Ends, middle and quarter points of the edge.
                let worst = (0..=4)
                    .filter_map(|k| {
                        distance_to(
                            &face.surface,
                            e.curve.eval(t0 + (t1 - t0) * f64::from(k) / 4.0),
                        )
                    })
                    .fold(0.0, f64::max);
                if worst > tol.linear {
                    defects.push(Defect::EdgeOffSurface {
                        face: id,
                        edge: *edge,
                        distance: worst,
                    });
                }
            }
        }
    }
    for (i, &uses) in loop_uses.iter().enumerate() {
        if uses != 1 {
            defects.push(Defect::LoopUses {
                loop_: LoopId(i as u32),
                uses,
            });
        }
    }

    let mut face_uses = vec![0_usize; body.faces.len()];
    for (i, shell) in body.shells.iter().enumerate() {
        for face in &shell.faces {
            match face_uses.get_mut(face.0 as usize) {
                Some(uses) => *uses += 1,
                None => defects.push(Defect::DanglingReference {
                    from: format!("shell {i}"),
                    to: format!("face {}", face.0),
                }),
            }
        }
    }
    let shells_sound = face_uses.iter().all(|&uses| uses == 1) && defects.is_empty();
    for (i, &uses) in face_uses.iter().enumerate() {
        if uses != 1 {
            defects.push(Defect::FaceUses {
                face: FaceId(i as u32),
                uses,
            });
        }
    }

    // Loops inside each other, where the face can be charted (planes).
    // Only where the references hold: a dangling one is reported above.
    if defects.is_empty() {
        for (face, loop_) in crate::stranded(body, tol) {
            defects.push(Defect::StrandedLoop { face, loop_ });
        }
    }

    if body.kind == BodyKind::Solid {
        for i in 0..body.edges.len() {
            let edge = EdgeId(i as u32);
            let uses = edge_uses.get(&edge).map_or(&[][..], Vec::as_slice);
            match uses {
                [a, b] if a != b => {}
                [_, _] => defects.push(Defect::EdgeUsedTwiceSameWay { edge }),
                _ => defects.push(Defect::EdgeUses {
                    edge,
                    uses: uses.len(),
                }),
            }
        }
        if genus(body).is_none() {
            let (vertices, edges, faces, rings, shells, _) = euler(body);
            defects.push(Defect::Euler {
                vertices,
                edges,
                faces,
                rings,
                shells,
            });
        }
    } else {
        // A sheet: an edge of its boundary is used once, any other edge
        // once each way.
        for i in 0..body.edges.len() {
            let edge = EdgeId(i as u32);
            let uses = edge_uses.get(&edge).map_or(&[][..], Vec::as_slice);
            match uses {
                [_] => {}
                [a, b] if a != b => {}
                [_, _] => defects.push(Defect::EdgeUsedTwiceSameWay { edge }),
                _ => defects.push(Defect::EdgeUses {
                    edge,
                    uses: uses.len(),
                }),
            }
        }
    }
    if shells_sound {
        shell_defects(body, &mut defects);
    }
    defects
}

/// What holds for each shell on its own, once every face is in exactly one
/// shell and every reference holds: its faces hang together through edges
/// and share no edge with another shell's, and (for a solid of several
/// shells) its counts satisfy Euler–Poincaré.
fn shell_defects(body: &Body, defects: &mut Vec<Defect>) {
    let mut shell_of = vec![ShellId(u32::MAX); body.faces.len()];
    for (s, shell) in body.shells.iter().enumerate() {
        for face in &shell.faces {
            if let Some(slot) = shell_of.get_mut(face.0 as usize) {
                *slot = ShellId(u32::try_from(s).unwrap_or(u32::MAX));
            }
        }
    }
    // An edge's shells, by the faces whose loops use it.
    let mut edge_shell: BTreeMap<EdgeId, ShellId> = BTreeMap::new();
    let mut reported: std::collections::BTreeSet<EdgeId> = std::collections::BTreeSet::new();
    for (f, face) in body.faces.iter().enumerate() {
        let Some(&shell) = shell_of.get(f) else {
            continue;
        };
        for loop_id in &face.loops {
            let Some(loop_) = body.loop_(*loop_id) else {
                continue;
            };
            for coedge in &loop_.coedges {
                let first = *edge_shell.entry(coedge.edge).or_insert(shell);
                if first != shell && reported.insert(coedge.edge) {
                    defects.push(Defect::EdgeInTwoShells {
                        edge: coedge.edge,
                        shells: [first.min(shell), first.max(shell)],
                    });
                }
            }
        }
    }
    // Pieces per shell: the groups of faces hanging together, counted by
    // the shells their faces are in.
    let mut parts = vec![0_usize; body.shells.len()];
    for group in crate::face_groups(body) {
        let mut seen = std::collections::BTreeSet::new();
        for face in group {
            if let Some(&shell) = shell_of.get(face.0 as usize)
                && seen.insert(shell)
                && let Some(count) = parts.get_mut(shell.0 as usize)
            {
                *count += 1;
            }
        }
    }
    for (s, &count) in parts.iter().enumerate() {
        if count > 1 {
            defects.push(Defect::ShellApart {
                shell: ShellId(u32::try_from(s).unwrap_or(u32::MAX)),
                parts: count,
            });
        }
    }
    // At a vertex of a sheet at most two edges of a shell's boundary end
    // (two shells that share a vertex touch, which is geometry's to
    // refuse, as for the lumps of a solid).
    if body.kind == BodyKind::Sheet {
        let mut uses: BTreeMap<EdgeId, usize> = BTreeMap::new();
        for face in &body.faces {
            for loop_ in face.loops.iter().filter_map(|l| body.loop_(*l)) {
                for coedge in &loop_.coedges {
                    *uses.entry(coedge.edge).or_insert(0) += 1;
                }
            }
        }
        let mut ends: BTreeMap<(ShellId, VertexId), usize> = BTreeMap::new();
        for (&edge, _) in uses.iter().filter(|&(_, &n)| n == 1) {
            let (Some(&shell), Some(e)) = (edge_shell.get(&edge), body.edge(edge)) else {
                continue;
            };
            for end in [e.start, e.end] {
                *ends.entry((shell, end)).or_insert(0) += 1;
            }
        }
        for ((_, vertex), count) in ends {
            if count > 2 {
                defects.push(Defect::BoundaryPinch {
                    vertex,
                    ends: count,
                });
            }
        }
    }
    // A sheet's shells are counted one by one always: what one shell's
    // boundary loops allow, another's cannot make up for.
    let sheet = body.kind == BodyKind::Sheet;
    if (body.kind == BodyKind::Solid && body.shells.len() > 1) || sheet {
        for s in 0..body.shells.len() {
            let shell = ShellId(u32::try_from(s).unwrap_or(u32::MAX));
            let Some((vertices, edges, faces, rings, handles)) =
                crate::shells::shell_counts(body, shell)
            else {
                continue;
            };
            let boundary = if sheet {
                match crate::shells::boundary_loops(body, shell) {
                    Some(loops) => i64::try_from(loops).unwrap_or(i64::MAX),
                    None => continue,
                }
            } else {
                0
            };
            let chi = vertices as i64 - edges as i64 + faces as i64 - rings - 2 * handles;
            let twice = 2 - chi - boundary;
            if twice < 0 || twice % 2 != 0 {
                defects.push(Defect::ShellEuler {
                    shell,
                    vertices,
                    edges,
                    faces,
                    rings,
                });
            }
        }
    }
}
