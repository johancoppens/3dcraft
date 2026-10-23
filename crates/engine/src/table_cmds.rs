//! Tables: `table.set`, `table.delete`, `table.rename`, `table.list`
//! (`craft3d_doc::table`). A table is a named grid of values that
//! expressions look up (`belt[profile].pitch`, `belt["GT2"].pitch`) and
//! that a choice parameter can take its labels from (`variable.publish`
//! with `choicesFrom`). Tables are part of the design like variables: a
//! changed table rebuilds what its values reach.

use crate::commands::{Run, add_failed, edit, fail, failing, spec};
use crate::params::Params;
use crate::{CommandError, CommandSpec, Session};
use craft3d_doc::{Document, Table, TableCell, TableColumn};
use serde_json::{Value, json};

pub(crate) static SPECS: &[CommandSpec] = &[
    spec!("table.set", "Set Table", r#"{"name": string, "columns": [column name, or {name, unit: um|mm|cm|m|in|ft|deg|rad or null}, ...], "rows": [[number or string, ...], ...], "key": string = the first column}"#,
        "Defines a table or replaces it whole (one undo step), e.g. {\"name\": \"belt\", \"columns\": [\"profile\", {\"name\": \"pitch\", \"unit\": \"mm\"}], \"rows\": [[\"GT2\", 2], [\"HTD3\", 3]]}. A cell is a number in its column's unit or a short text (printable ASCII without quotes or backslashes, at most 64 characters). key is the column whose cells name the rows: unique and not empty; a choice parameter shows them (variable.publish with choicesFrom). An expression looks a value up as belt[i].pitch, with i the row's number counted from 1 (a variable or a choice parameter), or belt[\"GT2\"].pitch by its key; the value has its column's unit. At most 10000 rows and 64 columns; the name is a variable's kind of name and not a variable's. Returns the table as table.list shows it, and failed (the features that fail now and did not before) as variable.set does.", doc: true, table_set),
    spec!("table.delete", "Delete Table", r#"{"name": string, "force": boolean = false}"#,
        "Deletes a table. A table still used is refused, with every user named (variables, features, assertions, parameter ranges, choice parameters); with force it is deleted anyway and what looked values up in it fails until it is defined again, except a choice parameter's table, which force does not delete (unpublish the parameter or give it other choices first). One undo step.", doc: true, table_delete),
    spec!("table.rename", "Rename Table", r#"{"name": string, "to": string}"#,
        "Renames a table and every lookup in it (variables, features, assertions, parameter ranges) and the choice parameters that take their labels from it. One undo step.", doc: true, table_rename),
    spec!("table.list", "List Tables", r#"{"name": string = every table}"#,
        "Lists the tables of the active document: name, key, columns (name, unit), the number of rows, the keys of the rows and usedBy. With name, that table only, with its rows.", doc: true, table_list),
];

const SET: &str = "table.set";

/// A column given as `"name"` or `{"name", "unit"}`.
fn column(value: &Value) -> Result<TableColumn, CommandError> {
    let bad = || {
        CommandError::new(format!(
            "{SET}: a column is a name or {{\"name\": string, \"unit\": string or null}}"
        ))
    };
    match value {
        Value::String(name) => Ok(TableColumn {
            name: name.clone(),
            unit: None,
        }),
        Value::Object(map) => {
            if let Some(key) = map.keys().find(|k| *k != "name" && *k != "unit") {
                return Err(CommandError::new(format!(
                    "{SET}: a column has no `{key}`; it has name and unit"
                )));
            }
            let name = map.get("name").and_then(Value::as_str).ok_or_else(bad)?;
            let unit = match map.get("unit") {
                None | Some(Value::Null) => None,
                Some(Value::String(unit)) => Some(unit.clone()),
                Some(_) => return Err(bad()),
            };
            Ok(TableColumn {
                name: name.to_string(),
                unit,
            })
        }
        _ => Err(bad()),
    }
}

