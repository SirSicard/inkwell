//! The event schema (`schema/events.schema.json`) and the generator for the shells' types
//! (architecture rule 8: events are defined once).
//!
//! The file is JSON Schema, restricted to what every target language maps without judgement:
//!
//! - a root `oneOf` of references to the events, in order;
//! - `$defs` holding string enums (`"type": "string"` with `"enum"`) and objects (`"type":
//!   "object"`, `"properties"`, `"required"`, `"additionalProperties": false`);
//! - properties that are a scalar (`string`, `integer`, `number`, `boolean`), an `array` of one
//!   of these, or a `$ref` to a def; an event's `type` property is a `const`.
//!
//! [`Schema::parse`] reads it into a small model and refuses anything outside that subset, so a
//! schema edit the generators cannot express fails at once instead of producing wrong types.
//! Emitters work from the model: [`swift`] now; a C# emitter joins it for the Windows shell.
//! [`Schema::validate`] checks an event against the same model, which is how the tests hold the
//! Rust side to the schema.

pub mod swift;

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value};

/// A property's type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ty {
    /// A string.
    String,
    /// A whole number (the core never sends one above `i64::MAX`).
    Integer,
    /// A number.
    Number,
    /// `true` or `false`.
    Boolean,
    /// A list.
    Array(Box<Ty>),
    /// A def by name.
    Ref(String),
}

/// A property of an object.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    /// The JSON name.
    pub name: String,
    /// Its description.
    pub doc: String,
    /// Its type.
    pub ty: Ty,
    /// Whether it is always present. Optional fields are omitted, never null.
    pub required: bool,
    /// The one value it may have (an event's `type`).
    pub constant: Option<String>,
}

/// What a def is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DefKind {
    /// A string with one of these values.
    Enum(Vec<String>),
    /// An object with these fields, sorted by name.
    Object(Vec<Field>),
}

/// A named type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Def {
    /// Its name (PascalCase).
    pub name: String,
    /// Its description.
    pub doc: String,
    /// What it is.
    pub kind: DefKind,
}

/// The schema, as the emitters and the validator see it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Schema {
    /// The root description.
    pub doc: String,
    /// Every def, sorted by name (so output never depends on a JSON map's order).
    pub defs: Vec<Def>,
    /// The events: def names in the root `oneOf`'s order.
    pub events: Vec<String>,
}

fn text(v: &Map<String, Value>, key: &str, at: &str) -> Result<String, String> {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("{at}: needs a string \"{key}\""))
}

fn keys_only(v: &Map<String, Value>, allowed: &[&str], at: &str) -> Result<(), String> {
    match v.keys().find(|k| !allowed.contains(&k.as_str())) {
        Some(k) => Err(format!(
            "{at}: \"{k}\" is outside the subset the generators read"
        )),
        None => Ok(()),
    }
}

fn ref_name(r: &str, at: &str) -> Result<String, String> {
    r.strip_prefix("#/$defs/")
        .map(str::to_owned)
        .ok_or_else(|| format!("{at}: a $ref must point into #/$defs/"))
}

fn scalar(name: &str, at: &str) -> Result<Ty, String> {
    Ok(match name {
        "string" => Ty::String,
        "integer" => Ty::Integer,
        "number" => Ty::Number,
        "boolean" => Ty::Boolean,
        other => return Err(format!("{at}: type \"{other}\" is not supported")),
    })
}

fn property(v: &Value, at: &str) -> Result<(Ty, String, Option<String>), String> {
    let v = v
        .as_object()
        .ok_or_else(|| format!("{at}: not an object"))?;
    let doc = v
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    if let Some(c) = v.get("const") {
        keys_only(v, &["const", "description"], at)?;
        let c = c
            .as_str()
            .ok_or_else(|| format!("{at}: only string consts are supported"))?;
        return Ok((Ty::String, doc, Some(c.to_owned())));
    }
    if let Some(r) = v.get("$ref").and_then(Value::as_str) {
        keys_only(v, &["$ref", "description"], at)?;
        return Ok((Ty::Ref(ref_name(r, at)?), doc, None));
    }
    let ty = text(v, "type", at)?;
    if ty == "array" {
        keys_only(v, &["type", "items", "description"], at)?;
        let items = v
            .get("items")
            .ok_or_else(|| format!("{at}: an array needs \"items\""))?;
        let (item, _, constant) = property(items, &format!("{at}.items"))?;
        if constant.is_some() {
            return Err(format!("{at}: an array of consts is not supported"));
        }
        return Ok((Ty::Array(Box::new(item)), doc, None));
    }
    keys_only(v, &["type", "description", "minimum"], at)?;
    Ok((scalar(&ty, at)?, doc, None))
}

