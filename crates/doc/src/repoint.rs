//! What a feature inserted at the rollback marker re-pointed
//! (docs/design/names.md › *A feature inserted at the marker*).
//!
//! A feature added at the marker that takes a body takes it before the
//! features after the marker that named that body by its feature's id;
//! those are pointed at the new feature ([`Document::repoint_after`]).
//! That edit is kept in the document, beside the history, as an
//! [`Inserted`] entry: which feature was inserted, and for each parameter
//! it re-pointed the feature it named before. So the re-point can be
//! taken back, without the undo history and after a save and a load:
//! deleting the inserted feature gives every parameter it re-pointed (and
//! still names it) its old feature back, so the document is as if it had
//! never been inserted; an edit of it that no longer takes a body gives
//! the parameters it re-pointed from that body theirs back, and one that
//! takes another body re-points the later features that named that one.
//! Regeneration does not read the entries: a file regenerates as its
//! features are written.

use crate::{DocError, Document, FeatureId};
use serde::{Deserialize, Serialize};

/// A feature inserted at the rollback marker that takes a body, and what
/// its insertion (and its edits since) re-pointed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inserted {
    /// The inserted feature.
    pub feature: FeatureId,
    /// The parameters of later features it pointed at itself, in history
    /// order.
    #[serde(default)]
    pub repointed: Vec<Repointed>,
}

/// One parameter re-pointed at an inserted feature.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Repointed {
    /// The later feature.
    pub feature: FeatureId,
    /// Its parameter ([`crate::FeatureKind::body_params`]: `target`,
    /// `tool`, `source`).
    pub param: String,
    /// The feature the parameter named before.
    pub was: FeatureId,
}

/// Most entries a document keeps: one per feature at most.
pub const MAX_INSERTED: usize = crate::MAX_FEATURES;

impl Document {
    /// The entry of the inserted feature `id`, if it was inserted at the
    /// marker taking a body.
    pub fn inserted(&self, id: FeatureId) -> Option<&Inserted> {
        self.inserted.iter().find(|entry| entry.feature == id)
    }

