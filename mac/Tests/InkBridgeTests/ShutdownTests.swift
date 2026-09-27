// Shutting the session down from several places at once: every caller returns only once the core
// has stopped.
import Foundation
import InkBridge
import InkCore
import Synchronization
import XCTest

/// An engine whose release takes a while: it runs inside ink_shutdown, so shutdown is slow enough
/// for a second caller to arrive while the first is still inside it.
final class SlowToRelease: InkOfflineEngine {
    let id = "slow-release"
    let licence = "MIT"
    let jobs: [(job: Job, wer: Double)] = [(.meetingFinal, 1.0)]

    func transcribe(
        _ samples: [Float], channel: Channel, context: String?,
        completion: @escaping @Sendable (Result<[InkSegment], InkEngineError>) -> Void
    ) {
        completion(.success([]))
    }

    deinit {
        Thread.sleep(forTimeInterval: 0.15)
    }
}

final class ShutdownTests: XCTestCase {
    private func start(_ events: Events) throws -> (InkSession, URL) {
        let data = FileManager.default.temporaryDirectory
            .appendingPathComponent("inkwell-shutdown-\(UUID().uuidString)")
        let session = try InkSession.start(
            InkConfig(dataDir: data.path, logLevel: "warn"), onEvent: { events.record($0) })
        XCTAssertNotNil(events.wait(5) { if case .coreReady = $0 { true } else { nil } })
        try session.register(SlowToRelease())
        return (session, data)
    }

    // Static, so closures on other threads call it without capturing the (non-Sendable) test case.
    private static func stopped(_ events: Events) -> Bool {
        if case .coreStopped = events.all.last { true } else { false }
    }

    func testConcurrentShutdownsAllReturnOnlyOnceTheCoreHasStopped() throws {
        let events = Events()
        let (session, data) = try start(events)
        defer { try? FileManager.default.removeItem(at: data) }
        let sawStopped = Mutex<[Bool]>([])
        DispatchQueue.concurrentPerform(iterations: 6) { _ in
            session.shutdown()
            let done = Self.stopped(events)
            sawStopped.withLock { $0.append(done) }
        }
        XCTAssertEqual(sawStopped.withLock { $0 }, Array(repeating: true, count: 6))
    }

    func testLettingGoOfTheSessionShutsItDownBeforeTheLastReleaseReturns() throws {
        let events = Events()
        var data: URL?
        do {
            let (session, dir) = try start(events)
            data = dir
            // A second holder shuts it down on another thread while this one lets go: whichever
            // is last runs deinit, and every path returns only once the core has stopped.
            let other = session
            let sawStopped = Mutex<Bool?>(nil)
            let done = DispatchSemaphore(value: 0)
            DispatchQueue.global().async {
                other.shutdown()
                sawStopped.withLock { $0 = Self.stopped(events) }
                done.signal()
            }
            done.wait()
            XCTAssertEqual(sawStopped.withLock { $0 }, true)
        }
        XCTAssertTrue(Self.stopped(events))
        if let data { try? FileManager.default.removeItem(at: data) }

        // No explicit shutdown at all: the last release does it, before it returns.
        let quiet = Events()
        var dir2: URL?
        do {
            let (session, dir) = try start(quiet)
            dir2 = dir
            _ = session
        }
        XCTAssertTrue(Self.stopped(quiet), "deinit returned before the core stopped")
        if let dir2 { try? FileManager.default.removeItem(at: dir2) }
    }
}
