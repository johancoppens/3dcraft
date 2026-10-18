//! Sheets: bodies of faces with a boundary and no volume. A fill makes one
//! from the closed profiles of a sketch; `feature.boolean` trims a sheet by
//! a solid or cuts a solid with one.

use crate::commands::{Run, add_feature, doc_error, feature_ref, spec};
use crate::params::Params;
use crate::{CommandSpec, Session};
use craft3d_doc::{FeatureKind, Fill};
use serde_json::Value;

pub(crate) static SPECS: &[CommandSpec] = &[
    spec!("feature.fill", "Fill", r#"{"sketch": id or name, "name": string = automatic}"#,
        "Adds a sheet: the flat faces the closed profiles of a sketch (or of an offset of them) bound, one face per profile with its holes, in the sketch's plane and facing the way its normal points. A sheet has an area and no volume (document.measure gives its area, volume 0 and no centroid); it is drawn from both sides and exported to STEP as an open shell. feature.boolean with a sheet: a sheet as the target is trimmed by a solid tool (intersect keeps the part inside the solid, subtract the part outside); a solid target is cut by a sheet tool that reaches across it, as by a wall whose material is behind the sheet (subtract keeps the part of the solid in front of the sheet, the side its normal points to; intersect the part behind), and the cut face is named after the sheet's face. Refused: a union with a sheet, two sheets, a sheet that ends inside the solid it cuts or does not pass through it, a piece of the sheet on a face of the solid, a part of the solid the sheet does not cross. Names are those of an extrusion's start cap: the face F5:startCap(e1,e2,e3,e4) (after the curves of its outer loop), its edges F5:startEdge(e2) and vertices F5:startVertex(e1,e2). feature.edit changes sketch.", doc: true, feature_fill),
];

fn feature_fill(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "feature.fill";
    let p = Params::new(ID, params, &["sketch", "name"])?;
    let doc = session.document()?.history.current();
    let sketch = feature_ref(doc, &p, ID, "sketch")?;
    doc.profile_source(sketch).map_err(doc_error(ID))?;
    add_feature(
        session,
        ID,
        p.str("name")?,
        FeatureKind::Fill(Fill { sketch }),
    )
}
