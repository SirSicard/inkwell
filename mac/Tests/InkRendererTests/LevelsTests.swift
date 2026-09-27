// What the ink reads besides its state: the live levels from the core's bands, and the GPU time
// per frame a measurement asks for.
import Metal
import XCTest

@testable import InkRenderer

final class LevelsTests: XCTestCase {
    /// A band's value is its RMS amplitude, linear full scale.
    private func rms(_ dbfs: Double) -> Float { Float(pow(10, dbfs / 20)) }

    func testSilenceAndRoomNoiseReadZeroAndLoudSpeechReadsOne() {
        XCTAssertEqual(InkLevels.level(low: 0, mid: 0, high: 0), 0)
        XCTAssertEqual(InkLevels.level(low: rms(-70), mid: 0, high: 0), 0, "below the floor")
        XCTAssertEqual(InkLevels.level(low: 0, mid: rms(-10), high: 0), 1, "above the ceiling")
        XCTAssertEqual(InkLevels.level(low: 0, mid: rms(-40), high: 0), 0.5, accuracy: 1e-6, "halfway in dB")
    }

    func testTheBandsPowersAdd() {
        // Two equal bands carry twice the power: +3 dB.
        let one = InkLevels.level(low: rms(-40), mid: 0, high: 0)
        let two = InkLevels.level(low: rms(-40), mid: rms(-40), high: 0)
        XCTAssertEqual((two - one) * (InkLevels.ceilingDB - InkLevels.floorDB), 10 * log10(2), accuracy: 1e-4)
    }

    func testNonsenseReadsZero() {
        XCTAssertEqual(InkLevels.level(low: .nan, mid: 0, high: 0), 0)
        XCTAssertEqual(InkLevels.level(low: .infinity, mid: 0, high: 0), 0)
    }

    func testGPUTimesAreRecordedOnlyOnceStarted() throws {
        try XCTSkipUnless(InkRenderer.isSupported, "no Metal device")
        let pipeline = try InkPipeline.shared.get()
        let times = GPUFrameTimes()
        try frame(pipeline, observedBy: times)
        XCTAssertNil(times.drain(), "not recording")
        times.start()
        try frame(pipeline, observedBy: times)
        try frame(pipeline, observedBy: times)
        let spread = try XCTUnwrap(times.drain())
        XCTAssertEqual(spread.frames, 2)
        XCTAssertGreaterThan(spread.p50, 0)
        XCTAssertNil(times.drain(), "a drain forgets what it read")
    }

    func testTheSpreadFromManyThreads() {
        let times = GPUFrameTimes()
        times.start()
        DispatchQueue.concurrentPerform(iterations: 1000) { times.record(Double($0)) }
        let spread = times.drain()
        XCTAssertEqual(spread?.frames, 1000)
        XCTAssertEqual(spread?.p50, 499)
        XCTAssertEqual(spread?.p95, 949)
        XCTAssertEqual(spread?.max, 999)
    }

    /// One frame offscreen, observed, and waited for past its completion handlers (they run in
    /// the order they were added, so the last one runs after the observer's).
    private func frame(_ pipeline: InkPipeline, observedBy times: GPUFrameTimes) throws {
        let descriptor = MTLTextureDescriptor.texture2DDescriptor(
            pixelFormat: InkPipeline.pixelFormat, width: 64, height: 64, mipmapped: false)
        descriptor.usage = [.renderTarget]
        descriptor.storageMode = .private
        let target = try XCTUnwrap(pipeline.device.makeTexture(descriptor: descriptor))
        let commandBuffer = try XCTUnwrap(pipeline.queue.makeCommandBuffer())
        var sim = InkSimulation()
        sim.canvasWidth = 64
        sim.canvasHeight = 64
        pipeline.encode(into: target, commandBuffer: commandBuffer, uniforms: sim.uniforms(hasMark: false), mark: nil)
        times.observe(commandBuffer)
        let done = expectation(description: "completed")
        commandBuffer.addCompletedHandler { _ in done.fulfill() }
        commandBuffer.commit()
        wait(for: [done], timeout: 5)
    }
}