fn table_set(session: &mut Session, params: &Value) -> Run {
    let p = Params::new(SET, params, &["name", "columns", "rows", "key"])?;
    let name = p.required_str("name")?;
    let Some(Value::Array(given_columns)) = p.get("columns") else {
        return fail(format!("{SET}: `columns` must be a list of columns"));
    };
    if given_columns.len() > craft3d_expr::MAX_COLUMNS {
        return fail(format!(
            "{SET}: a table has at most {} columns, got {}",
            craft3d_expr::MAX_COLUMNS,
            given_columns.len()
        ));
    }
    let columns = given_columns
        .iter()
        .map(column)
        .collect::<Result<Vec<_>, _>>()?;
    let rows: Vec<Vec<TableCell>> = match p.get("rows") {
        None => Vec::new(),
        Some(Value::Array(rows)) => {
            if rows.len() > craft3d_expr::MAX_ROWS {
                return fail(format!(
                    "{SET}: a table has at most {} rows, got {}",
                    craft3d_expr::MAX_ROWS,
                    rows.len()
                ));
            }
            let mut out = Vec::with_capacity(rows.len());
            for (r, row) in rows.iter().enumerate() {
                let Value::Array(cells) = row else {
                    return fail(format!("{SET}: row {} is not a list of cells", r + 1));
                };
                if cells.len() > craft3d_expr::MAX_COLUMNS {
                    return fail(format!(
                        "{SET}: row {} has {} cells, and the table has {} columns",
                        r + 1,
                        cells.len(),
                        columns.len()
                    ));
                }
                let mut parsed = Vec::with_capacity(cells.len());
                for cell in cells {
                    parsed.push(match cell {
                        Value::Number(n) => match n.as_f64().filter(|v| v.is_finite()) {
                            Some(v) => TableCell::Number(v),
                            None => {
                                return fail(format!(
                                    "{SET}: row {}: a number is out of range",
                                    r + 1
                                ));
                            }
                        },
                        Value::String(text) => TableCell::Text(text.clone()),
                        _ => {
                            return fail(format!(
                                "{SET}: row {}: a cell is a number or a text, got {cell}",
                                r + 1
                            ));
                        }
                    });
                }
                out.push(parsed);
            }
            out
        }
        Some(_) => return fail(format!("{SET}: `rows` must be a list of rows")),
    };
    let key = match p.str("key")? {
        Some(key) => key.to_string(),
        None => columns.first().map(|c| c.name.clone()).unwrap_or_default(),
    };
    let table = Table {
        name: name.to_string(),
        key,
        columns,
        rows: std::sync::Arc::new(rows),
    };
    // Checked before the edit, so the message names the command.
    if let Err(reason) = table.check() {
        return fail(format!("{SET}: table `{name}`: {reason}"));
    }
    let existed = session.document()?.history.current().table(name).is_some();
    let before = failing(session)?;
    let doc_before = std::sync::Arc::clone(session.document()?.history.current());
    let mut warnings = edit(session, SET, &format!("Set table {name}"), |doc| {
        doc.set_table(table)
    })?;
    let doc = session.document()?.history.current();
    let mut result = describe(doc, name, false);
    // The choices that lost their key, then every other parameter the
    // change made invalid (a range that looks the table up), as
    // variable.set reports them.
    for warning in crate::param_cmds::invalidated(&doc_before, doc) {
        if !warnings.contains(&warning) {
            warnings.push(warning);
        }
    }
    if let Some(map) = result.as_object_mut() {
        map.insert("created".into(), (!existed).into());
        if !warnings.is_empty() {
            map.insert("parameterWarnings".into(), warnings.into());
        }
    }
    add_failed(session, &before, None, &mut result)?;
    crate::assert_cmds::add_summary(session, &mut result);
    Ok(result)
}

