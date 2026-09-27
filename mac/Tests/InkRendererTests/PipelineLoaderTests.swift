// The shader compiles once, on a background queue, from the moment the app starts; no ink view
// ever waits for it on the main thread. Until it is ready a view shows plain paper.
import AppKit
import Synchronization
import XCTest

@testable import InkRenderer

final class PipelineLoaderTests: XCTestCase {
    func testWarmCompilesOnceOffTheMainThreadAndEveryWaiterGetsThatPipeline() throws {
        try XCTSkipUnless(InkRenderer.isSupported, "no Metal device")
        let calls = Mutex(0), onMain = Mutex<Bool?>(nil)
        let loader = InkPipelineLoader {
            calls.withLock { $0 += 1 }
            onMain.withLock { $0 = Thread.isMainThread }
            return try InkPipeline()
        }
        XCTAssertNil(loader.outcome, "nothing before warm")
        loader.warm()
        loader.warm()
        let first = try loader.wait().get(), second = try loader.wait().get()
        XCTAssertTrue(first === second)
        XCTAssertEqual(calls.withLock { $0 }, 1, "one compile, however often it is asked for")
        XCTAssertEqual(onMain.withLock { $0 }, false, "compiled off the main thread")
        XCTAssertNotNil(loader.outcome)
        XCTAssertGreaterThan(try XCTUnwrap(loader.compileDuration), .zero)
    }

    @MainActor
    func testAnInkViewMadeBeforeTheCompileFinishesDoesNotWaitForIt() async throws {
        try XCTSkipUnless(InkRenderer.isSupported, "no Metal device")
        let gate = DispatchSemaphore(value: 0)
        let loader = InkPipelineLoader {
            // Held until the view exists: the compile is still running when it is made.
            gate.wait()
            return try InkPipeline()
        }
        let clock = ContinuousClock()
        let start = clock.now
        let view = InkView(frame: NSRect(x: 0, y: 0, width: 84, height: 84), loader: loader)
        view.assumeReduceMotion = false  // GitHub's macOS runners turn Reduce Motion on
        view.assumeOnScreen = true
        view.state = .meeting
        let elapsed = clock.now - start
        XCTAssertLessThan(elapsed, .milliseconds(100), "the view is made at once")
        XCTAssertFalse(view.isReady, "and shows paper until the compile finishes")
        XCTAssertFalse(view.isAnimating, "no clock without a pipeline")
        XCTAssertEqual(view.framesDrawn, 0)

        gate.signal()
        for _ in 0..<500 where !view.isReady {
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTAssertTrue(view.isReady, "the pipeline arrives on the main thread")
        XCTAssertNil(view.failure)
        XCTAssertTrue(view.isAnimating, "and the live ink starts then")
        view.state = .idle
        XCTAssertFalse(view.isAnimating)
    }

    @MainActor
    func testAFailedCompileLeavesPaperAndSaysWhy() async throws {
        let loader = InkPipelineLoader { throw InkRendererError.shaderMissing }
        let view = InkView(frame: NSRect(x: 0, y: 0, width: 84, height: 84), loader: loader)
        for _ in 0..<500 where view.failure == nil {
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTAssertEqual(view.failure, .shaderMissing)
        XCTAssertFalse(view.isReady)
        view.assumeOnScreen = true
        view.state = .dictating
        XCTAssertFalse(view.isAnimating, "a view that cannot draw never runs a clock")
    }

    @MainActor
    func testAViewMadeAfterTheCompileIsReadyAtOnce() throws {
        try XCTSkipUnless(InkRenderer.isSupported, "no Metal device")
        let loader = InkPipelineLoader { try InkPipeline() }
        _ = loader.wait()
        XCTAssertTrue(InkView(loader: loader).isReady)
    }

    /// The compile's cost, for the report: a first compile of new source (Metal's shader cache
    /// cannot help) and the shared loader's compile of the bundled source.
    func testTheCompileTime() throws {
        try XCTSkipUnless(InkRenderer.isSupported, "no Metal device")
        let source = try XCTUnwrap(InkShaderSource.load())
        let clock = ContinuousClock()
        let start = clock.now
        _ = try InkPipeline(source: source + "\n// \(UUID().uuidString)\n")
        let cold = clock.now - start
        _ = try InkPipelineLoader.shared.wait().get()
        let shared = try XCTUnwrap(InkPipelineLoader.shared.compileDuration)
        print("ink pipeline compile: new source \(cold.formatted(.units(allowed: [.milliseconds]))), "
              + "the shared loader \(shared.formatted(.units(allowed: [.milliseconds])))")
        XCTAssertGreaterThan(cold, .zero)
    }
}
