//! The steps of a design as they are kept: groups of commands with their
//! params, in which every reference to a feature by id is written by the
//! feature's name, `{N3}` for "F7", so that a reproducer still means the
//! same thing when the shrinker drops a group before it (ids are handed
//! out in order and would shift; names are given by the generator and do
//! not).

use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

/// One command of a group.
#[derive(Clone, Debug, PartialEq)]
pub struct Cmd {
    pub command: String,
    pub params: Value,
}

/// What the generator adds in one go: a primitive, a sketch with its
/// profile and the extrusion of it, a boolean, a fillet. A group is kept
/// or undone as a whole.
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    /// What kind of step it is (`primitive`, `extrude`, `boolean`, ...).
    pub step: String,
    pub commands: Vec<Cmd>,
}

impl Group {
    /// The command the group is about: its last one (the sketch commands
    /// before an extrusion only prepare it).
    pub fn main_command(&self) -> &str {
        self.commands.last().map_or("", |c| c.command.as_str())
    }
}

/// The document a design is built in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Setup {
    pub seed: u64,
    /// Lengths are multiples of `10 * scale` mm (the base size), the
    /// tolerance is `1e-6 * scale`.
    pub scale: f64,
}

impl Setup {
    pub fn base(&self) -> f64 {
        10.0 * self.scale
    }

    pub fn tolerance(&self) -> f64 {
        1e-6 * self.scale
    }
}

/// A whole design: the setup and its groups, as a reproducer file holds it.
#[derive(Clone, Debug, PartialEq)]
pub struct Design {
    pub setup: Setup,
    pub groups: Vec<Group>,
}

/// The largest design file read, in groups and commands per group: a
/// reproducer is small, and the file may be hostile.
pub const MAX_GROUPS: usize = 4096;
pub const MAX_COMMANDS: usize = 64;

impl Design {
    pub fn to_json(&self) -> Value {
        json!({
            "seed": self.setup.seed,
            "scale": self.setup.scale,
            "groups": self.groups.iter().map(|g| json!({
                "step": g.step,
                "commands": g.commands.iter().map(|c| json!({
                    "command": c.command, "params": c.params,
                })).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
        })
    }

    pub fn from_json(value: &Value) -> Result<Design, String> {
        let seed = value.get("seed").and_then(Value::as_u64).unwrap_or(0);
        let scale = value
            .get("scale")
            .and_then(Value::as_f64)
            .filter(|s| s.is_finite() && *s > 0.0)
            .ok_or("the design has no positive `scale`")?;
        let groups = value
            .get("groups")
            .and_then(Value::as_array)
            .ok_or("the design has no `groups` array")?;
        if groups.len() > MAX_GROUPS {
            return Err(format!("more than {MAX_GROUPS} groups"));
        }
        let mut out = Vec::with_capacity(groups.len());
        for (i, g) in groups.iter().enumerate() {
            let step = g
                .get("step")
                .and_then(Value::as_str)
                .unwrap_or("step")
                .to_string();
            let commands = g
                .get("commands")
                .and_then(Value::as_array)
                .ok_or_else(|| format!("group {i} has no `commands` array"))?;
            if commands.len() > MAX_COMMANDS {
                return Err(format!("group {i} has more than {MAX_COMMANDS} commands"));
            }
            let mut cmds = Vec::with_capacity(commands.len());
            for c in commands {
                let command = c
                    .get("command")
                    .and_then(Value::as_str)
                    .ok_or_else(|| format!("a command of group {i} has no `command`"))?;
                let params = c
                    .get("params")
                    .cloned()
                    .unwrap_or_else(|| Value::Object(Map::new()));
                cmds.push(Cmd {
                    command: command.to_string(),
                    params,
                });
            }
            out.push(Group {
                step,
                commands: cmds,
            });
        }
        Ok(Design {
            setup: Setup { seed, scale },
            groups: out,
        })
    }
}

/// `text` with every feature id ("F12", not inside a longer word) whose
/// name `names` knows written as `{name}`.
pub fn symbolic_text(text: &str, names: &BTreeMap<String, String>) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while let Some(&c) = chars.get(i) {
        let starts_word = i == 0
            || chars
                .get(i.wrapping_sub(1))
                .is_some_and(|p| !p.is_alphanumeric() && *p != '_' && *p != '{');
        if c == 'F' && starts_word {
            let mut j = i + 1;
            while chars.get(j).is_some_and(char::is_ascii_digit) {
                j += 1;
            }
            let ends_word = chars
                .get(j)
                .is_none_or(|n| !n.is_alphanumeric() && *n != '_');
            if j > i + 1 && ends_word {
                let id: String = chars.get(i..j).unwrap_or_default().iter().collect();
                if let Some(name) = names.get(&id) {
                    out.push('{');
                    out.push_str(name);
                    out.push('}');
                    i = j;
                    continue;
                }
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

/// `text` with every `{name}` that `ids` knows written as its id; an
/// unknown one is left as it is (the command then refuses it).
pub fn resolved_text(text: &str, ids: &BTreeMap<String, String>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('{') {
        let (before, after) = rest.split_at(open);
        out.push_str(before);
        let Some(close) = after.find('}') else {
            out.push_str(after);
            return out;
        };
        let name = after.get(1..close).unwrap_or_default();
        match ids.get(name) {
            Some(id) => out.push_str(id),
            None => out.push_str(after.get(..=close).unwrap_or_default()),
        }
        rest = after.get(close + 1..).unwrap_or_default();
    }
    out.push_str(rest);
    out
}

/// Every string of `value` mapped with `f`, keys left alone.
pub fn map_strings(value: &Value, f: &dyn Fn(&str) -> String, depth: u32) -> Value {
    if depth > 64 {
        return value.clone();
    }
    match value {
        Value::String(s) => Value::String(f(s)),
        Value::Array(items) => {
            Value::Array(items.iter().map(|v| map_strings(v, f, depth + 1)).collect())
        }
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), map_strings(v, f, depth + 1)))
                .collect(),
        ),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names() -> BTreeMap<String, String> {
        [("F1", "N0"), ("F12", "N4")]
            .into_iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect()
    }

    #[test]
    fn ids_become_names_and_back() {
        let text = "F12:copy(0)/F1:face(1) F1x F123 xF1 F";
        let s = symbolic_text(text, &names());
        assert_eq!(s, "{N4}:copy(0)/{N0}:face(1) F1x F123 xF1 F");
        let ids: BTreeMap<String, String> =
            names().into_iter().map(|(id, name)| (name, id)).collect();
        assert_eq!(resolved_text(&s, &ids), text);
        assert_eq!(resolved_text("{nope} {", &ids), "{nope} {");
    }

    #[test]
    fn a_design_reads_back_as_written() {
        let design = Design {
            setup: Setup {
                seed: 3,
                scale: 1e-3,
            },
            groups: vec![Group {
                step: "primitive".into(),
                commands: vec![Cmd {
                    command: "feature.block".into(),
                    params: json!({"size": [1, 2, 3]}),
                }],
            }],
        };
        assert_eq!(Design::from_json(&design.to_json()), Ok(design));
        assert!(Design::from_json(&json!({"scale": -1, "groups": []})).is_err());
        assert!(Design::from_json(&json!(5)).is_err());
    }
}