    /// Records that `id` was inserted at the rollback marker and re-pointed
    /// `changed` (as [`Document::repoint_after`] returns it). An entry
    /// already there gets the new parameters added.
    pub fn record_inserted(
        &mut self,
        id: FeatureId,
        changed: &[(FeatureId, &'static str, FeatureId)],
    ) {
        let added = changed.iter().map(|&(feature, param, was)| Repointed {
            feature,
            param: param.to_string(),
            was,
        });
        if let Some(entry) = self.inserted.iter_mut().find(|e| e.feature == id) {
            entry.repointed.extend(added);
            return;
        }
        self.inserted.push(Inserted {
            feature: id,
            repointed: added.collect(),
        });
    }

    /// Takes back what inserting `id` re-pointed from the features in
    /// `from` (every one when `None`): each such parameter that still names
    /// `id` names the feature it named before again. Returns `(feature,
    /// parameter, the feature it names now)` for each parameter given back,
    /// in history order. The entries taken back are dropped; the entry of
    /// `id` itself stays (it may re-point again when edited).
    pub fn restore_repointed(
        &mut self,
        id: FeatureId,
        from: Option<&[FeatureId]>,
    ) -> Vec<(FeatureId, String, FeatureId)> {
        let Some(at) = self.inserted.iter().position(|e| e.feature == id) else {
            return Vec::new();
        };
        let Some(entry) = self.inserted.get_mut(at) else {
            return Vec::new();
        };
        let (back, kept): (Vec<Repointed>, Vec<Repointed>) = std::mem::take(&mut entry.repointed)
            .into_iter()
            .partition(|r| from.is_none_or(|from| from.contains(&r.was)));
        entry.repointed = kept;
        let mut given = Vec::new();
        for r in &back {
            let Ok(later) = self.feature_mut(r.feature) else {
                continue;
            };
            for (param, named) in later.kind.body_params_mut() {
                if param == r.param && *named == id {
                    *named = r.was;
                    given.push((r.feature, r.param.clone(), r.was));
                }
            }
        }
        // Another feature inserted later that re-pointed one of these
        // parameters from `id` re-pointed it, as far as the document
        // before `id` knows, from what `id` had taken it from.
        for other in self.inserted.iter_mut().filter(|e| e.feature != id) {
            for r in &mut other.repointed {
                if r.was != id {
                    continue;
                }
                if let Some(old) = back
                    .iter()
                    .find(|b| b.feature == r.feature && b.param == r.param)
                {
                    r.was = old.was;
                }
            }
        }
        given
    }

    /// Drops what the document keeps of `id` as an inserted feature, as a
    /// later feature re-pointed by one and as the feature a parameter named
    /// before (deleting the inserted feature then leaves that parameter as
    /// it is rather than naming a feature that is gone), after its
    /// parameters were given back ([`Document::restore_repointed`]); for
    /// [`Document::remove_feature`].
    pub(crate) fn forget_inserted(&mut self, id: FeatureId) {
        self.inserted.retain(|e| e.feature != id);
        for entry in &mut self.inserted {
            entry.repointed.retain(|r| r.feature != id && r.was != id);
        }
    }

    /// Checks the entries of a document from elsewhere: each names a
    /// feature of the document once, and so does each parameter in them.
    pub(crate) fn validate_inserted(&self) -> Result<(), DocError> {
        if self.inserted.len() > MAX_INSERTED {
            return Err(DocError::Invalid(format!(
                "a document keeps at most {MAX_INSERTED} inserted features"
            )));
        }
        for (i, entry) in self.inserted.iter().enumerate() {
            if self.feature(entry.feature).is_none() {
                return Err(DocError::UnknownFeature(entry.feature));
            }
            if self
                .inserted
                .iter()
                .take(i)
                .any(|e| e.feature == entry.feature)
            {
                return Err(DocError::Invalid(format!(
                    "the feature {} is listed twice as inserted",
                    entry.feature
                )));
            }
            if entry.repointed.len() > crate::MAX_FEATURES.saturating_mul(2) {
                return Err(DocError::Invalid(format!(
                    "the feature {} re-pointed more parameters than there are",
                    entry.feature
                )));
            }
            for r in &entry.repointed {
                if self.feature(r.feature).is_none() {
                    return Err(DocError::UnknownFeature(r.feature));
                }
                if self.feature(r.was).is_none() {
                    return Err(DocError::UnknownFeature(r.was));
                }
                if !matches!(r.param.as_str(), "target" | "tool" | "source") {
                    return Err(DocError::Invalid(format!(
                        "the feature {} re-pointed `{}`, which is no parameter that names a body",
                        entry.feature,
                        r.param.chars().take(40).collect::<String>()
                    )));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        Block, Boolean, BooleanOp, Document, Expr, FeatureId, FeatureKind, Orientation, Transform,
        Units,
    };

    fn block() -> FeatureKind {
        FeatureKind::Block(Block {
            position: ["0 mm", "0 mm", "0 mm"].map(Expr::new),
            size: ["1 mm", "1 mm", "1 mm"].map(Expr::new),
            orientation: Orientation::default(),
        })
    }

    fn moved(source: FeatureId) -> FeatureKind {
        FeatureKind::Transform(Transform {
            source,
            translate: ["0 mm", "0 mm", "1 mm"].map(Expr::new),
            pivot: ["0 mm", "0 mm", "0 mm"].map(Expr::new),
            axis: ["0", "0", "1"].map(Expr::new),
            angle: Expr::new("0 deg"),
            keep: false,
        })
    }

    fn target(doc: &Document, id: FeatureId) -> FeatureId {
        match &doc.feature(id).unwrap().kind {
            FeatureKind::Boolean(b) => b.target,
            other => panic!("{other:?}"),
        }
    }

    /// Inserts `kind` right after `after`, re-pointing what it takes, as
    /// the engine does at the marker.
    fn insert(doc: &mut Document, after: FeatureId, name: &str, kind: FeatureKind) -> FeatureId {
        let taken = kind.grows_from();
        let id = doc.add_feature(Some(name), kind).unwrap();
        let at = doc.features.iter().position(|f| f.id == after).unwrap();
        doc.move_feature(id, at + 1).unwrap();
        let changed = doc.repoint_after(id, &taken).unwrap();
        doc.record_inserted(id, &changed);
        id
    }

    /// Two moves inserted one after the other before a subtraction:
    /// deleted in either order, the subtraction names the block again, and
    /// the document is the one before the insertions but for the ids used.
    #[test]
    fn deleting_inserted_features_gives_the_later_ones_back_in_any_order() {
        for first_deleted in [0, 1] {
            let mut doc = Document::new(Units::default());
            let a = doc.add_feature(Some("A"), block()).unwrap();
            let b = doc.add_feature(Some("B"), block()).unwrap();
            let cut = doc
                .add_feature(
                    Some("Cut"),
                    FeatureKind::Boolean(Boolean {
                        op: BooleanOp::Subtract,
                        target: a,
                        tool: b,
                        keep_tool: false,
                    }),
                )
                .unwrap();
            let original = doc.features.clone();
            let x1 = insert(&mut doc, b, "X1", moved(a));
            assert_eq!(target(&doc, cut), x1);
            let x2 = insert(&mut doc, x1, "X2", moved(x1));
            assert_eq!(target(&doc, cut), x2);
            assert_eq!(doc.inserted(x2).unwrap().repointed[0].was, x1);
            let order = if first_deleted == 0 {
                [x1, x2]
            } else {
                [x2, x1]
            };
            doc.remove_feature(order[0]).unwrap();
            if order[0] == x1 {
                // X2 now stands for what X1 took the cut's target from.
                assert_eq!(target(&doc, cut), x2);
                assert_eq!(doc.inserted(x2).unwrap().repointed[0].was, a);
            } else {
                assert_eq!(target(&doc, cut), x1);
            }
            doc.remove_feature(order[1]).unwrap();
            assert_eq!(target(&doc, cut), a);
            assert_eq!(doc.features, original);
            assert!(doc.inserted.is_empty());
            doc.validate().unwrap();
        }
    }

    /// A parameter changed again after the insertion is not given back:
    /// only one that still names the inserted feature is.
    #[test]
    fn only_a_parameter_that_still_names_the_inserted_feature_is_given_back() {
        let mut doc = Document::new(Units::default());
        let a = doc.add_feature(Some("A"), block()).unwrap();
        let b = doc.add_feature(Some("B"), block()).unwrap();
        let c = doc.add_feature(Some("C"), block()).unwrap();
        let cut = doc
            .add_feature(
                Some("Cut"),
                FeatureKind::Boolean(Boolean {
                    op: BooleanOp::Subtract,
                    target: a,
                    tool: b,
                    keep_tool: false,
                }),
            )
            .unwrap();
        let x = insert(&mut doc, c, "X", moved(a));
        if let FeatureKind::Boolean(boolean) = &mut doc.feature_mut(cut).unwrap().kind {
            boolean.target = c;
        }
        assert!(doc.restore_repointed(x, None).is_empty());
        assert_eq!(target(&doc, cut), c);
        // A hostile entry is refused by validation.
        let mut bad = doc.clone();
        bad.inserted[0].feature = FeatureId(999);
        assert!(bad.validate().is_err());
        let mut bad = doc.clone();
        bad.record_inserted(x, &[(cut, "target", a)]);
        bad.inserted[0].repointed[0].param = "faces".into();
        assert!(bad.validate().is_err());
        let mut bad = doc.clone();
        bad.record_inserted(x, &[(cut, "target", FeatureId(999))]);
        assert!(bad.validate().is_err());
    }

    /// Deleting the feature a re-pointed parameter named before forgets
    /// it: deleting the inserted feature afterwards leaves the parameter
    /// naming the inserted feature (gone, so it fails as any feature
    /// naming a deleted one), never a feature that no longer exists.
    #[test]
    fn deleting_what_a_parameter_named_before_forgets_it() {
        let mut doc = Document::new(Units::default());
        let a = doc.add_feature(Some("A"), block()).unwrap();
        let b = doc.add_feature(Some("B"), block()).unwrap();
        let cut = doc
            .add_feature(
                Some("Cut"),
                FeatureKind::Boolean(Boolean {
                    op: BooleanOp::Subtract,
                    target: a,
                    tool: b,
                    keep_tool: false,
                }),
            )
            .unwrap();
        let x = insert(&mut doc, b, "X", moved(a));
        assert_eq!(target(&doc, cut), x);
        doc.remove_feature(a).unwrap();
        assert!(doc.inserted(x).unwrap().repointed.is_empty());
        doc.validate().unwrap();
        doc.remove_feature(x).unwrap();
        assert_eq!(target(&doc, cut), x);
        assert!(doc.inserted.is_empty());
        doc.validate().unwrap();
    }
}
