// Structured answers on Foundation Models (S2.8): the core's schemas (a meeting's summary, the
// commitment judge, "said twice") become a generation schema the model is held to, and an answer
// that leaves a nullable field out gets it back as null, as the core's parsers want.
//
// The three schemas are copies of the core's (ink-llm: SUMMARY_SCHEMA, JUDGE_SCHEMA,
// DEDUP_SCHEMA): keep them in step when those change.
@testable import AppleEngines
import Foundation
import InkBridge
import XCTest
#if canImport(FoundationModels)
    import FoundationModels
#endif

private let judge = #"""
{"type": "object", "properties": {
  "class": {"type": "string", "enum": ["commitment", "hypothetical", "option_discussion", "declined_or_retracted", "delegated", "already_done"]},
  "confidence": {"type": "number", "minimum": 0, "maximum": 1},
  "task": {"type": ["string", "null"]}, "to": {"type": ["string", "null"]},
  "due": {"type": ["string", "null"]}, "quote": {"type": "string"}},
 "required": ["class", "confidence", "task", "due", "quote"], "additionalProperties": false}
"""#

private let summary = #"""
{"type": "object", "properties": {
  "headline": {"type": "string"}, "body": {"type": "string"},
  "decisions": {"type": "array", "items": {"type": "object", "properties": {
    "text": {"type": "string"}, "line": {"type": ["integer", "null"]},
    "quote": {"type": ["string", "null"]}}, "required": ["text", "line", "quote"]}},
  "actions": {"type": "array", "items": {"type": "object", "properties": {
    "text": {"type": "string"}, "owner": {"type": ["string", "null"]}, "to": {"type": ["string", "null"]},
    "due": {"type": ["string", "null"]}, "line": {"type": ["integer", "null"]},
    "quote": {"type": ["string", "null"]}}, "required": ["text", "owner", "to", "due", "line", "quote"]}},
  "open_questions": {"type": "array", "items": {"type": "string"}},
  "not_found": {"type": "array", "items": {"type": "string"}}},
 "required": ["headline", "body", "decisions", "actions"]}
"""#

private let dedup = #"""
{"type": "object", "properties": {"same": {"type": "boolean"}, "keep": {"type": "string", "enum": ["A", "B"]},
 "why": {"type": "string"}}, "required": ["same", "keep"], "additionalProperties": false}
"""#

private func object(_ json: String) -> [String: Any] {
    (try? JSONSerialization.jsonObject(with: Data(json.utf8))) as? [String: Any] ?? [:]
}

final class StructuredAnswerTests: XCTestCase {
    func testTheCoresSchemasAreRead() throws {
        guard case .object(let fields) = try SchemaNode.parse(judge) else { return XCTFail("an object") }
        let byName = Dictionary(uniqueKeysWithValues: fields.map { ($0.name, $0) })
        XCTAssertEqual(byName["class"]?.node, .string(choices: [
            "commitment", "hypothetical", "option_discussion", "declined_or_retracted", "delegated", "already_done",
        ]))
        XCTAssertEqual(byName["confidence"]?.node, .number)
        XCTAssertEqual(byName["task"]?.nullable, true)
        XCTAssertEqual(byName["to"]?.optional, true, "not required")
        XCTAssertEqual(byName["quote"]?.optional, false)
        XCTAssertNoThrow(try SchemaNode.parse(summary))
        XCTAssertNoThrow(try SchemaNode.parse(dedup))
    }

    func testASchemaOutsideTheSubsetIsRefused() {
        for bad in [
            #"{"type": "object", "properties": {"x": {"oneOf": []}}}"#,
            #"{"type": ["string", "integer"]}"#,
            #"{"type": "array"}"#,
            #"{"type": "string", "enum": []}"#,
            "not json",
        ] {
            XCTAssertThrowsError(try SchemaNode.parse(bad), bad)
        }
    }

    func testMissingNullableFieldsComeBackAsNull() throws {
        let node = try SchemaNode.parse(summary)
        let answer = #"{"headline":"Planned it.","body":"b","decisions":[{"text":"Ship"}],"actions":[{"text":"Send","owner":"You"}]}"#
        let filled = object(node.completed(answer))
        let decision = try XCTUnwrap((filled["decisions"] as? [[String: Any]])?.first)
        XCTAssertTrue(decision["line"] is NSNull && decision["quote"] is NSNull)
        let action = try XCTUnwrap((filled["actions"] as? [[String: Any]])?.first)
        XCTAssertEqual(action["owner"] as? String, "You", "what was there stays")
        XCTAssertTrue(action["to"] is NSNull && action["due"] is NSNull)
        XCTAssertNil(filled["open_questions"], "a non-nullable optional stays out")
        XCTAssertEqual(node.completed("not json"), "not json", "the core refuses it by its own check")
    }

    #if canImport(FoundationModels)
        /// The generation schema builds for each of the core's schemas: no Apple Intelligence
        /// needed, so it runs on CI too.
        func testEachCoreSchemaBecomesAGenerationSchema() throws {
            for json in [judge, summary, dedup] {
                XCTAssertNoThrow(try SchemaNode.parse(json).generationSchema())
            }
        }
    #endif

    /// A structured request goes to the backend's schema path, and its answer comes back filled.
    func testAStructuredRequestIsGeneratedToItsSchema() throws {
        final class Schemas: PolishBackend {
            func prewarm(instructions: String?) {}
            func respond(_ request: InkLlmRequest) async throws -> String { "plain" }
            func respond(_ request: InkLlmRequest, schema: SchemaNode) async throws -> String {
                #"{"class":"commitment","confidence":0.9,"quote":"I'll send it"}"#
            }
        }
        let model = FoundationModelsPolish(availability: { .available }, backend: Schemas())
        var structured = InkLlmRequest(system: "judge", user: "sentence", maxTokens: 400, temperature: 0)
        structured.jsonSchema = judge
        let answer = Signal<Result<String, InkEngineError>>()
        model.generate(structured, cancellation: InkCancellation()) { answer.set($0) }
        let text = try XCTUnwrap(answer.wait(10)).get()
        let fields = object(text)
        XCTAssertEqual(fields["class"] as? String, "commitment")
        XCTAssertTrue(fields["task"] is NSNull && fields["due"] is NSNull, "nullable fields present as null")
        var unreadable = structured
        unreadable.jsonSchema = ##"{"type": "object", "properties": {"x": {"$ref": "#/y"}}}"##
        let refused = Signal<Result<String, InkEngineError>>()
        model.generate(unreadable, cancellation: InkCancellation()) { refused.set($0) }
        XCTAssertEqual(refused.wait(10)?.failureValue, .badRequest)
    }

    /// Review (S2.8): the context size is read only from an SDK that declares it (26.4 and
    /// later, behind the same gate as the macOS 27 code); elsewhere nil, and the core sizes for
    /// 4,096 tokens. When said, it is a size the core accepts.
    func testTheOnDeviceModelSaysHowMuchContextItHasWhereTheSDKCanTell() {
        let model = FoundationModelsPolish(availability: { .available }, respond: { _ in "" })
        #if compiler(>=6.4) && canImport(FoundationModels, _version: 2.0)
            let tokens = try? XCTUnwrap(model.contextTokens)
            XCTAssertGreaterThanOrEqual(tokens ?? 0, 256)
        #else
            XCTAssertNil(model.contextTokens)
        #endif
    }
}
