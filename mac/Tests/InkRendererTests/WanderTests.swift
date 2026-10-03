// Where the main window's orb goes: a seeded path, so each check sees the same one. Live it
// wanders slowly from spot to spot; at rest it glides to a new spot when asked, then holds still.
import XCTest

@testable import InkRenderer

final class WanderTests: XCTestCase {
    private let bounds = OrbWander.Bounds(x: 0.46...0.68, y: 0.18...0.36)

    private func wander(seed: UInt32 = 7) -> OrbWander {
        OrbWander(bounds: bounds, start: SIMD2(0.56, 0.26), random: .seeded(seed))
    }

    private func distance(_ a: SIMD2<Double>, _ b: SIMD2<Double>) -> Double {
        ((a - b) * (a - b)).sum().squareRoot()
    }

    /// Ten minutes of live wander at 60 frames a second: always inside the bounds, never faster
    /// than a slow drift (no jump at the end of one leg and the start of the next), and over the
    /// minutes it visits much of the region, not one corner.
    func testLiveItWandersSlowlyAndSmoothlyInsideTheBounds() {
        var w = wander()
        var last = w.position(at: 0)
        var fastest = 0.0
        var seen = (x: last.x...last.x, y: last.y...last.y)
        for frame in 1...(60 * 600) {
            let t = Double(frame) / 60
            w.wander(at: t)
            let p = w.position(at: t)
            XCTAssertTrue(bounds.contains(p), "\(p) at \(t) s")
            fastest = max(fastest, distance(p, last) * 60)
            seen = (min(seen.x.lowerBound, p.x)...max(seen.x.upperBound, p.x),
                    min(seen.y.lowerBound, p.y)...max(seen.y.upperBound, p.y))
            last = p
        }
        // A fortieth of the view's width a second at most: about 26 pt in the default window.
        XCTAssertLessThan(fastest, 0.025, "per second, in fractions of the view")
        XCTAssertGreaterThan(seen.x.upperBound - seen.x.lowerBound, 0.6 * (bounds.x.upperBound - bounds.x.lowerBound))
        XCTAssertGreaterThan(seen.y.upperBound - seen.y.lowerBound, 0.6 * (bounds.y.upperBound - bounds.y.lowerBound))
    }

    func testTheSameSeedTakesTheSamePath() {
        var a = wander(seed: 3), b = wander(seed: 3), c = wander(seed: 4)
        var differs = false
        for frame in stride(from: 0, through: 60 * 60, by: 30) {
            let t = Double(frame) / 60
            a.wander(at: t)
            b.wander(at: t)
            c.wander(at: t)
            XCTAssertEqual(a.position(at: t), b.position(at: t))
            if a.position(at: t) != c.position(at: t) { differs = true }
        }
        XCTAssertTrue(differs, "another seed, another path")
    }

    /// At rest: asked to move, it glides over OrbWander.restGlide to a new spot well away from the
    /// old one, eased (slow at both ends), then holds it.
    func testAtRestItGlidesToANewSpotThenHoldsStill() {
        var w = wander()
        let from = w.position(at: 10)
        XCTAssertFalse(w.isMoving(at: 10), "it starts still")
        w.move(at: 10, animated: true)
        XCTAssertTrue(w.isMoving(at: 10.01))
        XCTAssertTrue(w.isMoving(at: 10 + OrbWander.restGlide - 0.01))
        XCTAssertFalse(w.isMoving(at: 10 + OrbWander.restGlide))
        let to = w.position(at: 10 + OrbWander.restGlide)
        XCTAssertTrue(bounds.contains(to))
        XCTAssertGreaterThanOrEqual(distance(from, to), OrbWander.minimumHop * bounds.diagonal - 1e-9, "a new spot, not a nudge")
        // Eased: the first tenth of the time covers far less than a tenth of the way.
        let early = w.position(at: 10 + OrbWander.restGlide * 0.1)
        XCTAssertLessThan(distance(from, early), 0.05 * distance(from, to))
        XCTAssertEqual(w.position(at: 500), to, "and it holds there")
    }

    /// With motion off it takes the new spot at once: nothing to animate.
    func testWithoutMotionItTakesTheNewSpotAtOnce() {
        var w = wander()
        let from = w.position(at: 10)
        w.move(at: 10, animated: false)
        XCTAssertFalse(w.isMoving(at: 10))
        XCTAssertNotEqual(w.position(at: 10), from)
        XCTAssertTrue(bounds.contains(w.position(at: 10)))
    }

    /// Going to rest mid-leg holds the orb where it is: no jump back to a rest spot.
    func testHoldingStopsItWhereItIs() {
        var w = wander()
        for frame in 0...(60 * 5) { w.wander(at: Double(frame) / 60) }
        let here = w.position(at: 5)
        w.hold(at: 5)
        XCTAssertFalse(w.isMoving(at: 5))
        XCTAssertEqual(w.position(at: 5), here)
        XCTAssertEqual(w.position(at: 60), here)
    }

    func testAStartOutsideTheBoundsIsPulledIn() {
        let w = OrbWander(bounds: bounds, start: SIMD2(0.9, 0.05), random: .seeded(1))
        XCTAssertEqual(w.position(at: 0), SIMD2(0.68, 0.18))
    }
}
