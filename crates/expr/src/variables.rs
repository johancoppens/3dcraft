//! The variable table: named expressions, their dependency order and their
//! values.
//!
//! The table stores source text. An expression that does not parse, refers
//! to a missing name or sits on a cycle is still stored (files and agents
//! produce such tables); it simply has an error instead of a value, and so
//! do the variables that depend on it. Everything else evaluates.

use crate::error::{ErrorKind, ExprError, VariableError};
use crate::lexer::{TokenKind, scan};
use crate::parser::{Expr, Function};
use crate::units::{Quantity, Unit};
use crate::{MAX_NAME_LEN, MAX_SOURCE_LEN, MAX_VARIABLES};
use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap};
use std::sync::Arc;

/// Whether `name` means something else in an expression: a unit, a function
/// or the constant `pi`. Case-sensitive, like the expression language.
pub fn is_reserved_name(name: &str) -> bool {
    name == "pi" || Unit::from_symbol(name).is_some() || Function::from_name(name).is_some()
}

/// Checks that `name` can name a variable: ASCII letters, digits and `_`,
/// not starting with a digit, at most [`MAX_NAME_LEN`] bytes, not reserved.
pub fn check_variable_name(name: &str) -> Result<(), VariableError> {
    let Some(first) = name.chars().next() else {
        return Err(VariableError::EmptyName);
    };
    if name.len() > MAX_NAME_LEN {
        return Err(VariableError::NameTooLong {
            len: name.len(),
            max: MAX_NAME_LEN,
        });
    }
    if first.is_ascii_digit() {
        return Err(VariableError::NameStartsWithDigit);
    }
    if let Some((pos, ch)) = name
        .char_indices()
        .find(|(_, ch)| !(ch.is_ascii_alphanumeric() || *ch == '_'))
    {
        return Err(VariableError::InvalidNameChar { pos, ch });
    }
    if is_reserved_name(name) {
        return Err(VariableError::ReservedName {
            name: name.to_string(),
        });
    }
    Ok(())
}

/// `source` with every reference to the variable `old` replaced by `new`.
/// Nothing else changes: spacing, number formatting and the rest of the
/// text are kept byte for byte. Works on source that does not parse, so a
/// half-typed expression follows a rename too.
///
/// Both names must be valid variable names; that also guarantees that no
/// unit or function name is ever rewritten.
pub fn rename_variable(source: &str, old: &str, new: &str) -> Result<String, VariableError> {
    check_variable_name(old)?;
    check_variable_name(new)?;
    Ok(rename_in(source, old, new).unwrap_or_else(|| source.to_string()))
}

/// As [`rename_variable`], for a variable whose name `old` has since
/// become reserved (a function added to the language, such as `floor`):
/// every name token `old` in `source` is replaced by `new`, also where
/// it is now followed by `(`. Only for reading files written before the
/// name was reserved, where every such token was the variable. `new` must
/// be a valid variable name; `old` must be a reserved one.
pub fn rename_reserved_variable(
    source: &str,
    old: &str,
    new: &str,
) -> Result<String, VariableError> {
    check_variable_name(new)?;
    if !is_reserved_name(old) {
        return Err(VariableError::UnknownVariable {
            name: old.to_string(),
        });
    }
    Ok(rename_in(source, old, new).unwrap_or_else(|| source.to_string()))
}

/// Every name token in `source`, in order, repeats included: variables,
/// functions, units, `pi`. Works on source that does not parse. For
/// finding a name that no expression uses yet.
pub fn names_in(source: &str) -> Vec<&str> {
    let mut names = Vec::new();
    let mut at = 0;
    loop {
        let token = match scan(source, at) {
            Ok(token) => token,
            Err(error) => {
                at = error.resume;
                continue;
            }
        };
        if token.kind == TokenKind::End {
            break;
        }
        at = token.end;
        if token.kind == TokenKind::Ident
            && let Some(name) = source.get(token.pos..token.end)
        {
            names.push(name);
        }
    }
    names
}

/// The renamed text, or `None` when `source` does not mention `old`.
/// `old` must be a valid variable name: then every identifier token equal
/// to it is a reference to that variable.
fn rename_in(source: &str, old: &str, new: &str) -> Option<String> {
    let mut renamed: Option<String> = None;
    // Everything before `copied` is already in `renamed`.
    let mut copied = 0;
    let mut at = 0;
    loop {
        let token = match scan(source, at) {
            Ok(token) => token,
            // An unlexable character: skip it and keep looking.
            Err(error) => {
                at = error.resume;
                continue;
            }
        };
        if token.kind == TokenKind::End {
            break;
        }
        at = token.end;
        if token.kind == TokenKind::Ident && source.get(token.pos..token.end) == Some(old) {
            let out = renamed.get_or_insert_with(String::new);
            out.push_str(source.get(copied..token.pos).unwrap_or(""));
            out.push_str(new);
            copied = token.end;
        }
    }
    let mut out = renamed?;
    out.push_str(source.get(copied..).unwrap_or(""));
    Some(out)
}

/// One row of a [`VariableTable`].
#[derive(Clone, Debug, PartialEq)]
pub struct Variable {
    name: String,
    source: String,
    /// Parsed once when the source is set, so ordering and evaluating the
    /// table do not parse again.
    parsed: Result<Expr, ExprError>,
}

impl Variable {
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The expression as typed. This is what gets saved.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// The parsed expression, or why the source does not parse.
    pub fn parsed(&self) -> Result<&Expr, &ExprError> {
        self.parsed.as_ref()
    }
}

/// An ordered list of `name = expression`. The order is the order of
/// insertion and is part of the document: it decides the evaluation order
/// among variables that do not depend on each other.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VariableTable {
    variables: Vec<Variable>,
}

impl VariableTable {
    pub fn new() -> VariableTable {
        VariableTable::default()
    }

    pub fn len(&self) -> usize {
        self.variables.len()
    }

    pub fn is_empty(&self) -> bool {
        self.variables.is_empty()
    }

    /// The variables in table (insertion) order.
    pub fn variables(&self) -> &[Variable] {
        &self.variables
    }

    pub fn get(&self, name: &str) -> Option<&Variable> {
        self.variables.iter().find(|v| v.name == name)
    }

