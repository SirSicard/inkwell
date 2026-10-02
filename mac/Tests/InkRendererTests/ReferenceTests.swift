// Checks that need something CI does not have, each skipped unless its variable is set:
//
//   INK_GPU_TIMING=1   GPU time per frame at each live canvas size, paced at 60 fps: under 2 ms.
//
// (The comparison with the old ink prototype's WebGL renders went with that ink: the Glow orb has
// no reference renders of its own yet.)
import Foundation
import Metal
import XCTest

@testable import InkRenderer

final class ReferenceTests: XCTestCase {
    private let environment = ProcessInfo.processInfo.environment

    /// GPU time per frame at the sizes the app draws, paced like the display link: the Drop's orb
    /// (58 pt at 2x), and the main window's (1,040 x 700 pt, capped at 1.25x).
    func testGPUTimePerFrameAt60FPS() throws {
        guard environment["INK_GPU_TIMING"] == "1" else { throw XCTSkip("INK_GPU_TIMING is not 1") }
        try XCTSkipUnless(InkRenderer.isSupported, "no Metal device")
        let pipeline = try InkPipelineLoader.shared.wait().get()
        for (name, w, h, placement) in [
            ("drop", 116, 116, OrbPlacement(x: 0.5, yFromTop: 0.5, unit: 1.15)),
            ("window", 1300, 875, OrbPlacement(x: 0.56, yFromTop: 0.26, unit: 0.72)),
        ] {
            let descriptor = MTLTextureDescriptor.texture2DDescriptor(
                pixelFormat: InkPipeline.pixelFormat, width: w, height: h, mipmapped: false)
            descriptor.usage = [.renderTarget]
            descriptor.storageMode = .private
            let target = try XCTUnwrap(pipeline.device.makeTexture(descriptor: descriptor))
            var sim = InkSimulation(random: .seeded(1))
            sim.state = .meeting
            sim.canvasWidth = Double(w)
            sim.canvasHeight = Double(h)
            sim.t = 12
            var times: [Double] = []
            for frame in 0..<330 {
                sim.step(1.0 / 60, snap: false, voice: .synthetic)
                let commandBuffer = try XCTUnwrap(pipeline.queue.makeCommandBuffer())
                pipeline.encode(into: target, commandBuffer: commandBuffer,
                                uniforms: sim.uniforms(palette: .neutral, placement: placement, motion: true))
                commandBuffer.commit()
                commandBuffer.waitUntilCompleted()
                // The first 30 frames warm the pipeline and the clocks.
                if frame >= 30 { times.append((commandBuffer.gpuEndTime - commandBuffer.gpuStartTime) * 1000) }
                Thread.sleep(forTimeInterval: 1.0 / 60)
            }
            times.sort()
            let p50 = times[times.count / 2], p95 = times[Int(Double(times.count - 1) * 0.95)]
            print(String(format: "gpu %@ %dx%d: p50 %.3f ms, p95 %.3f ms, max %.3f ms over %d frames",
                         name, w, h, p50, p95, times.last ?? 0, times.count))
            XCTAssertLessThan(p95, 2, "\(name): GPU time per frame at 60 fps")
        }
    }
}
