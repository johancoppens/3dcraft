//! Public parameters and document info: what makes a document a Jig
//! (docs/decisions.md, 2026-10-23).
//!
//! A variable can be *published*: it then carries a [`PublicParam`], the
//! description a parameter form (or an agent) needs to set it without
//! knowing the model: a label, a group, a kind (number, whole number,
//! yes/no, a choice from a list), a range and a step that may be
//! expressions of other variables, the unit to show it in, and the value
//! to reset to. A document with at least one public variable is a Jig.
//!
//! [`DocInfo`] is what a gallery shows of a document: a title, a
//! description, the author, the licence and tags.
//!
//! Like the appearance of bodies, both are beside the history:
//! regeneration does not read them. The document keeps them as data and
//! checks only their shape (lengths, caps, unit symbols); what the
//! expressions evaluate to is checked by the commands that publish a
//! variable and set a parameter (`craft3d-engine`, `param_cmds.rs`),
//! where the variables' values are known.

use crate::{DocError, Document, Expr, MAX_NAME_LEN};
use serde::{Deserialize, Serialize};

/// Longest label, group or choice label, in bytes.
pub const MAX_PARAM_TEXT: usize = 256;
/// Longest description of a parameter or of the document, in bytes.
pub const MAX_DESCRIPTION: usize = 4096;
/// Most choices a choice parameter offers.
pub const MAX_CHOICES: usize = 256;
/// Most tags in the document info.
pub const MAX_TAGS: usize = 32;
/// Longest title, author, licence, url or tag, in bytes.
pub const MAX_INFO_TEXT: usize = 1024;

/// What kind of value a public parameter holds, which decides the widget
/// a form shows for it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ParamKind {
    /// Any value of the variable's dimension (a length, an angle, a plain
    /// number).
    #[default]
    Number,
    /// A plain whole number (a count of teeth).
    Integer,
    /// 0 or 1: off or on.
    Boolean,
    /// A plain whole number from 1 to the number of `choices`, the one
    /// chosen.
    Choice,
}

impl ParamKind {
    pub fn name(self) -> &'static str {
        match self {
            ParamKind::Number => "number",
            ParamKind::Integer => "integer",
            ParamKind::Boolean => "boolean",
            ParamKind::Choice => "choice",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        [
            ParamKind::Number,
            ParamKind::Integer,
            ParamKind::Boolean,
            ParamKind::Choice,
        ]
        .into_iter()
        .find(|kind| kind.name() == name)
    }
}

/// How a published variable is shown and set.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicParam {
    /// What the form calls it ("Number of teeth"); empty shows the
    /// variable's name.
    pub label: String,
    /// One line, shown as the tooltip.
    pub description: String,
    /// The group it is shown in ("Bore"); empty is the top, before every
    /// group.
    pub group: String,
    /// Its place among the parameters; ties keep the variables' order.
    pub order: i32,
    pub kind: ParamKind,
    /// The labels of a [`ParamKind::Choice`], in order: the value 1 is the
    /// first. Empty for the other kinds.
    pub choices: Vec<String>,
    /// The least and the greatest value, and the step a form moves by:
    /// expressions, so a range may depend on other variables (a bore less
    /// than the root diameter).
    pub min: Option<Expr>,
    pub max: Option<Expr>,
    pub step: Option<Expr>,
    /// The unit a form shows the value in (`mm`, `in`, `deg`); none for a
    /// plain number, and the document's for a length or an angle when
    /// absent.
    pub unit: Option<String>,
    /// What a reset goes back to: a constant expression.
    pub default: Expr,
}

impl PublicParam {
    /// Why this description is not one a document may hold: a text too
    /// long, choices on a kind that has none or none on a choice, a unit
    /// that is not one.
    pub fn check(&self) -> Result<(), String> {
        check_text("label", &self.label, MAX_PARAM_TEXT)?;
        check_text("group", &self.group, MAX_PARAM_TEXT)?;
        if self.description.len() > MAX_DESCRIPTION {
            return Err(format!(
                "a description is at most {MAX_DESCRIPTION} bytes long"
            ));
        }
        match self.kind {
            ParamKind::Choice => {
                if self.choices.is_empty() {
                    return Err("a choice needs at least one label under choices".into());
                }
                if self.choices.len() > MAX_CHOICES {
                    return Err(format!("a choice has at most {MAX_CHOICES} labels"));
                }
                for label in &self.choices {
                    if label.trim().is_empty() {
                        return Err("a choice's label must not be empty".into());
                    }
                    check_text("choice label", label, MAX_PARAM_TEXT)?;
                }
            }
            kind => {
                if !self.choices.is_empty() {
                    return Err(format!(
                        "only a choice has choices, and this parameter is a {}",
                        kind.name()
                    ));
                }
            }
        }
        if let Some(unit) = &self.unit
            && craft3d_expr::Unit::from_symbol(unit).is_none()
        {
            return Err(format!(
                "`{unit}` is not a unit; use one of um, mm, cm, m, in, ft, deg, rad"
            ));
        }
        for (what, expr) in [
            ("min", self.min.as_ref()),
            ("max", self.max.as_ref()),
            ("step", self.step.as_ref()),
            ("default", Some(&self.default)),
        ] {
            if let Some(expr) = expr
                && expr.source().len() > craft3d_expr::MAX_SOURCE_LEN
            {
                return Err(format!(
                    "the {what} is longer than {} bytes",
                    craft3d_expr::MAX_SOURCE_LEN
                ));
            }
        }
        Ok(())
    }

