// Parakeet's model holder: loaded once however many ask, a failed load reported (never a
// download) and retried, and decodes taken one at a time.
@testable import AppleEngines
import InkBridge
import Synchronization
import XCTest

/// A backend that says how many decodes ran at once.
private final class Overlap: ParakeetBackend {
    private let state = Mutex((now: 0, most: 0))
    var most: Int { state.withLock { $0.most } }

    func transcribe(_ samples: [Float]) async throws(ParakeetError) -> Transcribed {
        state.withLock { $0.now += 1; $0.most = max($0.most, $0.now) }
        try? await Task.sleep(for: .milliseconds(20))
        state.withLock { $0.now -= 1 }
        return Transcribed(text: "", words: [], reportedDuration: 0)
    }
}

/// A backend whose first decode never returns, cancelled or not, as a wedged Core ML call would,
/// until the test lets it go; every later decode answers at once.
private final class Wedged: ParakeetBackend {
    private let state = Mutex<(calls: Int, parked: CheckedContinuation<Void, Never>?)>((0, nil))
    var calls: Int { state.withLock { $0.calls } }

    func transcribe(_ samples: [Float]) async throws(ParakeetError) -> Transcribed {
        let first = state.withLock { s in
            s.calls += 1
            return s.calls == 1
        }
        if first {
            await withCheckedContinuation { c in state.withLock { $0.parked = c } }
            return Transcribed(text: "late", words: [], reportedDuration: 0)
        }
        return Transcribed(text: "answered", words: [], reportedDuration: 0)
    }

    /// Lets the wedged decode return at last.
    func release() {
        state.withLock { s in
            defer { s.parked = nil }
            return s.parked
        }?.resume()
    }
}

final class ParakeetModelTests: XCTestCase {
    /// The earlier engine reloaded the models per file, which made a 330-file run unusable: Core
    /// ML compiles for seconds. Here every caller shares one load, including callers that arrive
    /// while it runs.
    func testModelsAreLoadedOnceWhoeverAsks() async throws {
        let backend = Overlap()
        let model = ParakeetModel(loader: {
            try? await Task.sleep(for: .milliseconds(100))
            return backend
        })
        try await withThrowingTaskGroup(of: Void.self) { group in
            for i in 0..<12 {
                group.addTask {
                    if i.isMultiple(of: 2) {
                        try await model.load()
                    } else {
                        _ = try await model.decode([Float](repeating: 0, count: 8_000))
                    }
                }
            }
            try await group.waitForAll()
        }
        let loads = await model.loads
        XCTAssertEqual(loads, 1)
    }

    func testDecodesTakeTurns() async throws {
        let backend = Overlap()
        let model = ParakeetModel(loader: { backend })
        await withTaskGroup(of: Void.self) { group in
            for _ in 0..<8 {
                group.addTask { _ = try? await model.decode([0]) }
            }
        }
        XCTAssertEqual(backend.most, 1, "the Neural Engine takes one decode at a time")
    }

    /// One decode that never returned used to keep the Neural Engine's turn forever: every stream
    /// and the offline fallback waited behind it, while every call into the engine still answered.
    func testAHungDecodeTimesOutAndTheNextCallerStillDecodes() async throws {
        let backend = Wedged()
        let model = ParakeetModel(loader: { backend }, decodeLimit: { _ in .milliseconds(200) })
        let audio = [Float](repeating: 0, count: 8_000)
        let hung = Task { () -> ParakeetError? in
            do throws(ParakeetError) {
                _ = try await model.transcribe(audio)
                return nil
            } catch {
                return error
            }
        }
        let until = Date().addingTimeInterval(5)
        while backend.calls == 0 {
            XCTAssertLessThan(Date(), until, "the first decode never started")
            try await Task.sleep(for: .milliseconds(5))
        }
        let started = ContinuousClock.now
        let second = try await model.transcribe(audio)
        XCTAssertEqual(second.text, "answered", "the second caller got the turn and its decode")
        XCTAssertLessThan(ContinuousClock.now - started, .seconds(3))
        let failure = await hung.value
        XCTAssertEqual(failure, .decodeTimedOut, "the hung decode reports a failure, not text")
        XCTAssertEqual(ParakeetError.decodeTimedOut.engineError, .failed(code: 203))
        backend.release()
    }

    /// The limit grows with the audio from a floor: a live window of at most 30 s gets under 30 s,
    /// and a long offline take gets time in proportion.
    func testTheDecodeLimitGrowsWithTheAudioFromAFloor() {
        XCTAssertEqual(ParakeetModel.decodeLimit(samples: 0), .seconds(20))
        XCTAssertEqual(ParakeetModel.decodeLimit(samples: LiveWindowConfig().maxBuffer), .seconds(27.5))
        XCTAssertEqual(ParakeetModel.decodeLimit(samples: 3_600 * 16_000), .seconds(920))
    }

    /// The offline engine answers a buffer too short for FluidAudio with no words, without loading
    /// or decoding; its guard is its own constant, not the live window's.
    func testTheOfflineEngineAnswersATooShortBufferEmptyWithoutADecode() throws {
        let loads = Counter()
        let model = ParakeetModel(loader: {
            loads.add()
            return Overlap()
        })
        let engine = ParakeetOfflineEngine(model: model)
        XCTAssertEqual(ParakeetOfflineEngine.minimumSamples, 4_800, "0.3 s")
        let short = Signal<Result<[InkSegment], InkEngineError>>()
        engine.transcribe(
            [Float](repeating: 0, count: ParakeetOfflineEngine.minimumSamples - 1), channel: .mic,
            context: nil) { short.set($0) }
        XCTAssertEqual(try XCTUnwrap(short.wait()), .success([]))
        XCTAssertEqual(loads.value, 0)
        let long = Signal<Result<[InkSegment], InkEngineError>>()
        engine.transcribe(
            [Float](repeating: 0, count: ParakeetOfflineEngine.minimumSamples), channel: .mic,
            context: nil) { long.set($0) }
        XCTAssertEqual(try XCTUnwrap(long.wait()), .success([]))
        XCTAssertEqual(loads.value, 1, "long enough: decoded")
    }

    func testAFailedLoadIsReportedAndCanBeRetried() async throws {
        let attempts = Counter()
        let model = ParakeetModel(loader: { () throws(ParakeetError) -> any ParakeetBackend in
            attempts.add()
            if attempts.value == 1 { throw .modelMissing }
            return Overlap()
        })
        do {
            try await model.load()
            XCTFail("the first load fails")
        } catch {
            XCTAssertEqual(error, .modelMissing)
        }
        try await model.load()
        try await model.load()
        XCTAssertEqual(attempts.value, 2)
    }
}
