// Polish on Foundation Models: an unavailable model is a state, never an answer; a cancelled call
// ends; errors are codes. The live call runs only with INK_APPLE_INTELLIGENCE=1 (it needs Apple
// Intelligence on, which CI runners never have).
@testable import AppleEngines
import Foundation
import InkBridge
import XCTest

private let request = InkLlmRequest(
    system: "Clean up this dictation. Return only the text.",
    user: "so um the the report is due on friday",
    maxTokens: 256, temperature: 0.3)

private func generate(
    _ model: FoundationModelsPolish, _ request: InkLlmRequest = request,
    cancellation: InkCancellation = InkCancellation()
) -> Result<String, InkEngineError>? {
    let answer = Signal<Result<String, InkEngineError>>()
    model.generate(request, cancellation: cancellation) { answer.set($0) }
    return answer.wait(60)
}

final class FoundationModelsPolishTests: XCTestCase {
    func testUnavailableIsAStateNeverAnAnswer() {
        for reason in [AppleIntelligence.Reason.deviceNotEligible, .notEnabled, .modelNotReady, .unsupported] {
            let asked = Counter()
            let model = FoundationModelsPolish(
                availability: { .unavailable(reason) },
                respond: { _ in asked.add(); return "made up" })
            XCTAssertEqual(generate(model)?.failureValue, .unavailable(code: reason.rawValue))
            XCTAssertEqual(asked.value, 0, "the model is never asked")
        }
    }

    func testAnAvailableModelAnswersWithItsText() {
        let model = FoundationModelsPolish(availability: { .available }, respond: { r in "Polished: \(r.user.count)" })
        XCTAssertEqual(try generate(model)?.get(), "Polished: \(request.user.count)")
        XCTAssertTrue(model.isLocal, "on-device: local-only mode keeps it")
    }

    func testACancelledCallEndsAsCancelled() {
        let model = FoundationModelsPolish(availability: { .available }, respond: { _ in
            try await Task.sleep(for: .seconds(30))
            return "too late"
        })
        let cancellation = InkCancellation()
        let answer = Signal<Result<String, InkEngineError>>()
        model.generate(request, cancellation: cancellation) { answer.set($0) }
        cancellation.cancel()
        XCTAssertEqual(answer.wait(5)?.failureValue, .cancelled)
    }

    func testAStructuredRequestIsRefusedNotAnsweredLoosely() {
        let asked = Counter()
        let model = FoundationModelsPolish(availability: { .available }, respond: { _ in asked.add(); return "{}" })
        var structured = request
        structured.jsonSchema = "{\"type\":\"object\"}"
        XCTAssertEqual(generate(model, structured)?.failureValue, .badRequest)
        XCTAssertEqual(asked.value, 0)
    }

    func testTheModelsErrorsBecomeCodesWithoutText() {
        struct Quoting: Error, CustomStringConvertible { var description: String { "heard: synthetic words" } }
        let model = FoundationModelsPolish(availability: { .available }, respond: { _ in throw Quoting() })
        XCTAssertEqual(generate(model)?.failureValue, .failed(code: 1))
    }

    /// The real model, on this Mac. Reports the availability either way.
    func testTheOnDeviceModelPolishesWhenAppleIntelligenceIsOn() throws {
        guard ProcessInfo.processInfo.environment["INK_APPLE_INTELLIGENCE"] == "1" else {
            throw XCTSkip("set INK_APPLE_INTELLIGENCE=1 to call the on-device model")
        }
        let availability = AppleIntelligence.current()
        print("Apple Intelligence on this Mac: \(availability)")
        let model = FoundationModelsPolish()
        let started = ContinuousClock.now
        let result = try XCTUnwrap(generate(model))
        let took = ContinuousClock.now - started
        switch availability {
        case .available:
            let text = try result.get()
            XCTAssertFalse(text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            print("polished in \(took); \(text.count) characters")
        case .unavailable(let reason):
            XCTAssertEqual(result.failureValue, .unavailable(code: reason.rawValue))
        }
    }
}
