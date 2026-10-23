//! Which features are hidden in the view: the body a feature makes, a
//! sketch or a datum, taken out of what a viewer draws (the viewport, the
//! window's pictures, `view.render`, `view.pick`) and nothing else.
//!
//! Like the appearance of bodies, this is kept beside the history, not in
//! it: regeneration does not read it, so hiding a feature changes no body,
//! no status and no exported file (unlike `suppressed`, which takes the
//! feature out of the model). It is saved with the document, so a file
//! opens as it was left. Changing it is an undo step all the same, as
//! every change of the document is (decided 2026-10-23, as for colours).

use crate::{DocError, Document, FeatureId};

impl Document {
    /// True when the feature `id` is hidden in the view.
    pub fn is_hidden(&self, id: FeatureId) -> bool {
        self.hidden.binary_search(&id).is_ok()
    }

    /// Hides the feature `id` in the view, or shows it again. The feature
    /// must exist.
    pub fn set_hidden(&mut self, id: FeatureId, hidden: bool) -> Result<(), DocError> {
        if self.feature(id).is_none() {
            return Err(DocError::UnknownFeature(id));
        }
        match (self.hidden.binary_search(&id), hidden) {
            (Err(at), true) => self.hidden.insert(at, id),
            (Ok(at), false) => {
                self.hidden.remove(at);
            }
            _ => {}
        }
        Ok(())
    }

    /// The rules of [`Document::validate`] for the hidden features: in
    /// ascending order of id without repeats, of ids that were given out.
    /// An id of a feature that was deleted is allowed (it hides nothing).
    pub(crate) fn validate_hidden(&self) -> Result<(), DocError> {
        let mut last: Option<FeatureId> = None;
        for &id in &self.hidden {
            if last.is_some_and(|last| last >= id) {
                return Err(DocError::Invalid(
                    "the hidden features are not in the order of their ids, or list one twice"
                        .to_string(),
                ));
            }
            last = Some(id);
            if id.0 == 0 || id.0 >= self.next_feature_id {
                return Err(DocError::Invalid(format!(
                    "hidden feature {id} is an id that was never given out"
                )));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use crate::{Block, Document, Expr, FeatureId, FeatureKind, Orientation, Units};

    fn block() -> FeatureKind {
        FeatureKind::Block(Block {
            position: [Expr::new("0"), Expr::new("0"), Expr::new("0")],
            size: [Expr::new("1"), Expr::new("1"), Expr::new("1")],
            orientation: Orientation::default(),
        })
    }

    #[test]
    fn hiding_keeps_the_ids_in_order_and_once() {
        let mut doc = Document::new(Units::default());
        let a = doc.add_feature(Some("A"), block()).unwrap();
        let b = doc.add_feature(Some("B"), block()).unwrap();
        doc.set_hidden(b, true).unwrap();
        doc.set_hidden(a, true).unwrap();
        doc.set_hidden(a, true).unwrap();
        assert_eq!(doc.hidden, [a, b]);
        assert!(doc.is_hidden(a) && doc.is_hidden(b));
        doc.validate().unwrap();
        doc.set_hidden(a, false).unwrap();
        doc.set_hidden(a, false).unwrap();
        assert_eq!(doc.hidden, [b]);
        // Deleting the feature forgets it.
        doc.remove_feature(b).unwrap();
        assert!(doc.hidden.is_empty());
    }

    #[test]
    fn an_unknown_feature_cannot_be_hidden() {
        let mut doc = Document::new(Units::default());
        assert!(doc.set_hidden(FeatureId(7), true).is_err());
        assert!(doc.hidden.is_empty());
    }

    #[test]
    fn a_bad_list_is_refused_by_validate() {
        let mut doc = Document::new(Units::default());
        let a = doc.add_feature(Some("A"), block()).unwrap();
        doc.hidden = vec![a, a];
        assert!(doc.validate().is_err());
        doc.hidden = vec![FeatureId(99)];
        assert!(doc.validate().is_err());
        doc.hidden = vec![FeatureId(0)];
        assert!(doc.validate().is_err());
    }
}
