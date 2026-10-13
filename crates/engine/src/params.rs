//! Typed access to a command's JSON parameters, with errors that name the
//! command, the parameter and what was expected.

use crate::CommandError;
use serde_json::{Map, Value};

static EMPTY: std::sync::OnceLock<Map<String, Value>> = std::sync::OnceLock::new();

pub(crate) struct Params<'a> {
    command: &'static str,
    map: &'a Map<String, Value>,
}

fn describe(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

impl<'a> Params<'a> {
    /// `value` must be an object (or null, for none) whose keys are all in
    /// `allowed`.
    pub fn new(
        command: &'static str,
        value: &'a Value,
        allowed: &[&str],
    ) -> Result<Self, CommandError> {
        let map = match value {
            Value::Null => EMPTY.get_or_init(Map::new),
            Value::Object(map) => map,
            other => {
                return Err(CommandError::new(format!(
                    "{command}: params must be a JSON object, got {}",
                    describe(other)
                )));
            }
        };
        if let Some(unknown) = map.keys().find(|k| !allowed.contains(&k.as_str())) {
            let expected = if allowed.is_empty() {
                "this command takes no parameters".to_string()
            } else {
                format!("its parameters are: {}", allowed.join(", "))
            };
            return Err(CommandError::new(format!(
                "{command}: unknown parameter `{unknown}`; {expected}"
            )));
        }
        Ok(Self { command, map })
    }

    fn wrong(&self, key: &str, expected: &str, got: &Value) -> CommandError {
        CommandError::new(format!(
            "{}: `{key}` must be {expected}, got {}",
            self.command,
            describe(got)
        ))
    }

    pub fn missing(&self, key: &str) -> CommandError {
        CommandError::new(format!("{}: missing parameter `{key}`", self.command))
    }

    /// The raw value, with JSON null counting as absent.
    pub fn get(&self, key: &str) -> Option<&'a Value> {
        self.map.get(key).filter(|v| !v.is_null())
    }

    /// True when the key is present, even as null.
    pub fn has(&self, key: &str) -> bool {
        self.map.contains_key(key)
    }

    pub fn str(&self, key: &str) -> Result<Option<&'a str>, CommandError> {
        match self.get(key) {
            None => Ok(None),
            Some(Value::String(s)) => Ok(Some(s)),
            Some(other) => Err(self.wrong(key, "a string", other)),
        }
    }

    pub fn required_str(&self, key: &str) -> Result<&'a str, CommandError> {
        self.str(key)?.ok_or_else(|| self.missing(key))
    }

    pub fn bool(&self, key: &str) -> Result<Option<bool>, CommandError> {
        match self.get(key) {
            None => Ok(None),
            Some(Value::Bool(b)) => Ok(Some(*b)),
            Some(other) => Err(self.wrong(key, "true or false", other)),
        }
    }

    /// A finite number.
    pub fn number(&self, key: &str) -> Result<Option<f64>, CommandError> {
        match self.get(key) {
            None => Ok(None),
            Some(value) => match value.as_f64().filter(|n| n.is_finite()) {
                Some(n) => Ok(Some(n)),
                None => Err(self.wrong(key, "a finite number", value)),
            },
        }
    }

    /// A whole number within `min..=max`.
    pub fn integer(&self, key: &str, min: u64, max: u64) -> Result<Option<u64>, CommandError> {
        match self.get(key) {
            None => Ok(None),
            Some(value) => match value.as_u64().filter(|n| (min..=max).contains(n)) {
                Some(n) => Ok(Some(n)),
                None => Err(CommandError::new(format!(
                    "{}: `{key}` must be a whole number from {min} to {max}, got {value}",
                    self.command
                ))),
            },
        }
    }

    /// An expression: a string such as `"width / 2"`, or a plain number,
    /// which is taken in the document's unit.
    pub fn expr(&self, key: &str) -> Result<Option<String>, CommandError> {
        match self.get(key) {
            None => Ok(None),
            Some(value) => self.expr_value(key, value).map(Some),
        }
    }

    /// The command these are the parameters of.
    pub fn command(&self) -> &'static str {
        self.command
    }

    /// An expression inside the value of parameter `key` (an item of an
    /// array): a string or a finite number.
    pub fn expr_of(&self, key: &str, value: &Value) -> Result<String, CommandError> {
        self.expr_value(key, value)
    }

    fn expr_value(&self, key: &str, value: &Value) -> Result<String, CommandError> {
        match value {
            Value::String(s) => Ok(s.clone()),
            Value::Number(n) => match n.as_f64().filter(|x| x.is_finite()) {
                Some(x) => Ok(x.to_string()),
                None => Err(self.wrong(key, "an expression (a string or a finite number)", value)),
            },
            other => Err(self.wrong(key, "an expression (a string or a number)", other)),
        }
    }

    /// Three expressions: `[x, y, z]`.
    pub fn expr3(&self, key: &str) -> Result<Option<[String; 3]>, CommandError> {
        let Some(value) = self.get(key) else {
            return Ok(None);
        };
        let wrong = || self.wrong(key, "an array of three expressions [x, y, z]", value);
        let Value::Array(items) = value else {
            return Err(wrong());
        };
        let [x, y, z] = items.as_slice() else {
            return Err(wrong());
        };
        Ok(Some([
            self.expr_value(key, x)?,
            self.expr_value(key, y)?,
            self.expr_value(key, z)?,
        ]))
    }
}