fn def(name: &str, v: &Value) -> Result<Def, String> {
    let at = format!("$defs.{name}");
    let v = v
        .as_object()
        .ok_or_else(|| format!("{at}: not an object"))?;
    let doc = text(v, "description", &at)?;
    let kind = match text(v, "type", &at)?.as_str() {
        "string" => {
            keys_only(v, &["type", "description", "enum"], &at)?;
            let values = v
                .get("enum")
                .and_then(Value::as_array)
                .ok_or_else(|| format!("{at}: a string def must be an enum"))?
                .iter()
                .map(|e| e.as_str().map(str::to_owned))
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| format!("{at}: enum values must be strings"))?;
            if values.is_empty() || values.iter().collect::<BTreeSet<_>>().len() != values.len() {
                return Err(format!("{at}: enum values must be unique and not empty"));
            }
            DefKind::Enum(values)
        }
        "object" => {
            keys_only(
                v,
                &[
                    "type",
                    "description",
                    "properties",
                    "required",
                    "additionalProperties",
                ],
                &at,
            )?;
            if v.get("additionalProperties") != Some(&Value::Bool(false)) {
                return Err(format!(
                    "{at}: objects must set additionalProperties: false"
                ));
            }
            let props = v
                .get("properties")
                .and_then(Value::as_object)
                .ok_or_else(|| format!("{at}: an object needs \"properties\""))?;
            let required: BTreeSet<&str> = v
                .get("required")
                .and_then(Value::as_array)
                .map(|r| r.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            if let Some(missing) = required.iter().find(|r| !props.contains_key(**r)) {
                return Err(format!("{at}: required \"{missing}\" is not a property"));
            }
            // Sorted, whatever order the JSON map keeps (serde_json's depends on a feature).
            let sorted: BTreeMap<&String, &Value> = props.iter().collect();
            let mut fields = Vec::new();
            for (field, p) in sorted {
                let (ty, doc, constant) = property(p, &format!("{at}.{field}"))?;
                fields.push(Field {
                    name: field.clone(),
                    doc,
                    ty,
                    required: required.contains(field.as_str()),
                    constant,
                });
            }
            DefKind::Object(fields)
        }
        other => {
            return Err(format!(
                "{at}: a def must be a string enum or an object, not {other}"
            ));
        }
    };
    Ok(Def {
        name: name.to_owned(),
        doc,
        kind,
    })
}