    /// The range and step expressions, by what they are.
    pub fn bounds(&self) -> [(&'static str, Option<&Expr>); 3] {
        [
            ("min", self.min.as_ref()),
            ("max", self.max.as_ref()),
            ("step", self.step.as_ref()),
        ]
    }

    /// Every expression it keeps (range, step and default), for the
    /// renames that rewrite expressions (`Document::rename_variable`,
    /// `Document::rename_reserved_variables`).
    pub fn exprs(&self) -> impl Iterator<Item = &Expr> {
        [self.min.as_ref(), self.max.as_ref(), self.step.as_ref()]
            .into_iter()
            .flatten()
            .chain(std::iter::once(&self.default))
    }

    /// [`PublicParam::exprs`], to rewrite them.
    pub fn exprs_mut(&mut self) -> impl Iterator<Item = &mut Expr> {
        [&mut self.min, &mut self.max, &mut self.step]
            .into_iter()
            .flatten()
            .chain(std::iter::once(&mut self.default))
    }
}

/// What a gallery shows of a document. Every field may be empty.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocInfo {
    pub title: String,
    pub description: String,
    pub author: String,
    /// An SPDX identifier, such as `CC-BY-4.0` or `MIT`.
    pub licence: String,
    /// Where the document comes from or is explained.
    pub url: String,
    pub tags: Vec<String>,
}

impl DocInfo {
    pub fn is_empty(&self) -> bool {
        *self == DocInfo::default()
    }

    /// Why this info is not one a document may hold.
    pub fn check(&self) -> Result<(), String> {
        for (what, text) in [
            ("title", &self.title),
            ("author", &self.author),
            ("licence", &self.licence),
            ("url", &self.url),
        ] {
            check_text(what, text, MAX_INFO_TEXT)?;
        }
        if self.description.len() > MAX_DESCRIPTION {
            return Err(format!(
                "the description is at most {MAX_DESCRIPTION} bytes long"
            ));
        }
        if self.tags.len() > MAX_TAGS {
            return Err(format!("there are at most {MAX_TAGS} tags"));
        }
        for tag in &self.tags {
            if tag.trim().is_empty() || tag.trim() != tag {
                return Err("a tag must not be empty or start or end with a space".into());
            }
            check_text("tag", tag, MAX_NAME_LEN)?;
        }
        Ok(())
    }
}

/// A one-line text: at most `max` bytes and no control characters.
fn check_text(what: &str, text: &str, max: usize) -> Result<(), String> {
    if text.len() > max {
        return Err(format!("the {what} is at most {max} bytes long"));
    }
    if text.chars().any(char::is_control) {
        return Err(format!("the {what} must not contain control characters"));
    }
    Ok(())
}

impl Document {
    /// Whether the document is a Jig: it has a public variable.
    pub fn is_jig(&self) -> bool {
        self.variables.iter().any(|v| v.public.is_some())
    }

    /// Publishes the variable `name` with `public`, or replaces what it
    /// had. Only the shape is checked here ([`PublicParam::check`]).
    pub fn publish_variable(&mut self, name: &str, public: PublicParam) -> Result<(), DocError> {
        public
            .check()
            .map_err(|reason| DocError::Invalid(format!("`{name}`: {reason}")))?;
        let variable = self
            .variables
            .iter_mut()
            .find(|v| v.name == name)
            .ok_or_else(|| craft3d_expr::VariableError::UnknownVariable {
                name: name.to_string(),
            })?;
        variable.public = Some(public);
        Ok(())
    }

    /// Makes the variable `name` private again; its value stays.
    pub fn unpublish_variable(&mut self, name: &str) -> Result<(), DocError> {
        let variable = self
            .variables
            .iter_mut()
            .find(|v| v.name == name)
            .ok_or_else(|| craft3d_expr::VariableError::UnknownVariable {
                name: name.to_string(),
            })?;
        variable.public = None;
        Ok(())
    }

