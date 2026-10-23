//! Tables that expressions look values up in: `belt[profile].pitch`.
//!
//! A table has named columns, each with an optional unit, and rows of
//! cells; one column is the key, whose cells name the rows. A row is found
//! by its number, counted from 1 (`belt[2]`, `belt[profile]` with a
//! variable that holds the number, as a choice parameter does), or by its
//! key written as a text (`belt["GT2"]`). A number cell is taken in its
//! column's unit, so the lookup has that column's dimension; a text cell
//! can name a row but is not a value.
//!
//! The document keeps its tables in its own types; this crate gets them as
//! [`Table`]s with the variables ([`crate::VariableTable::set_tables`]).

use crate::error::ErrorKind;
use crate::units::{Quantity, Unit};

/// Most rows a table holds.
pub const MAX_ROWS: usize = 10_000;
/// Most columns a table holds.
pub const MAX_COLUMNS: usize = 64;

/// One cell: a number in its column's unit, or a short text (a key such
/// as `GT2`), at most [`crate::MAX_TEXT_LEN`] bytes.
#[derive(Clone, Debug, PartialEq)]
pub enum Cell {
    Number(f64),
    Text(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Column {
    pub name: String,
    /// The unit its numbers are in; none for plain numbers.
    pub unit: Option<Unit>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Table {
    pub name: String,
    pub columns: Vec<Column>,
    /// The index in `columns` of the key column.
    pub key: usize,
    /// Each row has one cell per column.
    pub rows: Vec<Vec<Cell>>,
}

/// How a lookup names its row.
#[derive(Clone, Debug, PartialEq)]
pub enum RowRef<'a> {
    /// The number of the row, counted from 1: an expression's value.
    Number(Quantity),
    /// The row's key, written as a text.
    Key(&'a str),
}

/// The text of a cell as a label: a text as it is, a number as Rust
/// writes it (`2`, `2.5`).
pub fn cell_label(cell: &Cell) -> String {
    match cell {
        Cell::Number(value) => format!("{value}"),
        Cell::Text(text) => text.clone(),
    }
}

impl Table {
    /// The labels of the rows: the key column's cells, in order.
    pub fn keys(&self) -> Vec<String> {
        self.rows
            .iter()
            .map(|row| row.get(self.key).map(cell_label).unwrap_or_default())
            .collect()
    }

    /// The value of `column` in the row `row` names.
    pub fn lookup(&self, row: &RowRef<'_>, column: &str) -> Result<Quantity, ErrorKind> {
        let index = match row {
            RowRef::Number(q) => {
                if !q.dimension.is_dimensionless() {
                    return Err(ErrorKind::ArgumentDimension {
                        function: "[ ]",
                        expected: "a row number counted from 1, or a key in quotes",
                        found: q.dimension,
                    });
                }
                let whole = q.value.round();
                if (q.value - whole).abs() > 4.0 * f64::EPSILON * whole.abs().max(1.0) {
                    return Err(ErrorKind::NotWholeNumber {
                        function: "a table row",
                        value: q.value,
                    });
                }
                if whole < 1.0 || whole > self.rows.len() as f64 {
                    return Err(ErrorKind::RowOutOfRange {
                        table: self.name.clone(),
                        index: whole,
                        rows: self.rows.len(),
                    });
                }
                (whole as usize).saturating_sub(1)
            }
            RowRef::Key(key) => self
                .rows
                .iter()
                .position(|r| matches!(r.get(self.key), Some(Cell::Text(t)) if t == key))
                .ok_or_else(|| ErrorKind::UnknownKey {
                    table: self.name.clone(),
                    key: (*key).to_string(),
                })?,
        };
        let Some(c) = self.columns.iter().position(|c| c.name == column) else {
            return Err(ErrorKind::UnknownColumn {
                table: self.name.clone(),
                column: column.to_string(),
            });
        };
        let unit = self.columns.get(c).and_then(|c| c.unit);
        match self.rows.get(index).and_then(|r| r.get(c)) {
            Some(Cell::Number(value)) => Ok(match unit {
                Some(unit) => Quantity::from_unit(*value, unit),
                None => Quantity::dimensionless(*value),
            }),
            Some(Cell::Text(_)) => Err(ErrorKind::TextCell {
                table: self.name.clone(),
                column: column.to_string(),
            }),
            None => Err(ErrorKind::Internal),
        }
    }
}

/// `source` with every reference to the table `old` (a name followed by
/// `[`) replaced by `new`; nothing else changes. Works on source that does
/// not parse.
pub fn rename_table(source: &str, old: &str, new: &str) -> String {
    crate::variables::rename_tokens(source, old, new, crate::variables::NameRole::Table)
        .unwrap_or_else(|| source.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::units::Dimension;

    fn belt() -> Table {
        Table {
            name: "belt".into(),
            columns: vec![
                Column {
                    name: "profile".into(),
                    unit: None,
                },
                Column {
                    name: "pitch".into(),
                    unit: Some(Unit::Millimetre),
                },
                Column {
                    name: "teeth".into(),
                    unit: None,
                },
            ],
            key: 0,
            rows: vec![
                vec![
                    Cell::Text("GT2".into()),
                    Cell::Number(2.0),
                    Cell::Number(20.0),
                ],
                vec![
                    Cell::Text("T5".into()),
                    Cell::Number(0.2),
                    Cell::Text("n/a".into()),
                ],
            ],
        }
    }

    #[test]
    fn a_row_by_number_or_key() {
        let t = belt();
        let one = RowRef::Number(Quantity::dimensionless(1.0));
        assert_eq!(
            t.lookup(&one, "pitch").unwrap(),
            Quantity::new(2.0, Dimension::LENGTH)
        );
        assert_eq!(
            t.lookup(&RowRef::Key("T5"), "pitch").unwrap(),
            Quantity::from_unit(0.2, Unit::Millimetre)
        );
        assert_eq!(
            t.lookup(&one, "teeth").unwrap(),
            Quantity::dimensionless(20.0)
        );
        assert_eq!(t.keys(), ["GT2", "T5"]);
    }
}