    pub fn contains(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    /// Sets the expression of `name`. A new variable goes to the end of the
    /// table; an existing one keeps its place.
    ///
    /// Only the name and the length of `source` are checked here. A source
    /// that does not parse or refers to unknown names is accepted and shows
    /// up as an error in [`VariableTable::evaluate`].
    pub fn set(&mut self, name: &str, source: &str) -> Result<(), VariableError> {
        check_variable_name(name)?;
        if source.len() > MAX_SOURCE_LEN {
            return Err(VariableError::SourceTooLong {
                variable: name.to_string(),
                len: source.len(),
                max: MAX_SOURCE_LEN,
            });
        }
        let parsed = Expr::parse(source);
        if let Some(existing) = self.variables.iter_mut().find(|v| v.name == name) {
            existing.source = source.to_string();
            existing.parsed = parsed;
            return Ok(());
        }
        if self.variables.len() >= MAX_VARIABLES {
            return Err(VariableError::TooManyVariables { max: MAX_VARIABLES });
        }
        self.variables.push(Variable {
            name: name.to_string(),
            source: source.to_string(),
            parsed,
        });
        Ok(())
    }

    /// Deletes `name`. Variables that refer to it are left alone and get an
    /// "unknown variable" error when evaluated.
    pub fn remove(&mut self, name: &str) -> Result<(), VariableError> {
        let Some(index) = self.variables.iter().position(|v| v.name == name) else {
            return Err(VariableError::UnknownVariable {
                name: name.to_string(),
            });
        };
        self.variables.remove(index);
        Ok(())
    }

    /// Renames `old` to `new` and rewrites every reference to it in the
    /// other expressions (see [`rename_variable`]). The variable keeps its
    /// place. All or nothing: on an error the table is unchanged.
    pub fn rename(&mut self, old: &str, new: &str) -> Result<(), VariableError> {
        check_variable_name(new)?;
        let Some(index) = self.variables.iter().position(|v| v.name == old) else {
            return Err(VariableError::UnknownVariable {
                name: old.to_string(),
            });
        };
        if old == new {
            return Ok(());
        }
        if self.contains(new) {
            return Err(VariableError::DuplicateName {
                name: new.to_string(),
            });
        }
        // Work out every new source before touching the table.
        let mut updates: Vec<(usize, String)> = Vec::new();
        for (i, variable) in self.variables.iter().enumerate() {
            let Some(source) = rename_in(&variable.source, old, new) else {
                continue;
            };
            if source.len() > MAX_SOURCE_LEN {
                return Err(VariableError::SourceTooLong {
                    variable: variable.name.clone(),
                    len: source.len(),
                    max: MAX_SOURCE_LEN,
                });
            }
            updates.push((i, source));
        }
        for (i, source) in updates {
            if let Some(variable) = self.variables.get_mut(i) {
                variable.parsed = Expr::parse(&source);
                variable.source = source;
            }
        }
        if let Some(variable) = self.variables.get_mut(index) {
            variable.name = new.to_string();
        }
        Ok(())
    }

    /// The names in evaluation order: every variable comes after the
    /// variables it refers to, and at each step the earliest variable in
    /// the table whose dependencies are done is taken. A table that is
    /// already in dependency order therefore evaluates in table order.
    ///
    /// Variables on a cycle, and those that depend on one, cannot be
    /// ordered; they come last, in table order.
    pub fn evaluation_order(&self) -> Vec<&str> {
        let plan = self.plan();
        plan.order
            .iter()
            .filter_map(|&i| self.variables.get(i))
            .map(|v| v.name.as_str())
            .collect()
    }

    /// Evaluates every variable. A variable with an error (it does not
    /// parse, refers to an unknown name, mixes dimensions, is on a cycle,
    /// …) gets that error, the variables depending on it get
    /// [`ErrorKind::DependencyFailed`], and all others get their value.
    pub fn evaluate(&self) -> Evaluation {
        let plan = self.plan();
        // One shared name list per cycle, not one copy per member.
        let cycle_names: Vec<Arc<[String]>> = plan
            .cycles
            .iter()
            .map(|members| {
                members
                    .iter()
                    .filter_map(|&m| self.variables.get(m))
                    .map(|v| v.name.clone())
                    .collect()
            })
            .collect();
        let mut results: Vec<Option<Result<Quantity, ExprError>>> =
            vec![None; self.variables.len()];
        let mut entries = Vec::with_capacity(self.variables.len());
        for &i in &plan.order {
            let Some(variable) = self.variables.get(i) else {
                continue;
            };
            let result = self.evaluate_one(i, variable, &plan, &cycle_names, &results);
            entries.push(EvaluatedVariable {
                name: variable.name.clone(),
                result: result.clone(),
            });
            if let Some(slot) = results.get_mut(i) {
                *slot = Some(result);
            }
        }
        let index = entries
            .iter()
            .enumerate()
            .map(|(i, entry)| (entry.name.clone(), i))
            .collect();
        Evaluation { entries, index }
    }

    /// The result of variable `i`, given the results of everything before
    /// it in the plan's order.
    fn evaluate_one(
        &self,
        i: usize,
        variable: &Variable,
        plan: &Plan<'_>,
        cycle_names: &[Arc<[String]>],
        results: &[Option<Result<Quantity, ExprError>>],
    ) -> Result<Quantity, ExprError> {
        let expr = variable.parsed.as_ref().map_err(Clone::clone)?;
        let cycle = plan.cycle_of.get(i).copied().flatten();
        if let Some(members) = cycle.and_then(|id| plan.cycles.get(id)) {
            // Point at the first reference that leads back into the cycle.
            let pos = expr
                .variable_refs()
                .find(|(name, _)| {
                    plan.index
                        .get(name)
                        .is_some_and(|j| members.binary_search(j).is_ok())
                })
                .map_or(0, |(_, pos)| pos);
            let names = cycle
                .and_then(|id| cycle_names.get(id))
                .cloned()
                .unwrap_or_else(|| Arc::from(Vec::new()));
            return Err(ExprError::new(pos, ErrorKind::Cycle { names }));
        }
        evaluate_known(
            expr,
            |name| plan.index.contains_key(name),
            |name| {
                let j = *plan.index.get(name)?;
                match results.get(j) {
                    Some(Some(Ok(value))) => Some(*value),
                    _ => None,
                }
            },
        )
    }

    /// Dependency graph, evaluation order and cycles. Everything here is
    /// indexed by table position and iterated in a fixed order: no hash map
    /// order can leak into the result.
    fn plan(&self) -> Plan<'_> {
        let count = self.variables.len();
        let index: BTreeMap<&str, usize> = self
            .variables
            .iter()
            .enumerate()
            .map(|(i, v)| (v.name.as_str(), i))
            .collect();
        // deps[i]: the table positions variable i refers to, sorted, each
        // once. Unknown names are no edge; they fail at evaluation.
        let deps: Vec<Vec<usize>> = self
            .variables
            .iter()
            .map(|variable| {
                let Ok(expr) = &variable.parsed else {
                    return Vec::new();
                };
                let mut targets: Vec<usize> = expr
                    .variable_refs()
                    .filter_map(|(name, _)| index.get(name).copied())
                    .collect();
                targets.sort_unstable();
                targets.dedup();
                targets
            })
            .collect();

        // Kahn's algorithm with "smallest table position first" as the
        // tie-break, which makes the order unique.
        let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); count];
        for (i, targets) in deps.iter().enumerate() {
            for &target in targets {
                if let Some(list) = dependents.get_mut(target) {
                    list.push(i);
                }
            }
        }
        let mut waiting: Vec<usize> = deps.iter().map(Vec::len).collect();
        let mut ready: BinaryHeap<Reverse<usize>> = waiting
            .iter()
            .enumerate()
            .filter(|&(_, &pending)| pending == 0)
            .map(|(i, _)| Reverse(i))
            .collect();
        let mut order = Vec::with_capacity(count);
        let mut stuck = vec![true; count];
        while let Some(Reverse(i)) = ready.pop() {
            order.push(i);
            if let Some(flag) = stuck.get_mut(i) {
                *flag = false;
            }
            for &dependent in dependents.get(i).into_iter().flatten() {
                if let Some(pending) = waiting.get_mut(dependent) {
                    *pending = pending.saturating_sub(1);
                    if *pending == 0 {
                        ready.push(Reverse(dependent));
                    }
                }
            }
        }
        // What is left is on a cycle or behind one.
        order.extend((0..count).filter(|&i| at(&stuck, i, false)));

        let (cycle_of, cycles) = find_cycles(&deps, &stuck);
        Plan {
            index,
            order,
            cycle_of,
            cycles,
        }
    }
}