fn table_delete(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "table.delete";
    let p = Params::new(ID, params, &["name", "force"])?;
    let name = p.required_str("name")?;
    let force = p.bool("force")?.unwrap_or(false);
    let doc = session.document()?.history.current();
    if doc.table(name).is_none() {
        return fail(format!(
            "{ID}: there is no table named `{name}`{}",
            known(doc)
        ));
    }
    let users = doc.table_users(name);
    let choosers: Vec<&String> = users
        .iter()
        .filter(|u| u.starts_with("the choices of"))
        .collect();
    if !choosers.is_empty() {
        return fail(format!(
            "{ID}: `{name}` gives {}; unpublish that parameter or give it other choices first",
            choosers
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if !force && !users.is_empty() {
        return fail(format!(
            "{ID}: `{name}` is still used by {}; change those first, or give force: true to delete it anyway (they then fail until it is defined again)",
            users.join(", ")
        ));
    }
    let before = failing(session)?;
    edit(session, ID, &format!("Delete table {name}"), |doc| {
        doc.remove_table(name).map(|_| ())
    })?;
    let mut result = json!({ "deleted": name });
    if !users.is_empty() {
        if let Some(map) = result.as_object_mut() {
            map.insert("usedBy".into(), users.into());
        }
        add_failed(session, &before, None, &mut result)?;
    }
    Ok(result)
}

fn table_rename(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "table.rename";
    let p = Params::new(ID, params, &["name", "to"])?;
    let name = p.required_str("name")?;
    let to = p.required_str("to")?;
    let doc = session.document()?.history.current();
    if doc.table(name).is_none() {
        return fail(format!(
            "{ID}: there is no table named `{name}`{}",
            known(doc)
        ));
    }
    let mut probe = Document::clone(doc);
    if let Err(e) = probe.rename_table(name, to) {
        return fail(format!("{ID}: {e}"));
    }
    edit(
        session,
        ID,
        &format!("Rename table {name} to {to}"),
        |doc| doc.rename_table(name, to),
    )?;
    let doc = session.document()?.history.current();
    Ok(describe(doc, to, false))
}

fn table_list(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "table.list";
    let p = Params::new(ID, params, &["name"])?;
    let doc = session.document()?.history.current();
    match p.str("name")? {
        Some(name) => {
            if doc.table(name).is_none() {
                return fail(format!(
                    "{ID}: there is no table named `{name}`{}",
                    known(doc)
                ));
            }
            Ok(describe(doc, name, true))
        }
        None => {
            let tables: Vec<Value> = doc
                .tables
                .iter()
                .map(|t| describe(doc, &t.name, false))
                .collect();
            Ok(json!({ "tables": tables }))
        }
    }
}

/// ` (the tables are a, b)` or ` (the document has no tables)`.
fn known(doc: &Document) -> String {
    if doc.tables.is_empty() {
        " (the document has no tables)".to_string()
    } else {
        let names: Vec<&str> = doc.tables.iter().map(|t| t.name.as_str()).collect();
        format!(" (the tables are {})", names.join(", "))
    }
}

/// A table as `table.list` shows it; with `rows` its cells too.
fn describe(doc: &Document, name: &str, rows: bool) -> Value {
    let Some(table) = doc.table(name) else {
        return json!({ "table": name });
    };
    let columns: Vec<Value> = table
        .columns
        .iter()
        .map(|c| json!({ "name": c.name, "unit": c.unit }))
        .collect();
    let mut out = json!({
        "table": table.name,
        "key": table.key,
        "columns": columns,
        "rows": table.rows.len(),
        "keys": table.keys(),
        "usedBy": doc.table_users(name),
    });
    if rows && let Some(map) = out.as_object_mut() {
        let cells: Vec<Value> = table
            .rows
            .iter()
            .map(|row| {
                row.iter()
                    .map(|cell| match cell {
                        TableCell::Number(v) => json!(v),
                        TableCell::Text(t) => json!(t),
                    })
                    .collect()
            })
            .collect();
        map.insert("cells".into(), cells.into());
    }
    out
}
