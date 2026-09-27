// Parakeet's model holder: loaded once however many ask, a failed load reported (never a
// download) and retried, and decodes taken one at a time.
@testable import AppleEngines
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
