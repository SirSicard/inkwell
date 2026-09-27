// The C ABI from Swift, end to end: init → a mock engine registered from Swift → a replay command
// → the record event, with the engine called from the core's worker threads → shutdown.
//
// Needs the core's XCFramework: run mac/scripts/build-core.sh first. Thread Sanitizer locally:
// swift test --package-path mac --sanitize=thread
import Foundation
import InkBridge
import InkCore
import Synchronization
import XCTest

/// Every event, in order, as the event thread delivered it.
final class Events: Sendable {
    private let state = Mutex<[(event: InkEvent, thread: String)]>([])

    func record(_ event: InkEvent) {
        let thread = currentThreadName()
        state.withLock { $0.append((event, thread)) }
    }

    var all: [InkEvent] { state.withLock { $0.map(\.event) } }
    var threads: [String] { state.withLock { $0.map(\.thread) } }

    /// Waits for the first event `pick` accepts; looks at least once.
    func wait<T>(_ timeout: TimeInterval, _ pick: (InkEvent) -> T?) -> T? {
        let until = Date().addingTimeInterval(timeout)
        while true {
            if let found = all.lazy.compactMap(pick).first {
                return found
            }
            if Date() >= until {
                return nil
            }
            Thread.sleep(forTimeInterval: 0.01)
        }
    }
}

/// The pthread name of the calling thread: the core names its threads ("ink-events",
/// "ink-meeting", ...).
func currentThreadName() -> String {
    var buffer = [CChar](repeating: 0, count: 64)
    pthread_getname_np(pthread_self(), &buffer, buffer.count)
    let bytes = buffer.prefix { $0 != 0 }.map { UInt8(bitPattern: $0) }
    return String(decoding: bytes, as: UTF8.self)
}

/// An engine answering asynchronously, from a queue of its own, as a Core ML engine would.
final class MockEngine: InkOfflineEngine {
    let id = "swift-mock"
    let licence = "MIT"
    let jobs: [(job: Job, wer: Double)] = [(.meetingFinal, 1.0)]

    private let calls = Mutex<[(thread: String, isMain: Bool, samples: Int)]>([])
    private let queue = DispatchQueue(label: "swift-mock-engine", attributes: .concurrent)

    var callLog: [(thread: String, isMain: Bool, samples: Int)] { calls.withLock { $0 } }

    func transcribe(
        _ samples: [Float],
        channel: Channel,
        context: String?,
        completion: @escaping @Sendable (Result<[InkSegment], InkEngineError>) -> Void
    ) {
        let entry = (thread: currentThreadName(), isMain: Thread.isMainThread, samples: samples.count)
        calls.withLock { $0.append(entry) }
        let endMs = UInt64(samples.count / 16)
        queue.async {
            completion(.success([InkSegment(startMs: 0, endMs: endMs, text: "heard from swift")]))
        }
    }
}

final class SmokeTests: XCTestCase {
    func testInitReplayRecordSwiftEngineShutdown() throws {
        let fixtures = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()  // InkBridgeTests
            .deletingLastPathComponent()  // Tests
            .deletingLastPathComponent()  // mac
            .deletingLastPathComponent()  // the repository
            .appendingPathComponent("fixtures/ami")
        let mic = fixtures.appendingPathComponent("IS1009a-mic.wav").path
        let far = fixtures.appendingPathComponent("IS1009a-far.wav").path
        XCTAssertTrue(FileManager.default.fileExists(atPath: mic), mic)
        let data = FileManager.default.temporaryDirectory
            .appendingPathComponent("inkwell-smoke-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: data) }

        // init
        let events = Events()
        let session = try InkSession.start(
            InkConfig(dataDir: data.path, logLevel: "warn", logStderr: true),
            onEvent: { events.record($0) }
        )
        let ready = events.wait(5) { if case .coreReady(let r) = $0 { r } else { nil } }
        XCTAssertEqual(ready?.abi, Int64(INK_ABI_VERSION))

        // a mock external engine, registered from Swift
        let engine = MockEngine()
        try session.register(engine)
        let registered = events.wait(5) { if case .engineRegistered(let e) = $0 { e } else { nil } }
        XCTAssertEqual(registered?.id, "swift-mock")
        XCTAssertThrowsError(try session.register(engine), "an id can be registered once")

        // a replay command → the record event
        try session.command(["cmd": "replay_meeting", "mic": mic, "far": far, "title": "Smoke", "pacing": "fast"])
        let finished = events.wait(120) { if case .meetingFinished(let f) = $0 { f } else { nil } }
        let started = events.wait(0) { if case .meetingStarted(let s) = $0 { s } else { nil } }
        XCTAssertNotNil(finished, "no record: \(events.all)")
        XCTAssertEqual(finished?.record, started?.record)
        XCTAssertEqual(finished?.revision, 2, "the final pass, from the Swift engine, replaced the live one")
        let passes = events.all.compactMap { if case .meetingTranscribed(let t) = $0 { t.pass } else { nil } }
        XCTAssertEqual(passes.count, 2)
        XCTAssertTrue(passes.allSatisfy { $0.wordCount > 0 }, "\(passes)")
        // The whole 30 s fixture went through the pump and onto disk, on both sides.
        XCTAssertTrue(passes.allSatisfy { $0.capturedMs >= 29_900 && $0.regions > 0 }, "\(passes)")
        print("smoke: \(engine.callLog.count) engine calls; passes: \(passes.map { "\($0.channel) \($0.capturedMs) ms, \($0.regions) regions, \($0.wordCount) words" })")

        // ... with the Swift engine called from the core's worker threads, never the main thread
        let calls = engine.callLog
        XCTAssertFalse(calls.isEmpty)
        XCTAssertTrue(calls.allSatisfy { !$0.isMain && $0.thread == "ink-meeting" }, "\(calls.map(\.thread))")
        XCTAssertTrue(calls.allSatisfy { $0.samples > 0 })
        XCTAssertGreaterThan(InkSession.bands().published, 0, "the ink's bands were published")

        // shutdown
        session.shutdown()
        guard case .coreStopped = events.all.last else {
            return XCTFail("core.stopped must be the last event: \(String(describing: events.all.last))")
        }
        let count = events.all.count
        Thread.sleep(forTimeInterval: 0.05)
        XCTAssertEqual(events.all.count, count, "no event after shutdown returned")
        XCTAssertTrue(events.threads.allSatisfy { $0 == "ink-events" }, "\(Set(events.threads))")
        XCTAssertFalse(events.all.contains { if case .unknown = $0 { true } else { false } }, "every event decoded")
        XCTAssertThrowsError(try session.command(["cmd": "model.warm", "job": "dictation_final"]))
    }
}
