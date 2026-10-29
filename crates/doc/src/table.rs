//! Tables: named grids of values that expressions look up
//! (`belt[profile].pitch`), and that a choice parameter can take its
//! labels from (docs/decisions.md, 2026-10-23: tables are a general
//! document feature; their contents for one kind of product live in Jigs).
//!
//! A table has columns, each with a name and an optional unit, one of
//! which is the key, and rows of cells: a number in its column's unit, or
//! a short text. The key column's cells name the rows: a choice
//! parameter shows them, and an expression may find a row by its key
//! (`belt["GT2"].pitch`) as well as by its number counted from 1.
//!
//! A table is part of the design like a variable: regeneration reads it
//! through the variables' evaluation ([`Document::variable_table`]), so
//! editing it rebuilds what its values reach. It adds no dependency
//! between variables (it is a constant to them).

use crate::{DocError, Document, Expr};
use craft3d_expr::{MAX_COLUMNS, MAX_ROWS, MAX_TEXT_LEN};
use serde::{Deserialize, Serialize};

/// Most tables a document holds.
pub const MAX_TABLES: usize = 256;

/// Most cells all the tables of a document hold together.
pub const MAX_CELLS: usize = 1_000_000;

/// One cell: a number in its column's unit, or a short text. The
/// expression language's own type, so the document and an evaluation
/// share the cells ([`Table::rows`]).
pub use craft3d_expr::Cell as TableCell;

/// Serde for the shared cells: a number, or a string.
mod cells {
    use super::TableCell;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::sync::Arc;

    #[derive(Serialize, Deserialize)]
    #[serde(untagged)]
    enum Repr {
        Number(f64),
        Text(String),
    }