impl Schema {
    /// Reads the schema file's text.
    pub fn parse(json: &str) -> Result<Self, String> {
        let root: Value = serde_json::from_str(json).map_err(|e| format!("not JSON: {e}"))?;
        let root = root.as_object().ok_or("the root is not an object")?;
        let doc = text(root, "description", "root")?;
        let defs_map = root
            .get("$defs")
            .and_then(Value::as_object)
            .ok_or("the root needs \"$defs\"")?;
        let sorted: BTreeMap<&String, &Value> = defs_map.iter().collect();
        let defs = sorted
            .into_iter()
            .map(|(name, v)| def(name, v))
            .collect::<Result<Vec<_>, _>>()?;
        let mut events = Vec::new();
        for (i, e) in root
            .get("oneOf")
            .and_then(Value::as_array)
            .ok_or("the root needs \"oneOf\"")?
            .iter()
            .enumerate()
        {
            let r = e
                .get("$ref")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("oneOf[{i}]: needs a $ref"))?;
            events.push(ref_name(r, &format!("oneOf[{i}]"))?);
        }
        let schema = Self { doc, defs, events };
        schema.check()?;
        Ok(schema)
    }

    /// Every reference resolves, every event is an object with a unique `type` const, and names
    /// are unique.
    fn check(&self) -> Result<(), String> {
        let mut types = BTreeSet::new();
        for e in &self.events {
            let ty = self
                .event_type(e)
                .ok_or_else(|| format!("event {e} needs a required \"type\" const"))?;
            if !types.insert(ty) {
                return Err(format!("event type {ty} is used twice"));
            }
        }
        fn refs(ty: &Ty, out: &mut Vec<String>) {
            match ty {
                Ty::Ref(r) => out.push(r.clone()),
                Ty::Array(item) => refs(item, out),
                _ => {}
            }
        }
        for d in &self.defs {
            if let DefKind::Object(fields) = &d.kind {
                let mut out = Vec::new();
                for f in fields {
                    refs(&f.ty, &mut out);
                }
                if let Some(missing) = out.iter().find(|r| self.def(r).is_none()) {
                    return Err(format!("{}: $ref {missing} is not defined", d.name));
                }
            }
        }
        Ok(())
    }

    /// The def called `name`.
    pub fn def(&self, name: &str) -> Option<&Def> {
        self.defs.iter().find(|d| d.name == name)
    }

    /// The `type` value of the event def `name`.
    pub fn event_type(&self, name: &str) -> Option<&str> {
        match &self.def(name)?.kind {
            DefKind::Object(fields) => fields
                .iter()
                .find(|f| f.name == "type" && f.required)
                .and_then(|f| f.constant.as_deref()),
            DefKind::Enum(_) => None,
        }
    }

    /// Checks one event: a known `type`, every required field, no field the schema lacks, and
    /// every value of its type.
    pub fn validate(&self, event: &Value) -> Result<(), String> {
        let ty = event
            .get("type")
            .and_then(Value::as_str)
            .ok_or("an event needs a string \"type\"")?;
        let name = self
            .events
            .iter()
            .find(|e| self.event_type(e) == Some(ty))
            .ok_or_else(|| format!("unknown event type {ty}"))?;
        self.value(&Ty::Ref(name.clone()), event, ty)
    }

    fn value(&self, ty: &Ty, v: &Value, at: &str) -> Result<(), String> {
        let ok = match ty {
            Ty::String => v.is_string(),
            Ty::Integer => v.is_u64() || v.is_i64(),
            Ty::Number => v.is_number(),
            Ty::Boolean => v.is_boolean(),
            Ty::Array(item) => {
                let items = v.as_array().ok_or_else(|| format!("{at}: not an array"))?;
                for (i, x) in items.iter().enumerate() {
                    self.value(item, x, &format!("{at}[{i}]"))?;
                }
                true
            }
            Ty::Ref(name) => {
                let def = self
                    .def(name)
                    .ok_or_else(|| format!("{at}: {name} is not defined"))?;
                match &def.kind {
                    DefKind::Enum(values) => {
                        let s = v.as_str().ok_or_else(|| format!("{at}: not a string"))?;
                        if !values.iter().any(|x| x == s) {
                            return Err(format!("{at}: \"{s}\" is not a {name}"));
                        }
                    }
                    DefKind::Object(fields) => {
                        let obj = v
                            .as_object()
                            .ok_or_else(|| format!("{at}: not an object"))?;
                        if let Some(extra) =
                            obj.keys().find(|k| !fields.iter().any(|f| &f.name == *k))
                        {
                            return Err(format!("{at}: {name} has no field \"{extra}\""));
                        }
                        for f in fields {
                            let here = format!("{at}.{}", f.name);
                            match obj.get(&f.name) {
                                None if f.required => {
                                    return Err(format!("{here}: required, missing"));
                                }
                                None => {}
                                Some(x) => {
                                    if let Some(c) = &f.constant
                                        && x.as_str() != Some(c)
                                    {
                                        return Err(format!("{here}: must be {c}"));
                                    }
                                    self.value(&f.ty, x, &here)?;
                                }
                            }
                        }
                    }
                }
                true
            }
        };
        if ok {
            Ok(())
        } else {
            Err(format!("{at}: not a {ty:?}"))
        }
    }
}

/// The schema file's text, as built into the crate: what the core's events are held to.
pub const EVENTS_SCHEMA: &str = include_str!("../../../../../schema/events.schema.json");

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn schema() -> Schema {
        Schema::parse(EVENTS_SCHEMA).unwrap()
    }

    #[test]
    fn the_checked_in_schema_parses() {
        let s = schema();
        assert!(s.events.len() > 30, "{}", s.events.len());
        assert_eq!(s.event_type("CoreReady"), Some("core.ready"));
    }

    #[test]
    fn validation_catches_what_the_shell_could_not_decode() {
        let s = schema();
        s.validate(&json!({"type": "core.ready", "abi": 1, "version": "0"}))
            .unwrap();
        for bad in [
            json!({"type": "core.ready", "abi": 1}),
            json!({"type": "core.ready", "abi": "1", "version": "0"}),
            json!({"type": "core.ready", "abi": 1, "version": "0", "extra": 1}),
            json!({"type": "no.such"}),
            json!({"type": "meeting.side_state", "record": "r", "channel": "left", "state": "ok"}),
            json!({"type": "meeting.finished", "record": "r", "revision": null}),
        ] {
            assert!(s.validate(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn anything_outside_the_subset_is_refused() {
        let base = |extra: Value| {
            json!({
                "description": "d",
                "oneOf": [{"$ref": "#/$defs/E"}],
                "$defs": {"E": {
                    "type": "object", "description": "e", "additionalProperties": false,
                    "properties": {"type": {"const": "e"}, "x": extra},
                    "required": ["type"]
                }}
            })
            .to_string()
        };
        Schema::parse(&base(json!({"type": "string"}))).unwrap();
        for bad in [
            json!({"type": "string", "format": "uuid"}),
            json!({"anyOf": [{"type": "string"}]}),
            json!({"type": "null"}),
            json!({"$ref": "#/$defs/Missing"}),
        ] {
            assert!(Schema::parse(&base(bad.clone())).is_err(), "{bad}");
        }
    }
}
