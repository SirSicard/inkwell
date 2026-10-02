// The edge glow's gradient, as the design canvas computes it: nothing at rest, yours alone while
// dictating, yours and theirs in a call leaning away from whoever speaks, gone once blotted.
import XCTest

@testable import InkRenderer

final class EdgeGlowTests: XCTestCase {
    private let style = EdgeGlowStyle(
        strokes: [.init(width: 44, alpha: 0.06), .init(width: 2, alpha: 1)],
        band: 0.26, lean: 0.2, flowAmplitude: 0.08, flowSpeed: 0.7, cornerRadius: 26)
    private let palette = OrbPalette(
        yA: SIMD3(0, 0, 1), yB: SIMD3(0, 0.5, 1), tA: SIMD3(1, 0.5, 0), tB: SIMD3(1, 0.2, 0),
        idle: SIMD3(0.5, 0.5, 0.5), ink: SIMD3(0, 0, 0), dark: false)

    private func frame(_ state: InkState, you: Double = 0, them: Double = 0, t: Double = 0) -> EdgeGlowFrame? {
        EdgeGlowFrame.compute(w: InkSimulation.weights(for: state), you: you, them: them, t: t, palette: palette, style: style)
    }

    func testNothingShowsAtRestOrOnceBlotted() {
        XCTAssertNil(frame(.idle))
        XCTAssertNil(frame(.blotting, you: 1, them: 1))
    }

    func testDictatingIsYoursThroughout() throws {
        let glow = try XCTUnwrap(frame(.dictating, you: 1))
        XCTAssertEqual(glow.stops.count, 5)
        for stop in glow.stops {
            XCTAssertEqual(stop.color.w, 1, accuracy: 1e-9, "full while you speak")
            XCTAssertEqual(stop.color.x, 0, accuracy: 1e-9, "no red: none of theirs")
        }
        XCTAssertEqual(try XCTUnwrap(frame(.dictating)).stops[0].color.w, 0.3, accuracy: 1e-9, "a quiet glow in a pause")
    }

    func testACallLeansAwayFromWhoeverSpeaksWithinItsRange() throws {
        // t = 0: no flow. You speaking pushes the blend's centre right, toward them.
        let you = try XCTUnwrap(frame(.meeting, you: 1, them: 0))
        XCTAssertEqual(you.stops[2].location, 0.7, accuracy: 1e-9)
        let them = try XCTUnwrap(frame(.meeting, you: 0, them: 1))
        XCTAssertEqual(them.stops[2].location, 0.3, accuracy: 1e-9)
        XCTAssertEqual(them.stops[4].color.x, 1, accuracy: 1e-9, "theirs on the right")
        // In order, inside 0...1, whatever the flow does.
        for t in stride(from: 0.0, through: 20, by: 0.37) {
            let stops = try XCTUnwrap(frame(.meeting, you: 0, them: 1, t: t)).stops.map(\.location)
            XCTAssertEqual(stops, stops.sorted())
            XCTAssertTrue(stops.allSatisfy { (0...1).contains($0) })
        }
    }
}