    pub fn serialize<S: Serializer>(
        rows: &Arc<Vec<Vec<TableCell>>>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let repr: Vec<Vec<Repr>> = rows
            .iter()
            .map(|row| {
                row.iter()
                    .map(|cell| match cell {
                        TableCell::Number(v) => Repr::Number(*v),
                        TableCell::Text(t) => Repr::Text(t.clone()),
                    })
                    .collect()
            })
            .collect();
        repr.serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Arc<Vec<Vec<TableCell>>>, D::Error> {
        let repr = Vec::<Vec<Repr>>::deserialize(deserializer)?;
        Ok(Arc::new(
            repr.into_iter()
                .map(|row| {
                    row.into_iter()
                        .map(|cell| match cell {
                            Repr::Number(v) => TableCell::Number(v),
                            Repr::Text(t) => TableCell::Text(t),
                        })
                        .collect()
                })
                .collect(),
        ))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableColumn {
    pub name: String,
    /// The unit symbol its numbers are in (`mm`, `deg`); none for plain
    /// numbers.
    pub unit: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Table {
    pub name: String,
    /// The name of the key column.
    pub key: String,
    pub columns: Vec<TableColumn>,
    /// One cell per column in each row. Shared with the evaluations made
    /// from the document ([`Table::to_expr`] copies no cell); change it
    /// with `Arc::make_mut`.
    #[serde(with = "cells")]
    pub rows: std::sync::Arc<Vec<Vec<TableCell>>>,
}

/// Whether `text` is a text a cell may hold: printable ASCII without `"`
/// and `\`, so that it can be written as a key in quotes.
fn check_cell_text(text: &str) -> Result<(), String> {
    if text.len() > MAX_TEXT_LEN {
        return Err(format!(
            "the text \"{}…\" is longer than {MAX_TEXT_LEN} characters",
            text.chars().take(16).collect::<String>()
        ));
    }
    if let Some(ch) = text
        .chars()
        .find(|c| !(' '..='~').contains(c) || *c == '"' || *c == '\\')
    {
        return Err(format!(
            "the text {text:?} holds {ch:?}; a text is printable ASCII without `\"` and `\\`"
        ));
    }
    Ok(())
}

/// Whether `name` can name a column: an ASCII letter or `_`, then letters,
/// digits and `_`, at most 128 bytes (as it follows `.` in a lookup).
fn check_column_name(name: &str) -> Result<(), String> {
    let mut chars = name.chars();
    let starts = chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
    if !starts
        || !chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        || name.len() > craft3d_expr::MAX_NAME_LEN
    {
        return Err(format!(
            "`{name}` cannot name a column: use ASCII letters, digits and `_`, not starting with a digit"
        ));
    }
    Ok(())
}

impl Table {
    /// Why this table is not one a document may hold, apart from its name
    /// (which [`Document::set_table`] checks against the variables).
    pub fn check(&self) -> Result<(), String> {
        craft3d_expr::check_variable_name(&self.name)
            .map_err(|e| format!("`{}` cannot name a table: {e}", self.name))?;
        if self.columns.is_empty() {
            return Err("a table needs at least one column".into());
        }
        if self.columns.len() > MAX_COLUMNS {
            return Err(format!("a table has at most {MAX_COLUMNS} columns"));
        }
        if self.rows.len() > MAX_ROWS {
            return Err(format!("a table has at most {MAX_ROWS} rows"));
        }
        for (i, column) in self.columns.iter().enumerate() {
            check_column_name(&column.name)?;
            if self.columns.iter().take(i).any(|c| c.name == column.name) {
                return Err(format!("two columns are named `{}`", column.name));
            }
            if let Some(unit) = &column.unit
                && craft3d_expr::Unit::from_symbol(unit).is_none()
            {
                return Err(format!(
                    "column `{}`: `{unit}` is not a unit; use one of um, mm, cm, m, in, ft, deg, rad",
                    column.name
                ));
            }
        }
        let Some(key) = self.columns.iter().position(|c| c.name == self.key) else {
            return Err(format!("the key `{}` is not one of the columns", self.key));
        };
        let mut keys: Vec<String> = Vec::with_capacity(self.rows.len());
        for (r, row) in self.rows.iter().enumerate() {
            if row.len() != self.columns.len() {
                return Err(format!(
                    "row {} has {} cells, and the table has {} columns",
                    r + 1,
                    row.len(),
                    self.columns.len()
                ));
            }
            for cell in row {
                match cell {
                    TableCell::Number(value) if !value.is_finite() => {
                        return Err(format!("row {}: a number is not finite", r + 1));
                    }
                    TableCell::Number(_) => {}
                    TableCell::Text(text) => check_cell_text(text)?,
                }
            }
            let label = row.get(key).map(label).unwrap_or_default();
            if label.trim().is_empty() {
                return Err(format!("row {}: the key `{}` is empty", r + 1, self.key));
            }
            keys.push(label);
        }
        keys.sort();
        if let Some(pair) = keys.windows(2).find(|pair| pair.first() == pair.get(1)) {
            return Err(format!(
                "two rows have the key \"{}\"; a key names one row",
                pair.first().map(String::as_str).unwrap_or_default()
            ));
        }
        Ok(())
    }

    /// How many cells it holds.
    pub fn cells(&self) -> usize {
        self.rows.len().saturating_mul(self.columns.len())
    }

    /// The labels of the rows: the key column's cells, in order.
    pub fn keys(&self) -> Vec<String> {
        let key = self.columns.iter().position(|c| c.name == self.key);
        self.rows
            .iter()
            .map(|row| key.and_then(|k| row.get(k)).map(label).unwrap_or_default())
            .collect()
    }

    /// The table as the expression language takes it. A unit that is not
    /// one (a table that was not checked) reads as no unit.
    pub fn to_expr(&self) -> craft3d_expr::Table {
        craft3d_expr::Table {
            name: self.name.clone(),
            columns: self
                .columns
                .iter()
                .map(|c| craft3d_expr::Column {
                    name: c.name.clone(),
                    unit: c.unit.as_deref().and_then(craft3d_expr::Unit::from_symbol),
                })
                .collect(),
            key: self
                .columns
                .iter()
                .position(|c| c.name == self.key)
                .unwrap_or(0),
            rows: std::sync::Arc::clone(&self.rows),
        }
    }
}

/// A cell as a row's label: a text as it is, a number as `t["2.5"]`
/// finds it.
fn label(cell: &TableCell) -> String {
    craft3d_expr::cell_label(cell)
}

impl Document {
    pub fn table(&self, name: &str) -> Option<&Table> {
        self.tables.iter().find(|t| t.name == name)
    }

    /// Defines a table or replaces it whole (it keeps its place).
    ///
    /// A choice parameter that takes its labels from the table keeps
    /// pointing at the same key: its value and its default are moved to
    /// the row that has that key now. When the key is gone the value (or
    /// the default) becomes 0, which is no choice, so the parameter is
    /// invalid and what looks the row up fails, instead of silently taking
    /// another row; one warning per such parameter is returned.
    pub fn set_table(&mut self, table: Table) -> Result<Vec<String>, DocError> {
        table
            .check()
            .map_err(|reason| DocError::Invalid(format!("table `{}`: {reason}", table.name)))?;
        if self.variable(&table.name).is_some() {
            return Err(DocError::Invalid(format!(
                "`{}` is a variable; a table needs a name of its own",
                table.name
            )));
        }
        if self.output(&table.name).is_some() {
            return Err(DocError::Invalid(format!(
                "`{}` is an output; a table needs a name of its own",
                table.name
            )));
        }
        let others: usize = self
            .tables
            .iter()
            .filter(|t| t.name != table.name)
            .map(Table::cells)
            .sum();
        if others.saturating_add(table.cells()) > MAX_CELLS {
            return Err(DocError::Invalid(format!(
                "the tables of a document hold at most {MAX_CELLS} cells together; table `{}` would make {}",
                table.name,
                others.saturating_add(table.cells())
            )));
        }
        let choosers: Vec<String> = self
            .variables
            .iter()
            .filter(|v| {
                v.public
                    .as_ref()
                    .is_some_and(|p| p.choices_from.as_deref() == Some(table.name.as_str()))
            })
            .map(|v| v.name.clone())
            .collect();
        if !choosers.is_empty() && table.rows.len() > crate::public::MAX_CHOICES {
            return Err(DocError::Invalid(format!(
                "table `{}` gives the choices of {}, and a choice offers at most {} labels; the table has {} rows",
                table.name,
                choosers.join(", "),
                crate::public::MAX_CHOICES,
                table.rows.len()
            )));
        }
        let old_keys = self.table(&table.name).map(Table::keys);
        let new_keys = table.keys();
        let mut warnings = Vec::new();
        if let Some(old_keys) = old_keys {
            for name in &choosers {
                let Some(variable) = self.variables.iter_mut().find(|v| v.name == *name) else {
                    continue;
                };
                let mut lost = Vec::new();
                let remap = |expr: &mut Expr, lost: &mut Vec<String>| {
                    let Ok(q) = craft3d_expr::evaluate(expr.source(), |_| None) else {
                        return;
                    };
                    let k = q.value;
                    if !(q.dimension.is_dimensionless() && k.fract() == 0.0 && k >= 1.0) {
                        return;
                    }
                    let Some(key) = old_keys.get((k as usize).saturating_sub(1)) else {
                        return;
                    };
                    match new_keys.iter().position(|n| n == key) {
                        Some(at) if at + 1 != k as usize => *expr = Expr::new((at + 1).to_string()),
                        Some(_) => {}
                        None => {
                            *expr = Expr::new("0");
                            lost.push(key.clone());
                        }
                    }
                };
                remap(&mut variable.expr, &mut lost);
                if let Some(public) = &mut variable.public {
                    remap(&mut public.default, &mut lost);
                }
                if !lost.is_empty() {
                    warnings.push(format!(
                        "the parameter `{name}` was the row \"{}\" of table `{}`, which it no longer has; it is 0 (no choice) until it is set again",
                        lost.first().map(String::as_str).unwrap_or_default(),
                        table.name
                    ));
                }
            }
        }
        match self.tables.iter_mut().find(|t| t.name == table.name) {
            Some(slot) => *slot = table,
            None => {
                if self.tables.len() >= MAX_TABLES {
                    return Err(DocError::Invalid(format!(
                        "a document holds at most {MAX_TABLES} tables"
                    )));
                }
                self.tables.push(table);
            }
        }
        Ok(warnings)
    }

    /// Removes a table. Expressions that still look values up in it fail
    /// with "unknown table" until it is defined again.
    pub fn remove_table(&mut self, name: &str) -> Result<Table, DocError> {
        let index = self
            .tables
            .iter()
            .position(|t| t.name == name)
            .ok_or_else(|| DocError::Invalid(format!("there is no table named `{name}`")))?;
        Ok(self.tables.remove(index))
    }

    /// What uses the table `name`, in words: variables, features,
    /// assertions and public parameters whose expressions look values up
    /// in it, and choice parameters that take their labels from it.
    pub fn table_users(&self, name: &str) -> Vec<String> {
        let uses = |expr: &Expr| {
            craft3d_expr::Expr::parse(expr.source()).is_ok_and(|e| e.tables().contains(&name))
        };
        let mut users = Vec::new();
        for variable in &self.variables {
            if uses(&variable.expr) {
                users.push(format!("variable `{}`", variable.name));
            }
            if let Some(public) = &variable.public {
                if public.choices_from.as_deref() == Some(name) {
                    users.push(format!("the choices of parameter `{}`", variable.name));
                } else if public.exprs().any(uses) {
                    users.push(format!("the range of parameter `{}`", variable.name));
                }
            }
        }
        for feature in &self.features {
            let formulas = feature.kind.formula_exprs();
            if feature
                .exprs()
                .iter()
                .chain(formulas.iter())
                .any(|(_, e)| uses(e))
            {
                users.push(format!("feature {} ({})", feature.id, feature.name));
            }
        }
        for assertion in &self.assertions {
            if assertion.exprs().into_iter().any(uses) {
                users.push(format!("assertion `{}`", assertion.name));
            }
        }
        for body in &self.bodies {
            if body.exprs().into_iter().any(uses) {
                users.push(format!("the colour or material of {}", body.feature));
            }
        }
        // As `rename_table` renames them. What an assertion or an output
        // measures is text this crate keeps but does not read: the engine
        // looks there.
        for output in &self.outputs {
            if let crate::OutputDef::Expr(expr) = &output.def
                && uses(expr)
            {
                users.push(format!("output `{}`", output.name));
            }
        }
        users
    }

    /// Renames every table whose name has become reserved since the
    /// document was written (a function added to the expression language,
    /// such as `sign` in format version 37) to the first free name
    /// `{name}_1`, `{name}_2`, …, as [`Document::rename_table`] does: the
    /// lookups in every expression and the choice parameters that take
    /// their labels from it. A name is free when no variable, table or
    /// expression uses it. Returns the renames made, old name first, in
    /// the order of the tables. For reading old files only.
    pub fn rename_reserved_tables(&mut self) -> Result<Vec<(String, String)>, DocError> {
        let reserved: Vec<String> = self
            .tables
            .iter()
            .map(|t| t.name.clone())
            .filter(|name| craft3d_expr::is_reserved_name(name))
            .collect();
        let mut renames = Vec::new();
        for old in reserved {
            let mut used: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
            used.extend(self.variables.iter().map(|v| v.name.clone()));
            used.extend(self.tables.iter().map(|t| t.name.clone()));
            let mut sources: Vec<&Expr> = Vec::new();
            for variable in &self.variables {
                sources.push(&variable.expr);
                sources.extend(variable.public.iter().flat_map(crate::PublicParam::exprs));
            }
            let formulas: Vec<&Expr> = self
                .features
                .iter()
                .flat_map(|f| {
                    let mut all: Vec<&Expr> = f.exprs().into_iter().map(|(_, e)| e).collect();
                    all.extend(f.kind.formula_exprs().into_iter().map(|(_, e)| e));
                    all
                })
                .collect();
            sources.extend(formulas);
            for assertion in &self.assertions {
                sources.extend(assertion.exprs());
            }
            for body in &self.bodies {
                sources.extend(body.exprs());
            }
            for expr in sources {
                used.extend(
                    craft3d_expr::names_in(expr.source())
                        .into_iter()
                        .map(str::to_string),
                );
            }
            let taken = |name: &str| used.contains(name) || craft3d_expr::is_reserved_name(name);
            let Some(new) = (1..=used.len().saturating_add(1))
                .map(|k| format!("{old}_{k}"))
                .find(|name| !taken(name))
            else {
                return Err(DocError::Invalid(format!(
                    "the table `{old}` has a name that is now reserved, and no free name was found for it"
                )));
            };
            self.rename_table(&old, &new)?;
            renames.push((old, new));
        }
        Ok(renames)
    }

    /// Renames a table everywhere: the lookups in every expression and the
    /// choice parameters that take their labels from it. All or nothing.
    pub fn rename_table(&mut self, old: &str, new: &str) -> Result<(), DocError> {
        if self.table(old).is_none() {
            return Err(DocError::Invalid(format!(
                "there is no table named `{old}`"
            )));
        }
        if old == new {
            return Ok(());
        }
        if self.table(new).is_some() || self.variable(new).is_some() || self.output(new).is_some() {
            return Err(DocError::Invalid(format!(
                "`{new}` is taken by a table, a variable or an output"
            )));
        }
        craft3d_expr::check_variable_name(new)
            .map_err(|e| DocError::Invalid(format!("`{new}` cannot name a table: {e}")))?;
        let mut doc = self.clone();
        let rename = |expr: &mut Expr| -> Result<(), DocError> {
            let renamed = craft3d_expr::rename_table(expr.source(), old, new);
            if renamed.len() > craft3d_expr::MAX_SOURCE_LEN {
                return Err(DocError::Invalid(format!(
                    "renaming `{old}` would make `{}` longer than {} bytes",
                    expr.source(),
                    craft3d_expr::MAX_SOURCE_LEN
                )));
            }
            *expr = Expr::new(renamed);
            Ok(())
        };
        for variable in &mut doc.variables {
            rename(&mut variable.expr)?;
            if let Some(public) = &mut variable.public {
                for expr in public.exprs_mut() {
                    rename(expr)?;
                }
                if public.choices_from.as_deref() == Some(old) {
                    public.choices_from = Some(new.to_string());
                }
            }
        }
        for feature in &mut doc.features {
            for expr in feature.exprs_mut() {
                rename(expr)?;
            }
            // The formulas of curves look tables up as any expression does.
            for expr in feature.kind.formula_exprs_mut() {
                rename(expr)?;
            }
        }
        for assertion in &mut doc.assertions {
            for expr in assertion.exprs_mut() {
                rename(expr)?;
            }
        }
        for body in &mut doc.bodies {
            for expr in body.exprs_mut() {
                rename(expr)?;
            }
        }
        for output in &mut doc.outputs {
            if let crate::OutputDef::Expr(expr) = &mut output.def {
                rename(expr)?;
            }
        }
        if let Some(table) = doc.tables.iter_mut().find(|t| t.name == old) {
            table.name = new.to_string();
        }
        *self = doc;
        Ok(())
    }

    /// The labels a choice parameter offers: its own `choices`, or the
    /// keys of the table it takes them from (none when that table is
    /// missing).
    pub fn choices_of(&self, public: &crate::PublicParam) -> Vec<String> {
        match &public.choices_from {
            Some(table) => self.table(table).map(Table::keys).unwrap_or_default(),
            None => public.choices.clone(),
        }
    }

    pub(crate) fn validate_tables(&self) -> Result<(), DocError> {
        if self.tables.len() > MAX_TABLES {
            return Err(DocError::Invalid(format!(
                "a document holds at most {MAX_TABLES} tables"
            )));
        }
        let cells: usize = self.tables.iter().map(Table::cells).sum();
        if cells > MAX_CELLS {
            return Err(DocError::Invalid(format!(
                "the tables of a document hold at most {MAX_CELLS} cells together, these {cells}"
            )));
        }
        for (i, table) in self.tables.iter().enumerate() {
            table
                .check()
                .map_err(|reason| DocError::Invalid(format!("table `{}`: {reason}", table.name)))?;
            if self.tables.iter().take(i).any(|t| t.name == table.name) {
                return Err(DocError::Invalid(format!(
                    "two tables are named `{}`",
                    table.name
                )));
            }
            if self.variable(&table.name).is_some() {
                return Err(DocError::Invalid(format!(
                    "`{}` names both a table and a variable",
                    table.name
                )));
            }
        }
        for variable in &self.variables {
            if let Some(from) = variable
                .public
                .as_ref()
                .and_then(|p| p.choices_from.as_ref())
                && self.table(from).is_none()
            {
                return Err(DocError::Invalid(format!(
                    "the parameter `{}` takes its choices from the table `{from}`, which there is not",
                    variable.name
                )));
            }
            if let Some(from) = variable
                .public
                .as_ref()
                .and_then(|p| p.choices_from.as_ref())
                && self
                    .table(from)
                    .is_some_and(|t| t.rows.len() > crate::public::MAX_CHOICES)
            {
                return Err(DocError::Invalid(format!(
                    "the parameter `{}` takes its choices from the table `{from}`, which has more than {} rows",
                    variable.name,
                    crate::public::MAX_CHOICES
                )));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::{ParamKind, PublicParam, Units};

    pub(crate) fn belt() -> Table {
        let column = |name: &str, unit: Option<&str>| TableColumn {
            name: name.into(),
            unit: unit.map(str::to_string),
        };
        Table {
            name: "belt".into(),
            key: "profile".into(),
            columns: vec![
                column("profile", None),
                column("pitch", Some("mm")),
                column("width", Some("mm")),
            ],
            rows: std::sync::Arc::new(vec![
                vec![
                    TableCell::Text("GT2".into()),
                    TableCell::Number(2.0),
                    TableCell::Number(6.0),
                ],
                vec![
                    TableCell::Text("HTD3".into()),
                    TableCell::Number(3.0),
                    TableCell::Number(9.0),
                ],
            ]),
        }
    }

    #[test]
    fn a_table_is_checked_whole() {
        let mut doc = Document::new(Units::default());
        doc.set_table(belt()).unwrap();
        assert_eq!(doc.table("belt").unwrap().keys(), ["GT2", "HTD3"]);
        let bad = |edit: &dyn Fn(&mut Table)| {
            let mut t = belt();
            edit(&mut t);
            Document::new(Units::default())
                .set_table(t)
                .unwrap_err()
                .to_string()
        };
        assert!(bad(&|t| t.key = "nope".into()).contains("key"));
        assert!(
            bad(&|t| std::sync::Arc::make_mut(&mut t.rows)[1][0] = TableCell::Text("GT2".into()))
                .contains("two rows")
        );
        assert!(
            bad(&|t| std::sync::Arc::make_mut(&mut t.rows)[0]
                .pop()
                .map(|_| ())
                .unwrap_or(()))
            .contains("cells")
        );
        assert!(bad(&|t| t.columns[1].unit = Some("inch".into())).contains("not a unit"));
        assert!(bad(&|t| t.columns[2].name = "pitch".into()).contains("two columns"));
        assert!(bad(&|t| t.columns[2].name = "2x".into()).contains("column"));
        assert!(bad(&|t| t.name = "mm".into()).contains("cannot name a table"));
        assert!(
            bad(&|t| std::sync::Arc::make_mut(&mut t.rows)[0][1] = TableCell::Number(f64::NAN))
                .contains("finite")
        );
        assert!(
            bad(&|t| std::sync::Arc::make_mut(&mut t.rows)[0][0] = TableCell::Text("a\"b".into()))
                .contains("printable")
        );
        assert!(
            bad(&|t| std::sync::Arc::make_mut(&mut t.rows)[0][0] = TableCell::Text("x".repeat(65)))
                .contains("longer")
        );
        assert!(
            bad(&|t| std::sync::Arc::make_mut(&mut t.rows)[0][0] = TableCell::Text(" ".into()))
                .contains("empty")
        );
        assert!(
            bad(&|t| t.rows = std::sync::Arc::new(vec![t.rows[0].clone(); MAX_ROWS + 1]))
                .contains("rows")
        );
        // A table and a variable do not share a name.
        doc.set_variable("width", "1 mm").unwrap();
        let mut t = belt();
        t.name = "width".into();
        assert!(doc.set_table(t).is_err());
        assert!(doc.set_variable("belt", "1").is_err());
    }

    #[test]
    fn variables_look_values_up_and_renames_follow() {
        let mut doc = Document::new(Units::default());
        doc.set_table(belt()).unwrap();
        doc.set_variable("profile", "2").unwrap();
        doc.set_variable("pitch", "belt[profile].pitch").unwrap();
        doc.publish_variable(
            "profile",
            PublicParam {
                kind: ParamKind::Choice,
                choices_from: Some("belt".into()),
                default: Expr::new("1"),
                ..PublicParam::default()
            },
        )
        .unwrap();
        doc.validate().unwrap();
        let values = doc.variable_table().unwrap().evaluate();
        assert_eq!(values.value("pitch").unwrap().value, 3.0);
        let public = doc.variable("profile").unwrap().public.clone().unwrap();
        assert_eq!(doc.choices_of(&public), ["GT2", "HTD3"]);
        assert_eq!(
            doc.table_users("belt"),
            ["the choices of parameter `profile`", "variable `pitch`"]
        );
        doc.set_output(crate::Output {
            name: "pitchOut".into(),
            def: crate::OutputDef::Expr(Expr::new("belt[1].pitch")),
        })
        .unwrap();
        assert_eq!(
            doc.table_users("belt"),
            [
                "the choices of parameter `profile`",
                "variable `pitch`",
                "output `pitchOut`"
            ]
        );
        doc.rename_table("belt", "strap").unwrap();
        assert_eq!(
            doc.variable("pitch").unwrap().expr.source(),
            "strap[profile].pitch"
        );
        let public = doc.variable("profile").unwrap().public.clone().unwrap();
        assert_eq!(public.choices_from.as_deref(), Some("strap"));
        doc.validate().unwrap();
        // A variable rename leaves the table and its columns alone.
        doc.rename_variable("profile", "pitch_row").unwrap();
        assert_eq!(
            doc.variable("pitch").unwrap().expr.source(),
            "strap[pitch_row].pitch"
        );
        assert!(doc.rename_table("strap", "pitch").is_err());
        assert!(doc.rename_table("nope", "x").is_err());
        // Nor may a variable take a table's name by a rename.
        let e = doc
            .rename_variable("pitch", "strap")
            .unwrap_err()
            .to_string();
        assert!(e.contains("is a table"), "{e}");
        assert_eq!(
            doc.variable("pitch").unwrap().expr.source(),
            "strap[pitch_row].pitch"
        );
    }

    /// A choice parameter follows its key when the table changes, and
    /// becomes no choice (0) when its key is gone, with a warning.
    #[test]
    fn a_choice_keeps_its_key_when_the_table_changes() {
        let mut doc = Document::new(Units::default());
        doc.set_table(belt()).unwrap();
        doc.set_variable("profile", "2").unwrap();
        doc.publish_variable(
            "profile",
            PublicParam {
                kind: ParamKind::Choice,
                choices_from: Some("belt".into()),
                default: Expr::new("2"),
                ..PublicParam::default()
            },
        )
        .unwrap();
        // A row put in front: HTD3 is row 3 now.
        let mut t = belt();
        let rows = std::sync::Arc::make_mut(&mut t.rows);
        rows.insert(
            0,
            vec![
                TableCell::Text("T5".into()),
                TableCell::Number(5.0),
                TableCell::Number(10.0),
            ],
        );
        assert!(doc.set_table(t.clone()).unwrap().is_empty());
        let profile = doc.variable("profile").unwrap();
        assert_eq!(profile.expr.source(), "3");
        assert_eq!(profile.public.as_ref().unwrap().default.source(), "3");
        // HTD3 gone: 0, never another row.
        std::sync::Arc::make_mut(&mut t.rows).remove(2);
        let warnings = doc.set_table(t).unwrap();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("\"HTD3\""), "{warnings:?}");
        assert_eq!(doc.variable("profile").unwrap().expr.source(), "0");
        // Too many rows for a choice.
        let mut big = belt();
        big.rows = std::sync::Arc::new(
            (0..300)
                .map(|i| {
                    vec![
                        TableCell::Text(format!("r{i}")),
                        TableCell::Number(1.0),
                        TableCell::Number(1.0),
                    ]
                })
                .collect(),
        );
        let e = doc.set_table(big).unwrap_err().to_string();
        assert!(e.contains("at most 256 labels"), "{e}");
    }

    #[test]
    fn the_cells_of_all_tables_are_capped() {
        let mut doc = Document::new(Units::default());
        let wide = |name: &str, rows: usize| Table {
            name: name.into(),
            key: "c0".into(),
            columns: (0..50)
                .map(|i| TableColumn {
                    name: format!("c{i}"),
                    unit: None,
                })
                .collect(),
            rows: std::sync::Arc::new(
                (0..rows)
                    .map(|r| {
                        (0..50)
                            .map(|c| TableCell::Number((r * 50 + c) as f64))
                            .collect()
                    })
                    .collect(),
            ),
        };
        for i in 0..2 {
            doc.set_table(wide(&format!("t{i}"), 10_000)).unwrap();
        }
        let e = doc.set_table(wide("t2", 1)).unwrap_err().to_string();
        assert!(e.contains("1000000 cells"), "{e}");
        doc.validate().unwrap();
    }
}
