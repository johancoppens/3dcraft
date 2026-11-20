//! A body cut by planes, one side of each kept (`feature.planeCut`,
//! docs/design/plane-cut.md): the feature as data.

use crate::{Expr, FeatureId, PersistentName, PlaneRef};
use serde::{Deserialize, Serialize};

/// Most planes one plane cut holds.
pub const MAX_CUT_PLANES: usize = 256;

/// The highest id a plane of a plane cut has; ids count from 1.
pub const MAX_CUT_PLANE_ID: u32 = u32::MAX - 1;

/// Which side of a plane is kept. The words are those of `measure.below`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CutSide {
    /// The side the plane's normal points away from.
    #[default]
    Below,
    /// The side the normal points to.
    Above,
}

impl CutSide {
    pub const ALL: [CutSide; 2] = [CutSide::Below, CutSide::Above];

    pub fn name(self) -> &'static str {
        match self {
            CutSide::Below => "below",
            CutSide::Above => "above",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|side| side.name() == name)
    }
}

/// How a plane of a plane cut is given.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum CutPlaneBy {
    /// A base plane, a datum plane, or a flat face of an earlier body by
    /// its name (its normal points out of that body).
    Plane { plane: PlaneRef },
    /// The plane through `point` (three lengths) square to `normal`
    /// (three plain numbers or three lengths, any length: only where it
    /// points counts).
    PointNormal { point: [Expr; 3], normal: [Expr; 3] },
}

fn no_offset() -> Expr {
    Expr::new("0")
}

/// One plane of a plane cut.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CutPlane {
    /// Its id in the feature, from 1: what the face it cuts is named by
    /// (`F9:cut(3)`). Never given to another plane of the feature, also
    /// not after this one is deleted ([`PlaneCut::next_plane`]).
    pub id: u32,
    pub by: CutPlaneBy,
    /// How far the plane is moved along its normal: a length.
    #[serde(default = "no_offset")]
    pub offset: Expr,
    #[serde(default)]
    pub side: CutSide,
}

impl CutPlane {
    /// The persistent names the plane refers to: the face it lies on,
    /// when it is given by one.
    pub fn names(&self) -> Vec<&PersistentName> {
        match &self.by {
            CutPlaneBy::Plane { plane } => plane.names(),
            CutPlaneBy::PointNormal { .. } => Vec::new(),
        }
    }

    /// The features the plane is placed by.
    pub fn inputs(&self) -> Vec<FeatureId> {
        match &self.by {
            CutPlaneBy::Plane { plane } => plane.inputs(),
            CutPlaneBy::PointNormal { .. } => Vec::new(),
        }
    }

    /// True when the two planes are given alike in every field but the
    /// id: what a plane given without an id is found again by.
    pub fn same_but_id(&self, other: &CutPlane) -> bool {
        self.by == other.by && self.offset == other.offset && self.side == other.side
    }
}

/// The body of an earlier feature cut by planes: what lies on the kept
/// side of every one of them, the planes applied in the order of the
/// list. The body goes into this feature, unless it keeps it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaneCut {
    pub target: FeatureId,
    /// 1 to [`MAX_CUT_PLANES`] planes.
    pub planes: Vec<CutPlane>,
    /// The id the next new plane gets: above every id a plane of this
    /// feature has or ever had.
    pub next_plane: u32,
    /// The target keeps its body: this feature makes a cut copy.
    #[serde(default)]
    pub keep: bool,
}

impl PlaneCut {
    /// A cut of `target` by `planes`, each given with its id or without
    /// (0): those without get the next free ids in the order of the list.
    /// `None` when an id is past [`MAX_CUT_PLANE_ID`].
    pub fn new(target: FeatureId, planes: Vec<CutPlane>, keep: bool) -> Option<Self> {
        let mut cut = PlaneCut {
            target,
            planes,
            next_plane: 1,
            keep,
        };
        cut.number_new_planes()?;
        Some(cut)
    }

    /// Gives every plane whose id is 0 the next free id, in the order of
    /// the list, after raising the next free id above every id given.
    /// `None` when the ids run out.
    pub fn number_new_planes(&mut self) -> Option<()> {
        for plane in &self.planes {
            if plane.id > MAX_CUT_PLANE_ID {
                return None;
            }
            self.next_plane = self.next_plane.max(plane.id.checked_add(1)?);
        }
        for plane in &mut self.planes {
            if plane.id == 0 {
                if self.next_plane > MAX_CUT_PLANE_ID {
                    return None;
                }
                plane.id = self.next_plane;
                self.next_plane = self.next_plane.checked_add(1)?;
            }
        }
        Some(())
    }

