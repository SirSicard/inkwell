// The ink drawn offscreen through the real pipeline: the bundled MSL compiled at run time, as the
// app does. Skipped on a Mac without Metal.
import Metal
import XCTest

@testable import InkRenderer

final class RenderTests: XCTestCase {
    private var pipeline: InkPipeline!

    override func setUpWithError() throws {
        try XCTSkipUnless(InkRenderer.isSupported, "no Metal device")
        pipeline = try InkPipeline.shared.get()
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

    /// Two streams, two inks: the far end's sepia appears in a meeting, never while dictating.
    func testTheFarEndsInkAppearsOnlyWithTwoStreams() throws {
        XCTAssertEqual(try sepiaPixels(render(.dictating)), 0)
        XCTAssertEqual(try sepiaPixels(render(.idle)), 0)
        XCTAssertGreaterThan(try sepiaPixels(render(.meeting)), 500)
    }

    /// Problem: the far end's ink fades to a ghost, outlined in seal red.
    func testASilentFarEndIsAGhostWithASealRedOutline() throws {
        let problem = try render(.problem)
        XCTAssertLessThan(try sepiaPixels(problem), try sepiaPixels(render(.meeting)) / 4)
        var seal = 0
        for y in 0..<problem.height {
            for x in 0..<problem.width {
                let p = problem.pixel(x, y)
                if p.r > 150, p.g < 120, p.b < 110 { seal += 1 }
            }
        }
        XCTAssertGreaterThan(seal, 50, "the dashed outline")
    }

    /// The wordmark is knocked out of whatever sits under it: a letter is the inverse of what lies
    /// beneath, so dark on paper and light on ink. The ink is raised into the wordmark's band to
    /// see both.
    func testTheWordmarkIsDarkOnPaperAndLightOnInk() throws {
        let image = try InkSnapshot.render(.dictating, t: 12, width: 360, height: 720, cy: 0.9, pipeline: pipeline)
        let mark = try XCTUnwrap(image.wordmark)
        let plain = try InkSnapshot.render(.dictating, t: 12, width: 360, height: 720, wordmark: false, cy: 0.9,
                                           pipeline: pipeline)
        var onPaper = 0, onInk = 0
        for y in 0..<image.height {
            for x in 0..<image.width where mark.coverage[y * image.width + x] == 255 {
                let under = plain.pixel(x, y), letter = image.pixel(x, y)
                for (u, l) in [(under.r, letter.r), (under.g, letter.g), (under.b, letter.b)] {
                    XCTAssertLessThanOrEqual(abs(255 - Int(u) - Int(l)), 1, "the inverse of what is under (\(x), \(y))")
                }
                let lum = plain.luminance(x, y)
                if lum > 180 { onPaper += 1 } else if lum < 60 { onInk += 1 }
            }
        }
        XCTAssertGreaterThan(onPaper, 100, "letters on paper (dark)")
        XCTAssertGreaterThan(onInk, 100, "letters on ink (light): the raised ink runs under some")
    }

    func testTheWordmarkIsPlacedByThePrototypesRule() {
        // w = 360: 57 px type, 35 px from the left, top at 40 px.
        let mark = Wordmark.rasterize(width: 360, height: 720, pointWidth: 360)
        XCTAssertEqual(mark.fontSize, 57)
        XCTAssertEqual(mark.x, 35)
        XCTAssertEqual(mark.fontName, ".SFNS-Black", "SF Pro Black, the system's heaviest face")
        XCTAssertGreaterThan(mark.baselineFromTop, 40 + 57 * 0.6)
        XCTAssertLessThan(mark.baselineFromTop, 40 + 57 * 1.1)
        // At a 2x backing scale the canvas doubles and so does the type.
        XCTAssertEqual(Wordmark.rasterize(width: 720, height: 1440, pointWidth: 360).fontSize, 114)
    }

    /// The ink stays inside its zone: in the 56-point rail (112 x 1400 px at 2x) and in a small
    /// square zone like the Drop's, the wettest, loudest states spread toward the sides but never
    /// come nearer than half the fence band (a tenth of the shorter side), so no straight edge
    /// cuts the blob where the zone ends.
    func testTheInkStaysInsideItsZone() throws {
        for (w, h) in [(112, 1400), (168, 168)] {
            let keepOut = Int(0.05 * Double(min(w, h))) - 1
            var nearest = Int.max
            for state in [InkState.dictating, .meeting, .problem] {
                for t in stride(from: 3.0, through: 60, by: 3) {
                    let image = try InkSnapshot.render(state, t: t, width: w, height: h, wordmark: false,
                                                       voice: .levels(near: 1, far: 1), pipeline: pipeline)
                    for y in 0..<h {
                        for x in 0..<w where image.luminance(x, y) < 170 {
                            let d = min(x, w - 1 - x, y, h - 1 - y)
                            nearest = min(nearest, d)
                            XCTAssertGreaterThanOrEqual(d, keepOut, "\(state) t=\(t) \(w)x\(h): ink at (\(x), \(y))")
                        }
                    }
                }
            }
            // The check means something: the ink does get close to the fence.
            XCTAssertLessThan(nearest, keepOut + Int(0.05 * Double(min(w, h))) + 2, "\(w)x\(h): nearest ink \(nearest) px")
        }
    }

    // MARK: Helpers

    private func render(_ state: InkState) throws -> InkImage {
        try InkSnapshot.render(state, t: 12, width: 360, height: 720, pipeline: pipeline)
    }

    /// Pixels of the far end's sepia ink (0.494, 0.329, 0.192 in the shader, lit and dried).
    private func sepiaPixels(_ image: InkImage) throws -> Int {
        var n = 0
        for y in 0..<image.height {
            for x in 0..<image.width {
                let p = image.pixel(x, y)
                let r = Int(p.r), g = Int(p.g), b = Int(p.b)
                if r > 80, r < 180, r - g > 25, g - b > 20 { n += 1 }
            }
        }
        return n
    }
}