struct Plan<'a> {
    /// Name → table position.
    index: BTreeMap<&'a str, usize>,
    /// Table positions in evaluation order; all of them.
    order: Vec<usize>,
    /// For each variable, the entry of `cycles` it is on, if any.
    cycle_of: Vec<Option<usize>>,
    /// Each cycle as sorted table positions.
    cycles: Vec<Vec<usize>>,
}

/// `values[i]`, or `fallback` when out of range.
fn at<T: Copy>(values: &[T], i: usize, fallback: T) -> T {
    values.get(i).copied().unwrap_or(fallback)
}

fn put<T>(values: &mut [T], i: usize, value: T) {
    if let Some(slot) = values.get_mut(i) {
        *slot = value;
    }
}

/// Strongly connected components among the `stuck` variables (Tarjan,
/// 1972), written with an explicit stack because a dependency chain can be
/// as long as the table. A component is a cycle when it has more than one
/// member or its one member refers to itself. Variables that merely depend
/// on a cycle are components of their own and are not reported.
fn find_cycles(deps: &[Vec<usize>], stuck: &[bool]) -> (Vec<Option<usize>>, Vec<Vec<usize>>) {
    const UNVISITED: usize = usize::MAX;
    let count = deps.len();
    let mut number = vec![UNVISITED; count];
    let mut low = vec![0usize; count];
    let mut on_stack = vec![false; count];
    let mut stack: Vec<usize> = Vec::new();
    // (variable, how many of its dependencies have been looked at)
    let mut work: Vec<(usize, usize)> = Vec::new();
    let mut next_number = 0usize;
    let mut cycle_of = vec![None; count];
    let mut cycles: Vec<Vec<usize>> = Vec::new();

    for root in 0..count {
        if !at(stuck, root, false) || at(&number, root, UNVISITED) != UNVISITED {
            continue;
        }
        work.push((root, 0));
        while let Some(&(v, looked_at)) = work.last() {
            if looked_at == 0 && at(&number, v, UNVISITED) == UNVISITED {
                put(&mut number, v, next_number);
                put(&mut low, v, next_number);
                next_number += 1;
                stack.push(v);
                put(&mut on_stack, v, true);
            }
            let dependency = deps.get(v).and_then(|targets| targets.get(looked_at));
            if let Some(&w) = dependency {
                if let Some(top) = work.last_mut() {
                    top.1 += 1;
                }
                if !at(stuck, w, false) {
                    continue;
                }
                if at(&number, w, UNVISITED) == UNVISITED {
                    work.push((w, 0));
                } else if at(&on_stack, w, false) {
                    let lowest = at(&low, v, 0).min(at(&number, w, 0));
                    put(&mut low, v, lowest);
                }
                continue;
            }
            // All dependencies of v are done.
            work.pop();
            if let Some(&(parent, _)) = work.last() {
                let lowest = at(&low, parent, 0).min(at(&low, v, 0));
                put(&mut low, parent, lowest);
            }
            if at(&low, v, 0) != at(&number, v, UNVISITED) {
                continue;
            }
            let mut component = Vec::new();
            while let Some(w) = stack.pop() {
                put(&mut on_stack, w, false);
                component.push(w);
                if w == v {
                    break;
                }
            }
            let refers_to_itself = deps
                .get(v)
                .is_some_and(|targets| targets.binary_search(&v).is_ok());
            if component.len() > 1 || refers_to_itself {
                component.sort_unstable();
                for &member in &component {
                    put(&mut cycle_of, member, Some(cycles.len()));
                }
                cycles.push(component);
            }
        }
    }
    (cycle_of, cycles)
}

