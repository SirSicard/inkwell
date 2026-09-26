//! Swift `Codable` types from the [`Schema`].
//!
//! - A string enum becomes a `String`-backed enum.
//! - An object becomes a struct of `let`s; an optional field is an optional, decoded when present.
//! - The events become `InkEvent`, an enum with one case per event that decodes by `"type"` and
//!   keeps an event type it does not know as `.unknown`, rather than failing.
//!
//! JSON names are `snake_case`; Swift names are `camelCase`, with `CodingKeys` mapping them.

use std::fmt::Write;

use super::{DefKind, Field, Schema, Ty};

/// Words Swift reserves; an identifier equal to one is written in backticks.
const RESERVED: &[&str] = &[
    "Any",
    "Self",
    "Type",
    "as",
    "associatedtype",
    "break",
    "case",
    "catch",
    "class",
    "continue",
    "default",
    "defer",
    "deinit",
    "do",
    "else",
    "enum",
    "extension",
    "fallthrough",
    "false",
    "fileprivate",
    "final",
    "for",
    "func",
    "guard",
    "if",
    "import",
    "in",
    "init",
    "inout",
    "internal",
    "is",
    "let",
    "nil",
    "open",
    "operator",
    "private",
    "protocol",
    "public",
    "repeat",
    "rethrows",
    "return",
    "self",
    "some",
    "static",
    "struct",
    "subscript",
    "super",
    "switch",
    "throw",
    "throws",
    "true",
    "try",
    "typealias",
    "var",
    "where",
    "while",
];

fn camel(snake: &str, upper_first: bool) -> String {
    let mut out = String::new();
    let mut upper = upper_first;
    for c in snake.chars() {
        if c == '_' || c == '.' {
            upper = true;
        } else if upper {
            out.extend(c.to_uppercase());
            upper = false;
        } else {
            out.push(c);
        }
    }
    out
}

fn ident(name: &str) -> String {
    if RESERVED.contains(&name) {
        format!("`{name}`")
    } else {
        name.to_owned()
    }
}

fn lower_first(name: &str) -> String {
    let mut c = name.chars();
    c.next()
        .map(|f| f.to_lowercase().chain(c).collect())
        .unwrap_or_default()
}

fn swift_type(ty: &Ty) -> String {
    match ty {
        Ty::String => "String".into(),
        Ty::Integer => "Int64".into(),
        Ty::Number => "Double".into(),
        Ty::Boolean => "Bool".into(),
        Ty::Array(item) => format!("[{}]", swift_type(item)),
        Ty::Ref(name) => name.clone(),
    }
}

fn doc(out: &mut String, indent: &str, text: &str) {
    if text.trim().is_empty() {
        return;
    }
    for line in wrap(text, 96usize.saturating_sub(indent.len() + 4)) {
        let _ = writeln!(out, "{indent}/// {line}");
    }
}

/// Greedy word wrap, so long descriptions stay readable in the generated file.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = vec![String::new()];
    for word in text.split_whitespace() {
        let line = lines.last_mut().expect("starts with one line");
        if !line.is_empty() && line.len() + 1 + word.len() > width {
            lines.push(word.to_owned());
        } else {
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
    }
    lines
}

fn object(out: &mut String, name: &str, fields: &[Field]) {
    let _ = writeln!(out, "public struct {name}: Codable, Sendable, Equatable {{");
    for f in fields {
        match &f.constant {
            Some(c) if f.doc.is_empty() => doc(out, "    ", &format!("Always `{c}`.")),
            _ => doc(out, "    ", &f.doc),
        }
        let optional = if f.required { "" } else { "?" };
        let _ = writeln!(
            out,
            "    public let {}: {}{optional}",
            ident(&camel(&f.name, false)),
            swift_type(&f.ty)
        );
    }
    if fields.iter().any(|f| camel(&f.name, false) != f.name) {
        out.push_str("\n    private enum CodingKeys: String, CodingKey {\n");
        for f in fields {
            let swift = camel(&f.name, false);
            if swift == f.name {
                let _ = writeln!(out, "        case {}", ident(&swift));
            } else {
                let _ = writeln!(out, "        case {} = \"{}\"", ident(&swift), f.name);
            }
        }
        out.push_str("    }\n");
    }
    out.push_str("}\n");
}

