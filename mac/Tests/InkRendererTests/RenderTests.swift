// The orb drawn offscreen through the real pipeline: the bundled MSL compiled at run time, as the
// app does. Skipped on a Mac without Metal.
import Metal
import XCTest

@testable import InkRenderer

final class RenderTests: XCTestCase {
    private var pipeline: InkPipeline!

    /// Yours blue, theirs orange: easy to tell apart in a frame.
    private let palette = OrbPalette(
        yA: SIMD3(0.2, 0.3, 1), yB: SIMD3(0.52, 0.58, 1), tA: SIMD3(1, 0.5, 0), tB: SIMD3(1, 0.7, 0.4),
        idle: SIMD3(0.6, 0.6, 0.6), ink: SIMD3(0.1, 0.1, 0.12), dark: false)

    override func setUpWithError() throws {
        try XCTSkipUnless(InkRenderer.isSupported, "no Metal device")
        pipeline = try InkPipelineLoader.shared.wait().get()
    }

    func testTheBundledShaderIsFoundAndCompiles() throws {
        let source = try XCTUnwrap(InkShaderSource.load(), "Resources/ink.msl is in the resource bundle")
        XCTAssertTrue(source.hasPrefix("// Generated from shaders/ink.wgsl"))
        XCTAssertNoThrow(try InkPipeline(source: source))
    }

    func testABrokenShaderIsAnErrorNotACrash() {
        XCTAssertThrowsError(try InkPipeline(source: "this is not MSL")) { error in
            guard case InkRendererError.shaderFailed = error else {
                return XCTFail("expected shaderFailed, got \(error)")
            }
        }
    }

    func testEveryStateRendersTheSameTwiceAndDiffersFromTheOthers() throws {
        var images: [InkState: InkImage] = [:]
        for state in InkState.allCases {
            let first = try render(state)
            let second = try render(state)
            XCTAssertEqual(first.rgba, second.rgba, "\(state): a fixed frame is deterministic")
            images[state] = first
        }
        for a in InkState.allCases {
            for b in InkState.allCases where a.rawValue < b.rawValue {
                XCTAssertNotEqual(images[a]?.rgba, images[b]?.rgba, "\(a) and \(b) look different")
            }
        }
    }

    /// Transparent wherever there is no orb, and premultiplied everywhere: the orb is composited
    /// over the window's content.
    func testTheOrbIsPremultipliedAndTransparentAroundIt() throws {
        for state in InkState.allCases {
            let image = try render(state)
            for (x, y) in [(0, 0), (image.width - 1, 0), (0, image.height - 1), (image.width - 1, image.height - 1)] {
                XCTAssertEqual(image.alpha(x, y), 0, "\(state): nothing at the corner (\(x), \(y))")
            }
            var opaque = 0
            for y in stride(from: 0, to: image.height, by: 3) {
                for x in stride(from: 0, to: image.width, by: 3) {
                    let p = image.pixel(x, y), a = image.alpha(x, y)
                    XCTAssertLessThanOrEqual(max(p.r, p.g, p.b), a, "\(state): premultiplied at (\(x), \(y))")
                    if a > 200 { opaque += 1 }
                }
            }
            XCTAssertGreaterThan(opaque, 0, "\(state): an orb is drawn")
        }
    }

    /// Two streams, two orbs: theirs appears in a meeting, never while dictating or at rest.
    func testTheFarEndsOrbAppearsOnlyInAMeeting() throws {
        XCTAssertEqual(try theirPixels(render(.dictating)), 0)
        XCTAssertEqual(try theirPixels(render(.idle)), 0)
        XCTAssertGreaterThan(try theirPixels(render(.meeting)), 500)
    }

    /// Problem: the far end's orb fades toward grey.
    func testASilentFarEndFadesTowardGrey() throws {
        XCTAssertLessThan(try theirPixels(render(.problem)), try theirPixels(render(.meeting)) / 4)
    }

    /// Blotting: both orbs condense into one small drop in the ink colour.
    func testBlottingCondensesToASmallInkDrop() throws {
        let meeting = try render(.meeting), blotting = try render(.blotting)
        XCTAssertLessThan(coverage(blotting), coverage(meeting) / 4, "a small drop")
        XCTAssertEqual(try theirPixels(blotting), 0, "neither colour is left")
        let centre = blotting.pixel(blotting.width / 2, blotting.height / 2), a = Double(blotting.alpha(blotting.width / 2, blotting.height / 2))
        XCTAssertGreaterThan(a, 100, "the drop sits at the centre")
        XCTAssertLessThan(Double(centre.b) / max(a, 1), 0.5, "in the ink's dark colour")
    }

    /// A still frame stops the shader's time: the same picture whatever the simulation's clock.
    func testAStillFrameIgnoresTheTime() throws {
        let a = try InkSnapshot.render(.dictating, t: 3, width: 120, height: 120, palette: palette, voice: .silent,
                                       motion: false, pipeline: pipeline)
        let b = try InkSnapshot.render(.dictating, t: 40, width: 120, height: 120, palette: palette, voice: .silent,
                                       motion: false, pipeline: pipeline)
        XCTAssertEqual(a.rgba, b.rgba)
    }

    // MARK: Helpers

    private func render(_ state: InkState) throws -> InkImage {
        try InkSnapshot.render(state, t: 12, width: 360, height: 720, palette: palette, pipeline: pipeline)
    }

    /// Pixels in their orange (un-premultiplied), where the orb is solid enough to tell.
    private func theirPixels(_ image: InkImage) throws -> Int {
        var n = 0
        for y in 0..<image.height {
            for x in 0..<image.width {
                let a = Int(image.alpha(x, y))
                guard a > 120 else { continue }
                let p = image.pixel(x, y)
                let r = Int(p.r) * 255 / a, g = Int(p.g) * 255 / a, b = Int(p.b) * 255 / a
                if r - g > 40, g - b > 40 { n += 1 }
            }
        }
        return n
    }

    /// Pixels the orb covers at all.
    private func coverage(_ image: InkImage) -> Int {
        var n = 0
        for y in 0..<image.height {
            for x in 0..<image.width where image.alpha(x, y) > 40 { n += 1 }
        }
        return n
    }
}