/// One variable's outcome in an [`Evaluation`].
#[derive(Clone, Debug, PartialEq)]
pub struct EvaluatedVariable {
    pub name: String,
    pub result: Result<Quantity, ExprError>,
}

/// The outcome of [`VariableTable::evaluate`].
#[derive(Clone, Debug, PartialEq)]
pub struct Evaluation {
    entries: Vec<EvaluatedVariable>,
    index: BTreeMap<String, usize>,
}

/// Evaluates `expr` against a set of variables: `known` says whether a
/// name is one of them, `value` gives its value when it has one.
///
/// A name that is not a variable at all is an error even where it would
/// not be used (in the branch of `if` that is not taken), so a typo does
/// not wait for the condition to flip. A variable without a value (it
/// failed) only counts where it is used, and is then reported as
/// [`ErrorKind::DependencyFailed`]; every other error is the expression's
/// own.
fn evaluate_known(
    expr: &Expr,
    known: impl Fn(&str) -> bool,
    value: impl FnMut(&str) -> Option<Quantity>,
) -> Result<Quantity, ExprError> {
    if let Some((name, pos)) = expr.variable_refs().find(|(name, _)| !known(name)) {
        return Err(ExprError::new(
            pos,
            ErrorKind::UnknownVariable {
                name: name.to_string(),
            },
        ));
    }
    expr.eval(value).map_err(|error| match error.kind {
        ErrorKind::UnknownVariable { name } if known(&name) => {
            ExprError::new(error.pos, ErrorKind::DependencyFailed { name })
        }
        kind => ExprError::new(error.pos, kind),
    })
}

impl Evaluation {
    /// Parses and evaluates `source` with these variables, as a variable
    /// of the table is: a name that is no variable is an error wherever it
    /// stands (also in a branch of `if` that is not taken), a variable that
    /// failed is [`ErrorKind::DependencyFailed`] where it is used. Use this,
    /// not `evaluate(source, |n| evaluation.value(n))`, for parameters.
    pub fn evaluate(&self, source: &str) -> Result<Quantity, ExprError> {
        let expr = Expr::parse(source)?;
        evaluate_known(
            &expr,
            |name| self.index.contains_key(name),
            |name| self.value(name),
        )
    }

    /// Evaluates a parsed expression as [`Evaluation::evaluate`] does, with
    /// one more name bound to `value`: the free parameter of a curve
    /// (`t`), which shadows a variable of the same name inside `expr`.
    /// Parse once and call this for every value of the parameter.
    pub fn evaluate_bound(
        &self,
        expr: &Expr,
        name: &str,
        value: Quantity,
    ) -> Result<Quantity, ExprError> {
        evaluate_known(
            expr,
            |known| known == name || self.index.contains_key(known),
            |wanted| {
                if wanted == name {
                    Some(value)
                } else {
                    self.value(wanted)
                }
            },
        )
    }

    /// Every variable of the table, in evaluation order
    /// ([`VariableTable::evaluation_order`]).
    pub fn entries(&self) -> &[EvaluatedVariable] {
        &self.entries
    }

    /// The value or error of `name`; `None` when the table has no such
    /// variable.
    pub fn get(&self, name: &str) -> Option<&Result<Quantity, ExprError>> {
        let entry = self.entries.get(*self.index.get(name)?)?;
        Some(&entry.result)
    }

    /// The value of `name`, when it has one. Fits the `lookup` argument of
    /// [`Expr::eval`]: `expr.eval(|name| evaluation.value(name))`.
    pub fn value(&self, name: &str) -> Option<Quantity> {
        self.get(name)?.as_ref().ok().copied()
    }

