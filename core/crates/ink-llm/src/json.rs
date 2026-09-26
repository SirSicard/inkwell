//! Reading JSON out of model answers, with errors that name fields, never values.
//!
//! `serde_json`'s own error messages can quote the input ("invalid type: string \"…\""), so they
//! are never passed on: every error here is built from a task name and a field name, both
//! `'static`.

use ink_core::LlmError;
use serde_json::{Map, Value};

/// A `BadResponse` naming the task and what was wrong.
pub(crate) fn bad(task: &'static str, what: &'static str) -> LlmError {
    LlmError::BadResponse(format!("{task}: {what}"))
}

/// A `BadResponse` naming the task, a field, and what was wrong with it.
pub(crate) fn bad_field(task: &'static str, field: &'static str, what: &'static str) -> LlmError {
    LlmError::BadResponse(format!("{task}: `{field}` {what}"))
}

/// The JSON object in a model's answer. Models wrap JSON in prose or code fences even when told
/// not to, so this takes the text from the first `{` to the last `}`.
pub(crate) fn object_in(text: &str, task: &'static str) -> Result<Map<String, Value>, LlmError> {
    let start = text
        .find('{')
        .ok_or_else(|| bad(task, "no JSON object in the answer"))?;
    let end = text
        .rfind('}')
        .ok_or_else(|| bad(task, "no JSON object in the answer"))?;
    let slice = text
        .get(start..=end)
        .ok_or_else(|| bad(task, "no JSON object in the answer"))?;
    match serde_json::from_str::<Value>(slice) {
        Ok(Value::Object(map)) => Ok(map),
        _ => Err(bad(task, "the answer is not a JSON object")),
    }
}

/// Typed access to one object's fields for one task.
pub(crate) struct Fields<'a> {
    task: &'static str,
    map: &'a Map<String, Value>,
}

impl<'a> Fields<'a> {
    pub(crate) fn new(task: &'static str, map: &'a Map<String, Value>) -> Self {
        Self { task, map }
    }

    fn get(&self, key: &'static str) -> Result<&'a Value, LlmError> {
        self.map
            .get(key)
            .ok_or_else(|| bad_field(self.task, key, "is missing"))
    }

    pub(crate) fn string(&self, key: &'static str) -> Result<&'a str, LlmError> {
        self.get(key)?
            .as_str()
            .ok_or_else(|| bad_field(self.task, key, "is not a string"))
    }

    /// A string or `null`; required to be present.
    pub(crate) fn nullable_string(&self, key: &'static str) -> Result<Option<&'a str>, LlmError> {
        match self.get(key)? {
            Value::Null => Ok(None),
            Value::String(s) => Ok(Some(s.as_str())),
            _ => Err(bad_field(self.task, key, "is not a string or null")),
        }
    }

    /// A string, `null`, or absent.
    pub(crate) fn optional_string(&self, key: &'static str) -> Result<Option<&'a str>, LlmError> {
        match self.map.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(s)) => Ok(Some(s.as_str())),
            Some(_) => Err(bad_field(self.task, key, "is not a string or null")),
        }
    }

    /// A non-negative integer, `null`, or absent.
    pub(crate) fn optional_index(&self, key: &'static str) -> Result<Option<usize>, LlmError> {
        match self.map.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => v
                .as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .map(Some)
                .ok_or_else(|| bad_field(self.task, key, "is not a non-negative integer")),
        }
    }

    pub(crate) fn number(&self, key: &'static str) -> Result<f64, LlmError> {
        self.get(key)?
            .as_f64()
            .ok_or_else(|| bad_field(self.task, key, "is not a number"))
    }

    pub(crate) fn boolean(&self, key: &'static str) -> Result<bool, LlmError> {
        self.get(key)?
            .as_bool()
            .ok_or_else(|| bad_field(self.task, key, "is not true or false"))
    }

    pub(crate) fn array(&self, key: &'static str) -> Result<&'a Vec<Value>, LlmError> {
        self.get(key)?
            .as_array()
            .ok_or_else(|| bad_field(self.task, key, "is not an array"))
    }

    /// An array, or absent (treated as empty).
    pub(crate) fn optional_array(&self, key: &'static str) -> Result<&'a [Value], LlmError> {
        match self.map.get(key) {
            None | Some(Value::Null) => Ok(&[]),
            Some(Value::Array(items)) => Ok(items.as_slice()),
            Some(_) => Err(bad_field(self.task, key, "is not an array")),
        }
    }

    /// The object in `key`'s array item, for nested shapes.
    pub(crate) fn object_item(
        task: &'static str,
        key: &'static str,
        item: &'a Value,
    ) -> Result<Fields<'a>, LlmError> {
        item.as_object()
            .map(|map| Fields::new(task, map))
            .ok_or_else(|| bad_field(task, key, "holds an item that is not an object"))
    }

    /// Strings in an array, or absent.
    pub(crate) fn optional_strings(&self, key: &'static str) -> Result<Vec<String>, LlmError> {
        self.optional_array(key)?
            .iter()
            .map(|v| {
                v.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| bad_field(self.task, key, "holds an item that is not a string"))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_object_is_found_inside_fences_and_prose() {
        let map = object_in("Sure:\n```json\n{\"a\": 1}\n```\nDone.", "t").unwrap();
        assert_eq!(map.get("a"), Some(&Value::from(1)));
        assert!(object_in("no json here", "t").is_err());
        assert!(object_in("[1, 2]", "t").is_err());
        assert!(object_in("} backwards {", "t").is_err());
    }

    #[test]
    fn errors_name_fields_never_values() {
        let map = object_in(r#"{"class": 42, "quote": "synthetic canary"}"#, "judge").unwrap();
        let fields = Fields::new("judge", &map);
        let err = fields.string("class").unwrap_err();
        assert_eq!(
            err,
            LlmError::BadResponse("judge: `class` is not a string".into())
        );
        let err = fields.number("quote").unwrap_err();
        assert!(!err.to_string().contains("canary"));
    }
}