/// The Swift source for `schema`.
pub fn swift(schema: &Schema) -> String {
    let mut out = String::new();
    out.push_str(
        "// Generated from schema/events.schema.json by `cargo run -p ink-ffi --bin ink-schema`.\n\
         // Do not edit: change the schema and regenerate. A core test fails while this is stale.\n\n\
         import Foundation\n\n",
    );
    doc(&mut out, "", &schema.doc);
    out.push_str("public enum InkEvent: Codable, Sendable, Equatable {\n");
    for e in &schema.events {
        if let Some(ty) = schema.event_type(e) {
            let _ = writeln!(out, "    /// `{ty}`");
        }
        let _ = writeln!(out, "    case {}({e})", ident(&lower_first(e)));
    }
    out.push_str(
        "    /// An event this build does not know. The core and the shell ship together, so this\n    \
         /// means a mismatched build.\n    \
         case unknown(type: String)\n\n    \
         private enum TypeKey: String, CodingKey {\n        case type\n    }\n\n    \
         /// Decodes one event: the JSON an `InkEventCallback` receives.\n    \
         public static func decode(_ json: Data) throws -> InkEvent {\n        \
         try JSONDecoder().decode(InkEvent.self, from: json)\n    }\n\n    \
         public init(from decoder: Decoder) throws {\n        \
         let type = try decoder.container(keyedBy: TypeKey.self).decode(String.self, forKey: .type)\n        \
         switch type {\n",
    );
    for e in &schema.events {
        if let Some(ty) = schema.event_type(e) {
            let _ = writeln!(
                out,
                "        case \"{ty}\": self = .{}(try {e}(from: decoder))",
                ident(&lower_first(e))
            );
        }
    }
    out.push_str(
        "        default: self = .unknown(type: type)\n        }\n    }\n\n    \
         public func encode(to encoder: Encoder) throws {\n        switch self {\n",
    );
    for e in &schema.events {
        let _ = writeln!(
            out,
            "        case .{}(let event): try event.encode(to: encoder)",
            ident(&lower_first(e))
        );
    }
    out.push_str(
        "        case .unknown(let type):\n            \
         var c = encoder.container(keyedBy: TypeKey.self)\n            \
         try c.encode(type, forKey: .type)\n        }\n    }\n}\n",
    );
    for d in &schema.defs {
        out.push('\n');
        doc(&mut out, "", &d.doc);
        match &d.kind {
            DefKind::Enum(values) => {
                let _ = writeln!(
                    out,
                    "public enum {}: String, Codable, Sendable, Equatable, CaseIterable {{",
                    d.name
                );
                for v in values {
                    let swift = camel(v, false);
                    if swift == *v {
                        let _ = writeln!(out, "    case {}", ident(&swift));
                    } else {
                        let _ = writeln!(out, "    case {} = \"{v}\"", ident(&swift));
                    }
                }
                out.push_str("}\n");
            }
            DefKind::Object(fields) => object(&mut out, &d.name, fields),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_become_swift_names() {
        assert_eq!(camel("speech_too_short", false), "speechTooShort");
        assert_eq!(camel("core.ready", true), "CoreReady");
        assert_eq!(ident("final"), "`final`");
        assert_eq!(ident("mic"), "mic");
        assert_eq!(lower_first("MeetingFinal"), "meetingFinal");
    }

    #[test]
    fn a_small_schema_generates_the_expected_swift() {
        let schema = Schema::parse(
            r##"{
              "description": "Events.",
              "oneOf": [{"$ref": "#/$defs/Tick"}],
              "$defs": {
                "Phase": {"type": "string", "description": "When.", "enum": ["live", "final"]},
                "Tick": {"type": "object", "description": "A tick.", "additionalProperties": false,
                  "properties": {"type": {"const": "tick.now"},
                    "at_ms": {"type": "integer", "minimum": 0, "description": "When, ms."},
                    "phase": {"$ref": "#/$defs/Phase", "description": "Which."}},
                  "required": ["type", "at_ms"]}
              }
            }"##,
        )
        .unwrap();
        let out = swift(&schema);
        for expected in [
            "case tick(Tick)",
            "case \"tick.now\": self = .tick(try Tick(from: decoder))",
            "public let atMs: Int64\n",
            "public let phase: Phase?\n",
            "case atMs = \"at_ms\"",
            "case `final`\n",
            "public enum Phase: String, Codable, Sendable, Equatable, CaseIterable {",
        ] {
            assert!(out.contains(expected), "missing {expected:?} in:\n{out}");
        }
    }
}
