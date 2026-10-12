//! The validity checker: what must hold for a body to be a body.

use crate::{Body, BodyKind, Coedge, EdgeId, FaceId, LoopId, VertexId};
use craft3d_geom::Surface;
use craft3d_math::Tolerance;
use std::collections::BTreeMap;
use std::fmt;

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
    /// In a solid every edge is used exactly twice.
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
    /// V − E + F − R is not 2 (shells − genus) for any whole genus ≥ 0.
    Euler {
        vertices: usize,
        edges: usize,
        faces: usize,
        rings: i64,
        shells: usize,
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
                "edge {} is used {uses} times; in a solid every edge is shared by exactly two faces",
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
        }
    }
}

/// `(V, E, F, R, S)` and the Euler characteristic `V − E + F − R`.
///
/// `R` counts, per face, its loops beyond the first. A face without loops
/// is a whole closed surface and counts −1, which makes a sphere come out
/// at 2; a loopless face closed in both parameter directions (a torus) is
/// a handle and takes 2 off.
fn euler(body: &Body) -> (usize, usize, usize, i64, usize, i64) {
    let (v, e, f) = (body.vertices.len(), body.edges.len(), body.faces.len());
    let mut rings = 0_i64;
    let mut handles = 0_i64;
    for face in &body.faces {
        rings += face.loops.len() as i64 - 1;
        if face.loops.is_empty()
            && face.surface.period_u().is_some()
            && face.surface.period_v().is_some()
        {
            handles += 1;
        }
    }
    let chi = v as i64 - e as i64 + f as i64 - rings - 2 * handles;
    (v, e, f, rings, body.shells.len(), chi)
}

/// The total genus (number of handles) of a solid, when its counts satisfy
/// Euler–Poincaré: `V − E + F − R = 2 (S − G)`.
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

/// Everything wrong with `body`; empty when it is valid.
///
/// Checked: references, finite numbers, edge ranges, vertices on their
/// edges, closed loops, each loop in one face and each face in one shell,
/// boundary edges on their face's surface, and for solids that every edge
/// is used once in each direction and that the counts satisfy
/// Euler–Poincaré. Not checked yet: self-intersection, that loops lie on
/// the right side of each other, and that each face is bounded by its
/// loops (`mass_properties` reports an unbounded face as an error).
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
    for (i, &uses) in face_uses.iter().enumerate() {
        if uses != 1 {
            defects.push(Defect::FaceUses {
                face: FaceId(i as u32),
                uses,
            });
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
    }
    defects
}
