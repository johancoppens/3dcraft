//! Small questions every operation asks of a body: which faces use an
//! edge, and which way a face points out of the body near a point.

use crate::{Body, Coedge, Face, FaceId};
use craft3d_geom::GeomError;
use craft3d_math::Vec3;

/// One use of an edge by a loop of a face.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EdgeUse {
    pub face: FaceId,
    /// The loop runs against the edge's direction.
    pub reversed: bool,
}

/// Per edge of `body`, every use of it by a loop of a face, in face order
/// and within a face in loop order (twice for a seam).
pub fn edge_uses(body: &Body) -> Vec<Vec<EdgeUse>> {
    per_edge(body, |face, coedge| EdgeUse {
        face,
        reversed: coedge.reversed,
    })
}

/// Per edge of `body`, the faces that use it, in face order (twice the
/// same face for a seam).
pub fn edge_faces(body: &Body) -> Vec<Vec<FaceId>> {
    per_edge(body, |face, _| face)
}

/// Per edge of `body`, `item` of every use of it, in face order and within
/// a face in loop order.
fn per_edge<T: Clone>(body: &Body, item: impl Fn(FaceId, &Coedge) -> T) -> Vec<Vec<T>> {
    let mut uses: Vec<Vec<T>> = vec![Vec::new(); body.edges.len()];
    for (index, face) in body.faces.iter().enumerate() {
        let id = FaceId(u32::try_from(index).unwrap_or(u32::MAX));
        for coedge in face
            .loops
            .iter()
            .filter_map(|l| body.loop_(*l))
            .flat_map(|l| &l.coedges)
        {
            if let Some(list) = uses.get_mut(coedge.edge.0 as usize) {
                list.push(item(id, coedge));
            }
        }
    }
    uses
}

/// The parameters of a point of a face and the face's unit outward normal
/// there.
pub type Outward = ((f64, f64), Vec3);

impl Face {
    /// The parameters of the point of the face's surface nearest `p`, and
    /// the face's unit outward normal there (the surface's, turned round
    /// for a reversed face). `Ok(None)` where the surface has no normal
    /// there (a cone's apex); `Err` when `p` cannot be projected.
    pub fn nearest(&self, p: Vec3) -> Result<Option<Outward>, GeomError> {
        let (u, v) = self.surface.project(p)?;
        Ok(self
            .surface
            .normal(u, v)
            .map(|normal| ((u, v), if self.reversed { -normal } else { normal })))
    }

    /// The face's unit outward normal at the point of it nearest `p`:
    /// [`Face::nearest`], `None` where there is none.
    pub fn outward(&self, p: Vec3) -> Option<Vec3> {
        self.nearest(p).ok().flatten().map(|(_, normal)| normal)
    }
}
