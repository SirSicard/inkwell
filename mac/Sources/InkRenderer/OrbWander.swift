// Where the main window's orb goes, so it is not always in the same place: a path of eased legs
// between random spots inside a region of the view. Live, one leg follows another, slowly, on the
// frames the orb already draws. At rest the orb glides to a new spot only when asked (InkView asks
// when it comes on screen, when the screen behind it changes, and when its window becomes key
// after `restInterval` in one spot) and then holds still, so a resting orb still draws nothing
// between moves (architecture rule 9).
//
// A value with its own random source: seeded, the same calls give the same path (the tests).
import Foundation

/// The orb's centre over time, as fractions of the view (x from the left, y from the top).
public struct OrbWander: Sendable {
    /// The region the centre stays in.
    public struct Bounds: Equatable, Sendable {
        public var x: ClosedRange<Double>
        public var y: ClosedRange<Double>

        public init(x: ClosedRange<Double>, y: ClosedRange<Double>) {
            self.x = x
            self.y = y
        }

        public func contains(_ p: SIMD2<Double>) -> Bool {
            x.contains(p.x) && y.contains(p.y)
        }

        /// The four corners: the region's extremes, which the contrast checks render.
        public var corners: [SIMD2<Double>] {
            [SIMD2(x.lowerBound, y.lowerBound), SIMD2(x.upperBound, y.lowerBound),
             SIMD2(x.lowerBound, y.upperBound), SIMD2(x.upperBound, y.upperBound)]
        }

        /// The corner-to-corner distance, in the same fractions.
        public var diagonal: Double {
            let w = x.upperBound - x.lowerBound, h = y.upperBound - y.lowerBound
            return (w * w + h * h).squareRoot()
        }

        func clamped(_ p: SIMD2<Double>) -> SIMD2<Double> {
            SIMD2(min(x.upperBound, max(x.lowerBound, p.x)), min(y.upperBound, max(y.lowerBound, p.y)))
        }
    }

    /// Seconds a glide at rest takes.
    public static let restGlide = 2.4
    /// Seconds a resting orb holds its spot before its window becoming key again moves it.
    public static let restInterval = 150.0
    /// A new spot is at least this share of the region's diagonal away from the old one.
    public static let minimumHop = 1.0 / 3
    /// Live legs: their mean speed in fractions of the view per second (an eased leg peaks at
    /// 1.875 times it, about 0.022: some 23 pt a second across the default window), and the
    /// shortest a leg takes.
    static let cruise = 0.012
    static let shortestLeg = 12.0

    /// One eased move: from, to, when it starts and how long it takes (0: it is there).
    private struct Leg: Sendable {
        var from: SIMD2<Double>
        var to: SIMD2<Double>
        var start: Double
        var duration: Double
    }

    public let bounds: Bounds
    private var random: InkRandom
    private var leg: Leg

    /// An orb at `start` (pulled into the bounds), still.
    public init(bounds: Bounds, start: SIMD2<Double>, random: InkRandom = .system) {
        self.bounds = bounds
        self.random = random
        let p = bounds.clamped(start)
        leg = Leg(from: p, to: p, start: 0, duration: 0)
    }

    /// Where the centre is at time `t` (seconds, any clock the caller keeps to).
    public func position(at t: Double) -> SIMD2<Double> {
        guard leg.duration > 0 else { return leg.to }
        let u = min(1, max(0, (t - leg.start) / leg.duration))
        // Smootherstep: no speed and no acceleration at either end, so legs join without a kink.
        let e = u * u * u * (u * (u * 6 - 15) + 10)
        return leg.from + (leg.to - leg.from) * e
    }

    /// Whether it is on its way somewhere at `t`.
    public func isMoving(at t: Double) -> Bool {
        leg.duration > 0 && t < leg.start + leg.duration
    }

    /// Live: once a leg is done, the next starts from there toward a new spot, slowly.
    public mutating func wander(at t: Double) {
        guard !isMoving(at: t) else { return }
        let from = position(at: t)
        let to = nextSpot(from: from)
        let distance = ((to - from) * (to - from)).sum().squareRoot()
        leg = Leg(from: from, to: to, start: t, duration: max(Self.shortestLeg, distance / Self.cruise))
    }

    /// At rest: to a new spot, gliding over `restGlide`, or at once when not `animated`.
    public mutating func move(at t: Double, animated: Bool) {
        let from = position(at: t)
        let to = nextSpot(from: from)
        leg = Leg(from: from, to: to, start: t, duration: animated ? Self.restGlide : 0)
    }

    /// Stops where it is at `t`.
    public mutating func hold(at t: Double) {
        let here = position(at: t)
        leg = Leg(from: here, to: here, start: t, duration: 0)
    }

    /// A random spot in the bounds at least `minimumHop` of the diagonal from `from`: the first of
    /// eight draws that is, else the farthest of them.
    private mutating func nextSpot(from: SIMD2<Double>) -> SIMD2<Double> {
        let hop = Self.minimumHop * bounds.diagonal
        var best = from, bestDistance = -1.0
        for _ in 0..<8 {
            let p = SIMD2(
                bounds.x.lowerBound + random.next() * (bounds.x.upperBound - bounds.x.lowerBound),
                bounds.y.lowerBound + random.next() * (bounds.y.upperBound - bounds.y.lowerBound))
            let d = ((p - from) * (p - from)).sum().squareRoot()
            if d >= hop { return p }
            if d > bestDistance {
                best = p
                bestDistance = d
            }
        }
        return best
    }
}
