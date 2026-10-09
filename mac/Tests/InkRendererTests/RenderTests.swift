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
            // At rest the orb is drawn at about half strength, by design; live, it is near solid.
            let solid = state == .idle ? 90 : 200
            var opaque = 0
            for y in stride(from: 0, to: image.height, by: 3) {
                for x in stride(from: 0, to: image.width, by: 3) {
                    let p = image.pixel(x, y), a = image.alpha(x, y)
                    XCTAssertLessThanOrEqual(max(p.r, p.g, p.b), a, "\(state): premultiplied at (\(x), \(y))")
                    if a > solid { opaque += 1 }
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

    /// At rest the orb leans toward the dots by the palette's rest tint; live, it goes on to yours
    /// from there. The first live frame is next to the tinted rest frame, not to the plain idle
    /// one: the change of state starts where the rest colour is, without a jump.
    func testTheLiveTransitionStartsFromTheTintedRestColour() throws {
        var tinted = palette
        tinted.restTint = 0.6
        let plainRest = try frame(palette) { _ in }
        let tintedRest = try frame(tinted) { _ in }
        let firstLive = try frame(tinted) { sim in
            sim.state = .dictating
            sim.step(1.0 / 60, snap: false, voice: .silent)
        }
        let tint = difference(plainRest, tintedRest), step = difference(tintedRest, firstLive)
        XCTAssertGreaterThan(tint, 0.1, "the tint shows")
        XCTAssertLessThan(step * 4, tint, "one live frame moves a little from the tinted rest: \(step) against \(tint)")
    }

    /// The rest tint rides in idle's fourth lane, clamped to 0...1; 0 rests in idle alone.
    func testTheRestTintIsPackedClamped() {
        var tinted = palette
        let sim = InkSimulation(random: .seeded(1))
        XCTAssertEqual(sim.uniforms(palette: palette, placement: .centred, motion: false).idle.w, 0)
        tinted.restTint = 0.2
        XCTAssertEqual(sim.uniforms(palette: tinted, placement: .centred, motion: false).idle, SIMD4(0.6, 0.6, 0.6, 0.2))
        tinted.restTint = 3
        XCTAssertEqual(sim.uniforms(palette: tinted, placement: .centred, motion: false).idle.w, 1)
        tinted.restTint = -1
        XCTAssertEqual(sim.uniforms(palette: tinted, placement: .centred, motion: false).idle.w, 0)
    }

    func testRestBoostUsesTheExistingUniformLaneAndLeavesLiveInkUnchanged() throws {
        var boosted = palette.withShellStrength(1)
        let sim = InkSimulation(random: .seeded(1))
        XCTAssertEqual(MemoryLayout<InkUniforms>.stride, 160)
        XCTAssertEqual(sim.uniforms(palette: boosted, placement: .centred, motion: false).ink.w, 1)
        boosted.restBoost = -1
        XCTAssertEqual(sim.uniforms(palette: boosted, placement: .centred, motion: false).ink.w, 0)
        boosted.restBoost = 2
        XCTAssertEqual(sim.uniforms(palette: boosted, placement: .centred, motion: false).ink.w, 1)
        let stronger = try frame(palette.withShellStrength(1)) { _ in }
        XCTAssertGreaterThan(coverage(stronger), coverage(try frame(palette) { _ in }))
        for state in [InkState.dictating, .meeting, .blotting, .problem] {
            let original = try InkSnapshot.render(state, t: 12, width: 120, height: 120, palette: palette,
                                                  motion: false, pipeline: pipeline)
            let boosted = try InkSnapshot.render(state, t: 12, width: 120, height: 120,
                                                 palette: palette.withShellStrength(1), motion: false, pipeline: pipeline)
            XCTAssertEqual(original.rgba, boosted.rgba, "settled live and blotting ignore rest boost")
        }
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

    /// A still frame (the shader's time stopped, so two frames differ only by their inputs) of the
    /// settled idle orb, after `change`.
    private func frame(_ palette: OrbPalette, _ change: (inout InkSimulation) -> Void) throws -> InkImage {
        var sim = InkSimulation(random: .seeded(1))
        sim.canvasWidth = 240
        sim.canvasHeight = 240
        sim.settle(voice: .silent)
        change(&sim)
        return try InkSnapshot.render(sim.uniforms(palette: palette, placement: .centred, motion: false),
                                      width: 240, height: 240, pipeline: pipeline)
    }

    /// How far apart two frames' colours are: the distance between the mean colours (0...1, not
    /// premultiplied) of the pixels where the orb is solid enough to tell. Coverage apart: the
    /// first live frame also grows the orb a little.
    private func difference(_ a: InkImage, _ b: InkImage) -> Double {
        func mean(_ image: InkImage) -> SIMD3<Double> {
            var sum = SIMD3<Double>(0, 0, 0), n = 0.0
            for i in stride(from: 0, to: image.rgba.count, by: 4) where image.rgba[i + 3] >= 64 {
                let alpha = Double(image.rgba[i + 3])
                sum += SIMD3(Double(image.rgba[i]), Double(image.rgba[i + 1]), Double(image.rgba[i + 2])) / alpha
                n += 1
            }
            return n > 0 ? sum / n : sum
        }
        let d = mean(a) - mean(b)
        return (d * d).sum().squareRoot()
    }

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