    /// What is wrong with the planes' ids or their number, when something
    /// is: no plane or too many, an id of 0 or past the highest, an id
    /// two planes have, an id not below the next free one.
    pub fn fault(&self) -> Option<String> {
        if self.planes.is_empty() {
            return Some("it has no planes".to_string());
        }
        if self.planes.len() > MAX_CUT_PLANES {
            return Some(format!(
                "it has {} planes; the most is {MAX_CUT_PLANES}",
                self.planes.len()
            ));
        }
        let mut seen = std::collections::BTreeSet::new();
        for (i, plane) in self.planes.iter().enumerate() {
            if plane.id == 0 || plane.id > MAX_CUT_PLANE_ID {
                return Some(format!(
                    "plane {i} has the id {}, which is not from 1 to {MAX_CUT_PLANE_ID}",
                    plane.id
                ));
            }
            if !seen.insert(plane.id) {
                return Some(format!("plane {i}'s id {} is another plane's", plane.id));
            }
            if plane.id >= self.next_plane {
                return Some(format!(
                    "plane {i}'s id {} is not below the next id {}",
                    plane.id, self.next_plane
                ));
            }
        }
        None
    }

    /// Every feature the cut is made from: the target, then what its
    /// planes are placed by, each once.
    pub fn inputs(&self) -> Vec<FeatureId> {
        let mut out = vec![self.target];
        for id in self.planes.iter().flat_map(CutPlane::inputs) {
            if !out.contains(&id) {
                out.push(id);
            }
        }
        out
    }

    /// Every expression with the name of its parameter, in a fixed order:
    /// plane by plane, its point and normal when it has them, then its
    /// offset.
    pub fn exprs(&self) -> Vec<(String, &Expr)> {
        let mut out = Vec::new();
        for (i, plane) in self.planes.iter().enumerate() {
            if let CutPlaneBy::PointNormal { point, normal } = &plane.by {
                for (name, triple) in [("point", point), ("normal", normal)] {
                    for (expr, axis) in triple.iter().zip(["x", "y", "z"]) {
                        out.push((format!("planes.{i}.{name}.{axis}"), expr));
                    }
                }
            }
            out.push((format!("planes.{i}.offset"), &plane.offset));
        }
        out
    }

    pub(crate) fn exprs_mut(&mut self) -> Vec<&mut Expr> {
        let mut out = Vec::new();
        for plane in &mut self.planes {
            if let CutPlaneBy::PointNormal { point, normal } = &mut plane.by {
                out.extend(point.iter_mut());
                out.extend(normal.iter_mut());
            }
            out.push(&mut plane.offset);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use crate::WorldPlane;

    fn base(id: u32) -> CutPlane {
        CutPlane {
            id,
            by: CutPlaneBy::Plane {
                plane: PlaneRef::World {
                    plane: WorldPlane::Xy,
                },
            },
            offset: Expr::new("0"),
            side: CutSide::Below,
        }
    }

    /// Planes without an id get the next free ones in the order of the
    /// list, above every id given; an id is never given twice, also not
    /// one a deleted plane had.
    #[test]
    fn new_planes_get_ids_no_plane_had() {
        let cut = PlaneCut::new(FeatureId(1), vec![base(0), base(7), base(0)], false).unwrap();
        let ids: Vec<u32> = cut.planes.iter().map(|p| p.id).collect();
        assert_eq!(ids, [8, 7, 9]);
        assert_eq!(cut.next_plane, 10);
        assert_eq!(cut.fault(), None);

        // Plane 9 deleted, a new one added: it is 10, not 9.
        let mut edited = cut.clone();
        edited.planes.pop();
        edited.planes.insert(0, base(0));
        edited.number_new_planes().unwrap();
        let ids: Vec<u32> = edited.planes.iter().map(|p| p.id).collect();
        assert_eq!(ids, [10, 8, 7]);
        assert_eq!(edited.next_plane, 11);
        assert_eq!(edited.fault(), None);

        assert!(PlaneCut::new(FeatureId(1), vec![base(u32::MAX)], false).is_none());
    }

    #[test]
    fn faults_of_the_ids_are_said() {
        let mut cut = PlaneCut::new(FeatureId(1), vec![base(0), base(0)], false).unwrap();
        cut.planes[1].id = 1;
        assert!(cut.fault().unwrap().contains("another plane's"));
        cut.planes[1].id = 0;
        assert!(cut.fault().unwrap().contains("not from 1"));
        cut.planes[1].id = 5;
        assert!(cut.fault().unwrap().contains("not below the next id"));
        cut.planes.clear();
        assert!(cut.fault().unwrap().contains("no planes"));
        cut.planes = (1..=257).map(base).collect();
        cut.next_plane = 258;
        assert!(cut.fault().unwrap().contains("the most is 256"));
    }

    #[test]
    fn expressions_are_listed_plane_by_plane() {
        let mut cut = PlaneCut::new(FeatureId(1), vec![base(0), base(0)], false).unwrap();
        cut.planes[1].by = CutPlaneBy::PointNormal {
            point: [Expr::new("a"), Expr::new("b"), Expr::new("c")],
            normal: [Expr::new("0"), Expr::new("0"), Expr::new("1")],
        };
        let names: Vec<String> = cut.exprs().into_iter().map(|(name, _)| name).collect();
        assert_eq!(
            names,
            [
                "planes.0.offset",
                "planes.1.point.x",
                "planes.1.point.y",
                "planes.1.point.z",
                "planes.1.normal.x",
                "planes.1.normal.y",
                "planes.1.normal.z",
                "planes.1.offset",
            ]
        );
        assert_eq!(cut.exprs_mut().len(), 8);
        assert!(base(1).same_but_id(&base(2)));
    }
}
