// Polish on Foundation Models: an unavailable model is a state, never an answer; a cancelled call
// ends; errors are codes. The live call runs only with INK_APPLE_INTELLIGENCE=1 (it needs Apple
// Intelligence on, which CI runners never have).
@testable import AppleEngines
import Foundation
import InkBridge
import Synchronization
import XCTest
#if canImport(FoundationModels)
    import FoundationModels
#endif

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
        #if canImport(FoundationModels)
            // Every case the SDK knows has its own code; one it adds later is 19.
            let context = LanguageModelSession.GenerationError.Context(debugDescription: "synthetic words")
            XCTAssertEqual(
                FoundationModelsPolish.engineError(LanguageModelSession.GenerationError.decodingFailure(context)),
                .failed(code: 16))
            XCTAssertEqual(
                FoundationModelsPolish.engineError(LanguageModelSession.GenerationError.unsupportedGuide(context)),
                .failed(code: 17))
        #endif
    }

    /// A fake system model: what was prewarmed, and which requests found a prepared session.
    private final class Recorder: PolishBackend {
        let prewarmed = Mutex<[String?]>([])
        let answered = Mutex<[Bool]>([])  // per request: whether a prepared session answered
        private let prepared = Mutex<String??>(nil)

        func prewarm(instructions: String?) {
            prewarmed.withLock { $0.append(instructions) }
            prepared.withLock { $0 = .some(instructions) }
        }

        func respond(_ request: InkLlmRequest) async throws -> String {
            let used = prepared.withLock { p -> Bool in
                defer { p = nil }
                return p == .some(request.system)
            }
            answered.withLock { $0.append(used) }
            return "ok"
        }
    }

    func testPrewarmNeverRunsWhileAppleIntelligenceIsUnavailable() {
        let backend = Recorder()
        let model = FoundationModelsPolish(availability: { .unavailable(.notEnabled) }, backend: backend)
        model.prewarm()
        XCTAssertEqual(backend.prewarmed.withLock { $0.count }, 0)
    }

    /// A prewarm at the start of a dictation prepares a session for the instructions the last
    /// polish used, and the next polish with those instructions answers on it.
    func testAPrewarmedSessionAnswersTheNextPolish() throws {
        let backend = Recorder()
        let model = FoundationModelsPolish(availability: { .available }, backend: backend)
        model.prewarm()  // nothing polished yet: the model loads, no instructions known
        XCTAssertEqual(try generate(model)?.get(), "ok")
        model.prewarm()  // now the last instructions are known
        XCTAssertEqual(try generate(model)?.get(), "ok")
        var other = request
        other.system = "Make it formal."
        model.prewarm()
        XCTAssertEqual(try generate(model, other)?.get(), "ok", "other instructions: a fresh session")
        XCTAssertEqual(backend.prewarmed.withLock { $0 }, [nil, request.system, request.system])
        XCTAssertEqual(backend.answered.withLock { $0 }, [false, true, false])
    }

    /// The engines prewarm polish when a dictation starts, and only then.
    func testADictationStartPrewarmsTheRegisteredPolish() throws {
        let core = try TestCore.start()
        defer { core.stop() }
        let backend = Recorder()
        let engines = AppleEngines(session: core.session, polish: FoundationModelsPolish(availability: { .available }, backend: backend))
        XCTAssertEqual(engines.syncPolish(availability: .available), .registered)
        let started = try InkEvent.decode(Data(#"{"type":"dictation.started"}"#.utf8))
        engines.handle(try InkEvent.decode(Data(#"{"type":"core.ready","abi":2,"version":"0"}"#.utf8)))
        XCTAssertEqual(backend.prewarmed.withLock { $0.count }, 0)
        engines.handle(started)
        XCTAssertEqual(backend.prewarmed.withLock { $0.count }, 1)
        // Unavailable since: not registered, nothing prewarmed.
        engines.syncPolish(availability: .unavailable(.notEnabled))
        engines.handle(started)
        XCTAssertEqual(backend.prewarmed.withLock { $0.count }, 1)
    }

    /// The first polish of a fresh process, cold or after a prewarm and a second of dictation,
    /// then a second polish for reference. One variant per process, so the model is cold at the
    /// start: INK_POLISH_PREWARM=0 or 1 (with INK_APPLE_INTELLIGENCE=1).
    func testFirstPolishColdAgainstPrewarmed() throws {
        let env = ProcessInfo.processInfo.environment
        guard env["INK_APPLE_INTELLIGENCE"] == "1", let variant = env["INK_POLISH_PREWARM"] else {
            throw XCTSkip("set INK_APPLE_INTELLIGENCE=1 and INK_POLISH_PREWARM=0 or 1")
        }
        guard case .available = AppleIntelligence.current() else { throw XCTSkip("Apple Intelligence is off") }
        let model = FoundationModelsPolish()
        if variant == "1" {
            model.prewarm()
            Thread.sleep(forTimeInterval: 1.0)  // the user speaks
        }
        var took: [Duration] = []
        for _ in 0..<2 {
            let started = ContinuousClock.now
            XCTAssertNoThrow(try XCTUnwrap(generate(model)).get())
            took.append(ContinuousClock.now - started)
        }
        print("polish \(variant == "1" ? "prewarmed" : "cold"): first \(took[0]), second \(took[1])")
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
