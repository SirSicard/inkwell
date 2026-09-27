// Structured answers on the on-device model (S2.8): a meeting's summary, the commitment judge and
// the "said twice" check ask for JSON in a shape of their own (a JSON Schema in the request).
// Foundation Models generates to a schema of its own kind, so the request's schema is turned into
// one here (guided generation: the model cannot answer outside it), and the answer comes back as
// JSON text for the core, which still checks it against its own task's shape.
//
// The subset the core's schemas use, and nothing more: objects with properties and a required
// list; strings (an "enum" becomes a choice); numbers, integers and booleans; arrays of one item
// schema; and a type that may also be null (["string", "null"]), which becomes an optional
// property. Bounds ("minimum", "maximum") are left to the core's check. Anything else is refused as
// a bad request, never answered loosely.
//
// Foundation Models leaves an optional property out when it has nothing for it; the core's parsers
// want a nullable field present as null ("task": null), so `completed(_:schema:)` puts each
// nullable property that is missing back as null.
import Foundation
#if canImport(FoundationModels)
    import FoundationModels
#endif

/// Why a request's schema cannot be used.
public struct UnsupportedSchema: Error, Equatable, Sendable {
    /// Where in the schema, as a path of property names; never content.
    public let path: String
}

/// The request's JSON Schema, read into the subset above.
public indirect enum SchemaNode: Equatable, Sendable {
    case string(choices: [String]?)
    case number
    case integer
    case boolean
    case array(SchemaNode)
    /// Properties in the order the schema lists them; `optional` when not required or nullable,
    /// `nullable` when its type allows null.
    case object([(name: String, node: SchemaNode, optional: Bool, nullable: Bool)])

    public static func == (a: SchemaNode, b: SchemaNode) -> Bool {
        switch (a, b) {
        case (.string(let x), .string(let y)): x == y
        case (.number, .number), (.integer, .integer), (.boolean, .boolean): true
        case (.array(let x), .array(let y)): x == y
        case (.object(let x), .object(let y)):
            x.count == y.count && zip(x, y).allSatisfy {
                $0.name == $1.name && $0.node == $1.node && $0.optional == $1.optional && $0.nullable == $1.nullable
            }
        default: false
        }
    }

    /// Reads `json`, a JSON Schema text.
    public static func parse(_ json: String) throws -> SchemaNode {
        guard let object = try? JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any] else {
            throw UnsupportedSchema(path: "$")
        }
        return try read(object, path: "$").node
    }

    /// A node, and whether its type allows null.
    private static func read(_ schema: [String: Any], path: String) throws -> (node: SchemaNode, nullable: Bool) {
        var types: [String]
        switch schema["type"] {
        case let one as String: types = [one]
        case let many as [String]: types = many
        default: throw UnsupportedSchema(path: path)
        }
        let nullable = types.contains("null")
        types.removeAll { $0 == "null" }
        guard types.count == 1, let type = types.first else { throw UnsupportedSchema(path: path) }
        switch type {
        case "string":
            let choices = schema["enum"] as? [String]
            if schema["enum"] != nil, choices?.isEmpty != false { throw UnsupportedSchema(path: path) }
            return (.string(choices: choices), nullable)
        case "number": return (.number, nullable)
        case "integer": return (.integer, nullable)
        case "boolean": return (.boolean, nullable)
        case "array":
            guard let items = schema["items"] as? [String: Any] else { throw UnsupportedSchema(path: path) }
            let item = try read(items, path: path + "[]")
            guard !item.nullable else { throw UnsupportedSchema(path: path + "[]") }
            return (.array(item.node), nullable)
        case "object":
            let properties = schema["properties"] as? [String: Any] ?? [:]
            let required = Set(schema["required"] as? [String] ?? [])
            // JSONSerialization loses the listed order: the schema text gives it back.
            let order = Self.propertyOrder(schema, names: Array(properties.keys))
            var out: [(String, SchemaNode, Bool, Bool)] = []
            for name in order {
                guard let property = properties[name] as? [String: Any] else {
                    throw UnsupportedSchema(path: path + "." + name)
                }
                let (node, isNull) = try read(property, path: path + "." + name)
                out.append((name, node, !required.contains(name) || isNull, isNull))
            }
            return (.object(out), nullable)
        default:
            throw UnsupportedSchema(path: path)
        }
    }

    /// The properties in a stable order: by name. (JSON objects are unordered; the model is told
    /// the schema, and the core reads fields by name.)
    private static func propertyOrder(_ schema: [String: Any], names: [String]) -> [String] {
        names.sorted()
    }

    /// `answer` (JSON text) with every nullable property this schema lists and the answer leaves
    /// out put back as null, at every depth. An answer that is not JSON comes back as it was: the
    /// core refuses it by its own check.
    public func completed(_ answer: String) -> String {
        guard let value = try? JSONSerialization.jsonObject(with: Data(answer.utf8), options: [.fragmentsAllowed]) else {
            return answer
        }
        let filled = fill(value)
        guard let data = try? JSONSerialization.data(withJSONObject: filled, options: [.sortedKeys, .fragmentsAllowed]) else {
            return answer
        }
        return String(decoding: data, as: UTF8.self)
    }

    private func fill(_ value: Any) -> Any {
        switch self {
        case .object(let properties):
            guard var object = value as? [String: Any] else { return value }
            for property in properties {
                if let present = object[property.name] {
                    if !(present is NSNull) {
                        object[property.name] = property.node.fill(present)
                    }
                } else if property.nullable {
                    object[property.name] = NSNull()
                }
            }
            return object
        case .array(let item):
            guard let array = value as? [Any] else { return value }
            return array.map { item.fill($0) }
        default:
            return value
        }
    }
}

// Availability, checked against the macOS 27 SDK's FoundationModels interface: every API below
// (GenerationSchema(root:dependencies:), DynamicGenerationSchema's name/properties, anyOf strings,
// arrayOf and type initializers, DynamicGenerationSchema.Property) and the session's
// respond(to:schema:includeSchemaInPrompt:options:) with GeneratedContent.jsonString (in
// FoundationModelsPolish) are macOS 26.0 API, in the 26.0 SDK, so `canImport` is their only gate.
// The 26.4 additions (DynamicGenerationSchema.null, representNilExplicitlyInGeneratedContent) and
// the macOS 27 respond overload (contextOptions, metadata) are not used.
#if canImport(FoundationModels)
    extension SchemaNode {
        /// The schema Foundation Models generates to.
        func generationSchema() throws -> GenerationSchema {
            try GenerationSchema(root: dynamic(name: "Answer"), dependencies: [])
        }

        /// This node as a dynamic schema; `name` names an object (unique by its path).
        func dynamic(name: String) -> DynamicGenerationSchema {
            switch self {
            case .string(let choices?):
                return DynamicGenerationSchema(name: name, anyOf: choices)
            case .string(nil):
                return DynamicGenerationSchema(type: String.self)
            case .number:
                return DynamicGenerationSchema(type: Double.self)
            case .integer:
                return DynamicGenerationSchema(type: Int.self)
            case .boolean:
                return DynamicGenerationSchema(type: Bool.self)
            case .array(let item):
                return DynamicGenerationSchema(arrayOf: item.dynamic(name: name + "Item"))
            case .object(let properties):
                return DynamicGenerationSchema(
                    name: name,
                    properties: properties.map { property in
                        DynamicGenerationSchema.Property(
                            name: property.name,
                            schema: property.node.dynamic(name: name + "_" + property.name),
                            isOptional: property.optional)
                    })
            }
        }
    }
#endif