    /// Whether every variable has a value.
    pub fn is_ok(&self) -> bool {
        self.entries.iter().all(|entry| entry.result.is_ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::units::Dimension;

    fn table(rows: &[(&str, &str)]) -> VariableTable {
        let mut table = VariableTable::new();
        for (name, source) in rows {
            table.set(name, source).unwrap();
        }
        table
    }

    #[test]
    fn a_bound_parameter_shadows_a_variable_of_its_name() {
        let values = table(&[("rb", "20 mm"), ("t", "3 mm"), ("bad", "1 mm / 0")]).evaluate();
        let x = Expr::parse("rb * (cos(t) + t * sin(t))").unwrap();
        let got = values
            .evaluate_bound(&x, "t", Quantity::dimensionless(0.5))
            .unwrap();
        assert_eq!(got.dimension, Dimension::LENGTH);
        assert!((got.value - 20.0 * (0.5f64.cos() + 0.5 * 0.5f64.sin())).abs() < 1e-12);
        // Without a variable of its name the parameter is still known.
        let plain = table(&[("rb", "20 mm")]).evaluate();
        assert!(
            plain
                .evaluate_bound(&x, "t", Quantity::dimensionless(1.0))
                .is_ok()
        );
        // A typo is still an error, and a failed variable is reported as one.
        let typo = Expr::parse("rb * tt").unwrap();
        assert!(matches!(
            values
                .evaluate_bound(&typo, "t", Quantity::dimensionless(1.0))
                .unwrap_err()
                .kind,
            ErrorKind::UnknownVariable { .. }
        ));
        let failed = Expr::parse("bad * t").unwrap();
        assert!(matches!(
            values
                .evaluate_bound(&failed, "t", Quantity::dimensionless(1.0))
                .unwrap_err()
                .kind,
            ErrorKind::DependencyFailed { .. }
        ));
    }

    fn sources(table: &VariableTable) -> Vec<(&str, &str)> {
        table
            .variables()
            .iter()
            .map(|v| (v.name(), v.source()))
            .collect()
    }

    fn length(value: f64) -> Result<Quantity, ExprError> {
        Ok(Quantity::new(value, Dimension::LENGTH))
    }

    fn kind(evaluation: &Evaluation, name: &str) -> ErrorKind {
        evaluation.get(name).unwrap().clone().unwrap_err().kind
    }

    fn cycle(names: &[&str]) -> ErrorKind {
        ErrorKind::Cycle {
            names: names.iter().map(|n| n.to_string()).collect(),
        }
    }

    fn dependency_failed(name: &str) -> ErrorKind {
        ErrorKind::DependencyFailed {
            name: name.to_string(),
        }
    }

    // --- names ---

    #[test]
    fn valid_names() {
        for name in [
            "a", "width", "_", "_1", "a1_b2", "Width", "MM", "PI", "mm2", "e",
        ] {
            assert_eq!(check_variable_name(name), Ok(()), "{name}");
        }
        assert_eq!(check_variable_name(&"a".repeat(MAX_NAME_LEN)), Ok(()));
    }

    #[test]
    fn invalid_names() {
        assert_eq!(check_variable_name(""), Err(VariableError::EmptyName));
        assert_eq!(
            check_variable_name("1a"),
            Err(VariableError::NameStartsWithDigit)
        );
        assert_eq!(
            check_variable_name("a b"),
            Err(VariableError::InvalidNameChar { pos: 1, ch: ' ' })
        );
        assert_eq!(
            check_variable_name("a-b"),
            Err(VariableError::InvalidNameChar { pos: 1, ch: '-' })
        );
        assert_eq!(
            check_variable_name("hoogté"),
            Err(VariableError::InvalidNameChar { pos: 5, ch: 'é' })
        );
        assert_eq!(
            check_variable_name("é"),
            Err(VariableError::InvalidNameChar { pos: 0, ch: 'é' })
        );
        assert_eq!(
            check_variable_name(&"a".repeat(MAX_NAME_LEN + 1)),
            Err(VariableError::NameTooLong {
                len: MAX_NAME_LEN + 1,
                max: MAX_NAME_LEN
            })
        );
    }

    #[test]
    fn reserved_names() {
        let mut reserved = vec!["pi"];
        reserved.extend(Unit::ALL.iter().map(|u| u.symbol()));
        reserved.extend(Function::ALL.iter().map(|f| f.name()));
        assert_eq!(reserved.len(), 1 + 8 + 26);
        for name in reserved {
            assert!(is_reserved_name(name));
            assert_eq!(
                check_variable_name(name),
                Err(VariableError::ReservedName {
                    name: name.to_string()
                }),
                "{name}"
            );
        }
        let mut t = VariableTable::new();
        assert!(matches!(
            t.set("mm", "1"),
            Err(VariableError::ReservedName { .. })
        ));
        assert!(t.is_empty());
    }

    // --- editing ---

    #[test]
    fn set_appends_and_updates_in_place() {
        let mut t = table(&[("a", "1 mm"), ("b", "a * 2")]);
        t.set("a", "5 mm").unwrap();
        t.set("c", "3").unwrap();
        assert_eq!(sources(&t), [("a", "5 mm"), ("b", "a * 2"), ("c", "3")]);
        assert_eq!(t.len(), 3);
        assert_eq!(t.get("b").unwrap().source(), "a * 2");
        assert!(t.get("b").unwrap().parsed().is_ok());
        assert!(t.get("zz").is_none());
        assert!(t.contains("c") && !t.contains("C"));
    }

    #[test]
    fn set_rejects_bad_names_and_long_sources() {
        let mut t = VariableTable::new();
        assert_eq!(t.set("", "1"), Err(VariableError::EmptyName));
        assert_eq!(t.set("2x", "1"), Err(VariableError::NameStartsWithDigit));
        let long = "1".repeat(MAX_SOURCE_LEN + 1);
        assert_eq!(
            t.set("a", &long),
            Err(VariableError::SourceTooLong {
                variable: "a".into(),
                len: MAX_SOURCE_LEN + 1,
                max: MAX_SOURCE_LEN
            })
        );
        assert!(t.is_empty());
    }

    #[test]
    fn set_accepts_a_source_that_does_not_parse() {
        let t = table(&[("a", "2 +")]);
        assert_eq!(t.get("a").unwrap().source(), "2 +");
        assert_eq!(t.get("a").unwrap().parsed().unwrap_err().pos, 3);
    }

    #[test]
    fn table_size_is_capped() {
        let mut t = VariableTable::new();
        for i in 0..MAX_VARIABLES {
            t.set(&format!("v{i}"), "1").unwrap();
        }
        assert_eq!(
            t.set("one_more", "1"),
            Err(VariableError::TooManyVariables { max: MAX_VARIABLES })
        );
        // Updating an existing one is still fine.
        t.set("v0", "2").unwrap();
        assert_eq!(t.len(), MAX_VARIABLES);
    }

    #[test]
    fn remove_deletes_and_leaves_references_dangling() {
        let mut t = table(&[("a", "1 mm"), ("b", "a * 2"), ("c", "3 mm")]);
        t.remove("a").unwrap();
        assert_eq!(sources(&t), [("b", "a * 2"), ("c", "3 mm")]);
        assert_eq!(
            t.remove("a"),
            Err(VariableError::UnknownVariable { name: "a".into() })
        );
        let e = t.evaluate();
        assert_eq!(
            e.get("b"),
            Some(&Err(ExprError::new(
                0,
                ErrorKind::UnknownVariable { name: "a".into() }
            )))
        );
        assert_eq!(e.get("c"), Some(&length(3.0)));
        assert_eq!(e.get("a"), None);
    }

    // --- order ---

    #[test]
    fn independent_variables_keep_table_order() {
        let t = table(&[("c", "1"), ("a", "2"), ("b", "3")]);
        assert_eq!(t.evaluation_order(), ["c", "a", "b"]);
    }

    #[test]
    fn a_table_in_dependency_order_keeps_its_order() {
        let t = table(&[
            ("width", "40 mm"),
            ("wall", "2 mm"),
            ("inner", "width - 2 * wall"),
            ("height", "30 mm"),
            ("volume", "inner * inner * height"),
        ]);
        assert_eq!(
            t.evaluation_order(),
            ["width", "wall", "inner", "height", "volume"]
        );
    }

    #[test]
    fn dependencies_come_first() {
        let t = table(&[("a", "b + c"), ("b", "c * 2"), ("c", "1 mm"), ("d", "5")]);
        assert_eq!(t.evaluation_order(), ["c", "b", "a", "d"]);
        let t = table(&[("a", "c + 1"), ("b", "1"), ("c", "2")]);
        // Earliest ready variable first: b is ready before a.
        assert_eq!(t.evaluation_order(), ["b", "c", "a"]);
    }

    #[test]
    fn diamond() {
        let t = table(&[
            ("top", "left + right"),
            ("left", "base * 2"),
            ("right", "base * 3"),
            ("base", "1 mm"),
        ]);
        assert_eq!(t.evaluation_order(), ["base", "left", "right", "top"]);
        assert_eq!(t.evaluate().get("top"), Some(&length(5.0)));
    }

    #[test]
    fn order_is_stable_across_calls_and_clones() {
        let rows: Vec<(String, String)> = (0..200)
            .map(|i| {
                // Each variable refers to two pseudo-random later ones.
                let a = (i * 7 + 3) % 200;
                let b = (i * 13 + 5) % 200;
                let source = match (a > i, b > i) {
                    (true, true) => format!("v{a} + v{b}"),
                    (true, false) => format!("v{a} + 1"),
                    (false, true) => format!("v{b} + 1"),
                    (false, false) => "1".to_string(),
                };
                (format!("v{i}"), source)
            })
            .collect();
        let mut t = VariableTable::new();
        for (name, source) in &rows {
            t.set(name, source).unwrap();
        }
        let order = t.evaluation_order();
        assert_eq!(order.len(), 200);
        for _ in 0..5 {
            assert_eq!(t.clone().evaluation_order(), order);
        }
        // It is a topological order …
        let position = |name: &str| order.iter().position(|n| *n == name).unwrap();
        for v in t.variables() {
            for dep in v.parsed().unwrap().variables() {
                assert!(
                    position(dep) < position(v.name()),
                    "{dep} before {}",
                    v.name()
                );
            }
        }
        // … and everything evaluates.
        assert!(t.evaluate().is_ok());
    }

    #[test]
    fn evaluation_lists_variables_in_evaluation_order() {
        let t = table(&[("a", "b + 1 mm"), ("b", "2 mm")]);
        let e = t.evaluate();
        let names: Vec<&str> = e
            .entries()
            .iter()
            .map(|entry| entry.name.as_str())
            .collect();
        assert_eq!(names, t.evaluation_order());
        assert_eq!(names, ["b", "a"]);
        assert_eq!(e.value("a"), Some(Quantity::new(3.0, Dimension::LENGTH)));
        assert!(e.is_ok());
    }

    #[test]
    fn empty_table() {
        let t = VariableTable::new();
        assert!(t.evaluation_order().is_empty());
        assert!(t.evaluate().entries().is_empty());
        assert!(t.evaluate().is_ok());
    }

    // --- cycles ---

    #[test]
    fn self_reference() {
        let t = table(&[("a", "1 mm"), ("b", "2 * b + a"), ("c", "a * 2")]);
        let e = t.evaluate();
        assert_eq!(e.get("b"), Some(&Err(ExprError::new(4, cycle(&["b"])))));
        assert_eq!(e.get("a"), Some(&length(1.0)));
        assert_eq!(e.get("c"), Some(&length(2.0)));
        assert_eq!(t.evaluation_order(), ["a", "c", "b"]);
        assert_eq!(
            e.get("b").unwrap().clone().unwrap_err().to_string(),
            "at byte 4: circular definition: `b` refers to itself"
        );
    }

    #[test]
    fn two_cycle() {
        let t = table(&[("a", "b + 1"), ("ok", "7"), ("b", "2 * a")]);
        let e = t.evaluate();
        assert_eq!(
            e.get("a"),
            Some(&Err(ExprError::new(0, cycle(&["a", "b"]))))
        );
        assert_eq!(
            e.get("b"),
            Some(&Err(ExprError::new(4, cycle(&["a", "b"]))))
        );
        assert_eq!(e.value("ok"), Some(Quantity::dimensionless(7.0)));
        assert_eq!(t.evaluation_order(), ["ok", "a", "b"]);
        assert_eq!(
            kind(&e, "a").to_string(),
            "circular definition: `a`, `b` depend on each other"
        );
    }

    #[test]
    fn longer_cycle_with_a_tail() {
        let t = table(&[
            ("free", "1 mm"),
            ("a", "b"),
            ("b", "c + free"),
            ("c", "d"),
            ("d", "a"),
            ("after", "a + 1 mm"),
            ("after2", "after * 2"),
            ("also_free", "free * 3"),
        ]);
        let e = t.evaluate();
        for name in ["a", "b", "c", "d"] {
            assert_eq!(kind(&e, name), cycle(&["a", "b", "c", "d"]), "{name}");
        }
        // Behind the cycle, not on it.
        assert_eq!(
            e.get("after"),
            Some(&Err(ExprError::new(0, dependency_failed("a"))))
        );
        assert_eq!(kind(&e, "after2"), dependency_failed("after"));
        assert_eq!(e.get("free"), Some(&length(1.0)));
        assert_eq!(e.get("also_free"), Some(&length(3.0)));
        assert_eq!(
            t.evaluation_order(),
            ["free", "also_free", "a", "b", "c", "d", "after", "after2"]
        );
    }

    #[test]
    fn two_separate_cycles_and_a_bridge() {
        // bridge depends on the first cycle and is depended on by the
        // second: it is on neither.
        let t = table(&[
            ("a", "b"),
            ("b", "a"),
            ("bridge", "a + 1"),
            ("x", "y + bridge"),
            ("y", "x"),
        ]);
        let e = t.evaluate();
        assert_eq!(kind(&e, "a"), cycle(&["a", "b"]));
        assert_eq!(kind(&e, "b"), cycle(&["a", "b"]));
        assert_eq!(kind(&e, "bridge"), dependency_failed("a"));
        assert_eq!(kind(&e, "x"), cycle(&["x", "y"]));
        assert_eq!(kind(&e, "y"), cycle(&["x", "y"]));
    }

    #[test]
    fn overlapping_cycles_are_reported_together() {
        let t = table(&[("a", "b + c"), ("b", "a"), ("c", "a")]);
        let e = t.evaluate();
        for name in ["a", "b", "c"] {
            assert_eq!(kind(&e, name), cycle(&["a", "b", "c"]), "{name}");
        }
    }

    #[test]
    fn breaking_a_cycle_heals_the_table() {
        let mut t = table(&[("a", "b + 1"), ("b", "a")]);
        assert!(!t.evaluate().is_ok());
        t.set("b", "41").unwrap();
        assert_eq!(t.evaluate().value("a"), Some(Quantity::dimensionless(42.0)));
    }

    #[test]
    fn a_long_chain_is_not_a_recursion_problem() {
        // Each variable depends on the next: the deepest possible graph,
        // stored in the worst order. Then the same as one big cycle.
        let mut t = VariableTable::new();
        let n = MAX_VARIABLES;
        for i in 0..n - 1 {
            t.set(&format!("v{i}"), &format!("v{} + 1", i + 1)).unwrap();
        }
        t.set(&format!("v{}", n - 1), "0").unwrap();
        let e = t.evaluate();
        assert_eq!(e.value("v0"), Some(Quantity::dimensionless((n - 1) as f64)));
        assert_eq!(
            t.evaluation_order().first(),
            Some(&format!("v{}", n - 1).as_str())
        );

        t.set(&format!("v{}", n - 1), "v0").unwrap();
        let e = t.evaluate();
        let ErrorKind::Cycle { names } = kind(&e, "v17") else {
            panic!("expected a cycle");
        };
        assert_eq!(names.len(), n);
    }

    // --- partial failure ---

    #[test]
    fn an_error_fails_only_its_dependents() {
        let t = table(&[
            ("width", "40 mm"),
            ("broken", "2 +"),
            ("uses_broken", "broken * 2"),
            ("deeper", "1 mm + uses_broken"),
            ("unknown", "width + nope"),
            ("mixed", "width + 30 deg"),
            ("uses_mixed", "mixed / 2"),
            ("zero_div", "width / 0"),
            ("fine", "width / 2"),
        ]);
        let e = t.evaluate();
        assert_eq!(e.get("width"), Some(&length(40.0)));
        assert_eq!(e.get("fine"), Some(&length(20.0)));
        assert!(matches!(kind(&e, "broken"), ErrorKind::Expected { .. }));
        assert_eq!(e.get("broken").unwrap().clone().unwrap_err().pos, 3);
        assert_eq!(
            e.get("uses_broken"),
            Some(&Err(ExprError::new(0, dependency_failed("broken"))))
        );
        assert_eq!(
            e.get("deeper"),
            Some(&Err(ExprError::new(7, dependency_failed("uses_broken"))))
        );
        assert_eq!(
            e.get("unknown"),
            Some(&Err(ExprError::new(
                8,
                ErrorKind::UnknownVariable {
                    name: "nope".into()
                }
            )))
        );
        assert!(matches!(
            kind(&e, "mixed"),
            ErrorKind::DimensionMismatch { .. }
        ));
        assert_eq!(kind(&e, "uses_mixed"), dependency_failed("mixed"));
        assert_eq!(kind(&e, "zero_div"), ErrorKind::DivisionByZero);
        assert!(!e.is_ok());
        assert_eq!(e.value("broken"), None);
        assert_eq!(
            e.get("uses_broken")
                .unwrap()
                .clone()
                .unwrap_err()
                .to_string(),
            "at byte 0: depends on `broken`, which has an error"
        );
        // Failed variables still have their place in the order.
        assert_eq!(t.evaluation_order().len(), 9);
    }

    #[test]
    fn fixing_the_error_heals_the_dependents() {
        let mut t = table(&[("a", "2 +"), ("b", "a * 2")]);
        assert!(t.evaluate().value("b").is_none());
        t.set("a", "2 + 1").unwrap();
        assert_eq!(t.evaluate().value("b"), Some(Quantity::dimensionless(6.0)));
    }

    // --- rename ---

    #[test]
    fn rename_updates_references_and_nothing_else() {
        let mut t = table(&[
            ("w", "40 mm"),
            ("a", "  w/2 +  w *sin( 30deg )-w2 "),
            ("w2", "max(w , 1.50e1 mm,w)"),
            ("other", "3"),
        ]);
        let before = t.evaluate();
        t.rename("w", "width").unwrap();
        assert_eq!(
            sources(&t),
            [
                ("width", "40 mm"),
                ("a", "  width/2 +  width *sin( 30deg )-w2 "),
                ("w2", "max(width , 1.50e1 mm,width)"),
                ("other", "3"),
            ]
        );
        let after = t.evaluate();
        assert_eq!(after.value("a"), before.value("a"));
        assert_eq!(after.value("width"), before.value("w"));
        assert!(after.is_ok());
    }

    #[test]
    fn rename_does_not_touch_look_alikes() {
        // `m` is a unit, `m1` and `am` are other names, `2e3` is a number.
        let source = "a + a1 + _a + aa + 2e3 + 3 m + A + sin(a)";
        assert_eq!(
            rename_variable(source, "a", "b").unwrap(),
            "b + a1 + _a + aa + 2e3 + 3 m + A + sin(b)"
        );
        assert_eq!(rename_variable("2e3 + e3", "e3", "x").unwrap(), "2e3 + x");
        assert_eq!(rename_variable("1 + 2", "a", "b").unwrap(), "1 + 2");
        assert_eq!(rename_variable("", "a", "b").unwrap(), "");
    }

    #[test]
    fn rename_works_on_source_that_does_not_parse() {
        assert_eq!(rename_variable("(a + ", "a", "b").unwrap(), "(b + ");
        assert_eq!(
            rename_variable("a $ a é a", "a", "bcd").unwrap(),
            "bcd $ bcd é bcd"
        );
        assert_eq!(rename_variable("1e999 * a", "a", "b").unwrap(), "1e999 * b");
        let mut t = table(&[("a", "1"), ("b", "a + (")]);
        t.rename("a", "x").unwrap();
        assert_eq!(sources(&t), [("x", "1"), ("b", "x + (")]);
    }

    #[test]
    fn rename_variable_rejects_reserved_and_invalid_names() {
        assert!(matches!(
            rename_variable("2 mm", "mm", "x"),
            Err(VariableError::ReservedName { .. })
        ));
        assert!(matches!(
            rename_variable("a", "a", "sin"),
            Err(VariableError::ReservedName { .. })
        ));
        assert_eq!(
            rename_variable("a", "a", "b c"),
            Err(VariableError::InvalidNameChar { pos: 1, ch: ' ' })
        );
    }

    #[test]
    fn rename_errors_leave_the_table_unchanged() {
        let mut t = table(&[("a", "1"), ("b", "a + 1")]);
        let before = t.clone();
        assert_eq!(
            t.rename("zz", "c"),
            Err(VariableError::UnknownVariable { name: "zz".into() })
        );
        assert_eq!(
            t.rename("a", "b"),
            Err(VariableError::DuplicateName { name: "b".into() })
        );
        assert!(matches!(
            t.rename("a", "deg"),
            Err(VariableError::ReservedName { .. })
        ));
        assert_eq!(t.rename("a", "9a"), Err(VariableError::NameStartsWithDigit));
        assert_eq!(t, before);
        // Renaming to itself is a no-op, not a duplicate.
        assert_eq!(t.rename("a", "a"), Ok(()));
        assert_eq!(t, before);
    }

    #[test]
    fn rename_that_would_overflow_a_source_changes_nothing() {
        // `user` is one byte under the cap and mentions `a` once, so a
        // longer name pushes it over.
        let padding = " ".repeat(MAX_SOURCE_LEN - 2);
        let mut t = table(&[
            ("a", "1"),
            ("first", "a + 1"),
            ("user", &format!("a{padding}")),
        ]);
        let before = t.clone();
        assert_eq!(
            t.rename("a", "abc"),
            Err(VariableError::SourceTooLong {
                variable: "user".into(),
                len: MAX_SOURCE_LEN + 1,
                max: MAX_SOURCE_LEN
            })
        );
        assert_eq!(t, before);
        t.rename("a", "ab").unwrap();
        assert_eq!(t.get("first").unwrap().source(), "ab + 1");
        assert!(t.evaluate().is_ok());
    }

    #[test]
    fn rename_keeps_position_and_self_references() {
        let mut t = table(&[("a", "1"), ("loop_", "loop_ + a"), ("z", "2")]);
        t.rename("loop_", "spin").unwrap();
        assert_eq!(sources(&t), [("a", "1"), ("spin", "spin + a"), ("z", "2")]);
        assert_eq!(kind(&t.evaluate(), "spin"), cycle(&["spin"]));
    }

    // --- conditions ---

    /// A reference in a branch that is not taken is still a dependency:
    /// the order puts it first and a cycle through it is a cycle, so the
    /// table does not change shape when a condition flips.
    #[test]
    fn a_cycle_through_a_branch_not_taken_is_a_cycle() {
        let t = table(&[
            ("big", "0"),
            ("a", "if(big > 0, b, 1 mm)"),
            ("b", "a + 1 mm"),
        ]);
        let e = t.evaluate();
        assert_eq!(kind(&e, "a"), cycle(&["a", "b"]));
        assert_eq!(kind(&e, "b"), cycle(&["a", "b"]));
        let t = table(&[("n", "choose(1, 2, n)")]);
        assert_eq!(kind(&t.evaluate(), "n"), cycle(&["n"]));
        let t = table(&[("c", "1 || c")]);
        assert_eq!(kind(&t.evaluate(), "c"), cycle(&["c"]));
    }

    #[test]
    fn dependencies_of_every_branch_come_first() {
        let t = table(&[
            ("pick", "if(flag, early, late)"),
            ("late", "2 mm"),
            ("flag", "1"),
            ("early", "1 mm"),
        ]);
        assert_eq!(t.evaluation_order(), ["late", "flag", "early", "pick"]);
        assert_eq!(t.evaluate().get("pick"), Some(&length(1.0)));
    }

    #[test]
    fn a_failed_variable_in_a_branch_not_taken_does_not_count() {
        let t = table(&[
            ("count", "0"),
            ("total", "12 mm"),
            ("share", "total / count"),
            ("each", "if(count > 0, share, total)"),
            ("other", "if(count == 0, share, total)"),
        ]);
        let e = t.evaluate();
        assert_eq!(kind(&e, "share"), ErrorKind::DivisionByZero);
        assert_eq!(e.get("each"), Some(&length(12.0)));
        assert_eq!(
            e.get("other"),
            Some(&Err(ExprError::new(15, dependency_failed("share"))))
        );
    }

    #[test]
    fn an_unknown_name_is_refused_also_in_a_branch_not_taken() {
        let t = table(&[
            ("on", "0"),
            ("a", "if(on > 0, widht, 2 mm)"),
            ("b", "choose(1, 1, typo)"),
            ("c", "0 && nope"),
        ]);
        let e = t.evaluate();
        let unknown = |name: &str| ErrorKind::UnknownVariable {
            name: name.to_string(),
        };
        assert_eq!(e.get("a"), Some(&Err(ExprError::new(11, unknown("widht")))));
        assert_eq!(kind(&e, "b"), unknown("typo"));
        assert_eq!(kind(&e, "c"), unknown("nope"));
        // The same for a parameter evaluated against the table.
        assert_eq!(
            e.evaluate("if(on > 0, widht, 1)").unwrap_err().kind,
            unknown("widht")
        );
        // `b` failed: it counts only where it is used.
        assert_eq!(
            e.evaluate("if(on == 0, b, 1)").unwrap_err().kind,
            dependency_failed("b")
        );
        assert_eq!(e.evaluate("if(on == 0, 1, b)").unwrap().value, 1.0);
        assert_eq!(e.evaluate("on + 2").unwrap().value, 2.0);
    }

    /// An error of the expression itself is not blamed on a dependency
    /// that failed but was not reached.
    #[test]
    fn an_own_error_is_reported_as_itself() {
        let t = table(&[
            ("bad", "1 / 0"),
            ("x", "1 mm + 1 deg + if(0, bad, 1 mm)"),
            ("y", "bad + 1"),
        ]);
        let e = t.evaluate();
        assert!(matches!(kind(&e, "x"), ErrorKind::DimensionMismatch { .. }));
        assert_eq!(kind(&e, "y"), dependency_failed("bad"));
    }
}