    /// Replaces the document info.
    pub fn set_info(&mut self, info: DocInfo) -> Result<(), DocError> {
        info.check()
            .map_err(|reason| DocError::Invalid(format!("document info: {reason}")))?;
        self.info = info;
        Ok(())
    }

    pub(crate) fn validate_public(&self) -> Result<(), DocError> {
        for variable in &self.variables {
            if let Some(public) = &variable.public {
                public.check().map_err(|reason| {
                    DocError::Invalid(format!("the public variable `{}`: {reason}", variable.name))
                })?;
            }
        }
        self.info
            .check()
            .map_err(|reason| DocError::Invalid(format!("document info: {reason}")))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::Units;

    fn doc() -> Document {
        let mut doc = Document::new(Units::default());
        doc.set_variable("teeth", "20").unwrap();
        doc.set_variable("bore", "5 mm").unwrap();
        doc
    }

    #[test]
    fn publishing_needs_a_variable_and_a_sound_description() {
        let mut doc = doc();
        let public = PublicParam {
            label: "Teeth".into(),
            kind: ParamKind::Integer,
            default: Expr::new("20"),
            ..PublicParam::default()
        };
        assert!(doc.publish_variable("nope", public.clone()).is_err());
        doc.publish_variable("teeth", public.clone()).unwrap();
        assert!(doc.is_jig());
        doc.validate().unwrap();

        let choice_without_labels = PublicParam {
            kind: ParamKind::Choice,
            ..public.clone()
        };
        assert!(
            doc.publish_variable("teeth", choice_without_labels)
                .is_err()
        );
        let labels_on_a_number = PublicParam {
            choices: vec!["a".into()],
            ..public.clone()
        };
        assert!(doc.publish_variable("teeth", labels_on_a_number).is_err());
        let bad_unit = PublicParam {
            unit: Some("furlong".into()),
            ..public.clone()
        };
        assert!(doc.publish_variable("teeth", bad_unit).is_err());
        let long = PublicParam {
            label: "x".repeat(MAX_PARAM_TEXT + 1),
            ..public
        };
        assert!(doc.publish_variable("teeth", long).is_err());

        doc.unpublish_variable("teeth").unwrap();
        assert!(!doc.is_jig());
    }

    #[test]
    fn a_rename_keeps_the_description_and_renames_inside_the_ranges() {
        let mut doc = doc();
        doc.publish_variable(
            "bore",
            PublicParam {
                max: Some(Expr::new("teeth * 0.5 mm")),
                default: Expr::new("5 mm"),
                ..PublicParam::default()
            },
        )
        .unwrap();
        doc.rename_variable("teeth", "z").unwrap();
        let bore = doc.variable("bore").unwrap().public.as_ref().unwrap();
        assert_eq!(bore.max.as_ref().unwrap().source(), "z * 0.5 mm");
        doc.rename_variable("bore", "hole").unwrap();
        assert!(doc.variable("hole").unwrap().public.is_some());
    }

    /// A variable of an old file whose name is now a function is renamed
    /// inside the ranges and defaults of public parameters too.
    #[test]
    fn a_reserved_name_is_renamed_inside_public_parameters() {
        let mut doc = Document::new(Units::default());
        for (name, expr) in [("floor", "3 mm"), ("w", "5 mm")] {
            doc.variables.push(crate::Variable {
                name: name.into(),
                expr: Expr::new(expr),
                public: None,
            });
        }
        doc.variables[1].public = Some(PublicParam {
            min: Some(Expr::new("floor")),
            max: Some(Expr::new("floor * 2")),
            default: Expr::new("5 mm"),
            ..PublicParam::default()
        });
        let renames = doc.rename_reserved_variables().unwrap();
        assert_eq!(renames, [("floor".to_string(), "floor_1".to_string())]);
        let w = doc.variable("w").unwrap().public.as_ref().unwrap();
        assert_eq!(w.min.as_ref().unwrap().source(), "floor_1");
        assert_eq!(w.max.as_ref().unwrap().source(), "floor_1 * 2");
        assert_eq!(w.default.source(), "5 mm");
    }

    #[test]
    fn info_is_checked() {
        let mut doc = doc();
        let mut info = DocInfo {
            title: "Pulley".into(),
            tags: vec!["gt2".into()],
            ..DocInfo::default()
        };
        doc.set_info(info.clone()).unwrap();
        info.tags.push(" spaced".into());
        assert!(doc.set_info(info.clone()).is_err());
        info.tags.pop();
        info.title = "a\u{7}".into();
        assert!(doc.set_info(info).is_err());
        assert_eq!(doc.info.title, "Pulley");
    }
}
