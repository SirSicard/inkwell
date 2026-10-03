// What the shell tells the orb and the edge glow about the theme: the colours, which the shell
// resolves from the user's appearance settings, where the orb sits, and the edge's strokes. The
// renderer keeps no palette of its own: the values come from the shell's design tokens.
import Foundation

/// The orb's colours, sRGB 0...1: yours (yA, and yB, its lighter partner), theirs (tA, tB), the
/// orb at rest (idle) and the blotted drop (ink), and whether the theme is dark. At rest the orb
/// leans from idle toward the dots by restTint (0...1): its first shade toward yA, its second
/// toward tA; 0 rests in idle alone.
public struct OrbPalette: Equatable, Sendable {
    public var yA: SIMD3<Float>
    public var yB: SIMD3<Float>
    public var tA: SIMD3<Float>
    public var tB: SIMD3<Float>
    public var idle: SIMD3<Float>
    public var ink: SIMD3<Float>
    public var dark: Bool
    public var restTint: Float

    public init(
        yA: SIMD3<Float>, yB: SIMD3<Float>, tA: SIMD3<Float>, tB: SIMD3<Float>, idle: SIMD3<Float>,
        ink: SIMD3<Float>, dark: Bool, restTint: Float = 0
    ) {
        self.yA = yA
        self.yB = yB
        self.tA = tA
        self.tB = tB
        self.idle = idle
        self.ink = ink
        self.dark = dark
        self.restTint = restTint
    }

    /// Greys, until the shell has read the theme (and for tests).
    public static let neutral = OrbPalette(
        yA: SIMD3(0.5, 0.5, 0.5), yB: SIMD3(0.7, 0.7, 0.7), tA: SIMD3(0.4, 0.4, 0.4),
        tB: SIMD3(0.64, 0.64, 0.64), idle: SIMD3(0.6, 0.6, 0.6), ink: SIMD3(0.1, 0.1, 0.1), dark: false)
}

/// Where the orb sits in its view: its centre as fractions of the width and of the height from the
/// top, and its unit (the orb's scale) as a fraction of the view's shorter side.
public struct OrbPlacement: Equatable, Sendable {
    public var x: Double
    public var yFromTop: Double
    public var unit: Double

    public init(x: Double, yFromTop: Double, unit: Double) {
        self.x = x
        self.yFromTop = yFromTop
        self.unit = unit
    }

    /// The view's centre, one unit to the shorter side.
    public static let centred = OrbPlacement(x: 0.5, yFromTop: 0.5, unit: 1)
}

/// The edge glow's look: its strokes from wide to narrow, the band the two colours blend over, how
/// far the blend leans toward whoever is quieter, its slow flow, and the window's corner radius.
public struct EdgeGlowStyle: Equatable, Sendable {
    /// One stroke of the stack: its width in points and its opacity.
    public struct Stroke: Equatable, Sendable {
        public var width: Double
        public var alpha: Double

        public init(width: Double, alpha: Double) {
            self.width = width
            self.alpha = alpha
        }
    }

    public var strokes: [Stroke]
    public var band: Double
    public var lean: Double
    public var flowAmplitude: Double
    /// Radians per second.
    public var flowSpeed: Double
    public var cornerRadius: Double

    public init(strokes: [Stroke], band: Double, lean: Double, flowAmplitude: Double, flowSpeed: Double, cornerRadius: Double) {
        self.strokes = strokes
        self.band = band
        self.lean = lean
        self.flowAmplitude = flowAmplitude
        self.flowSpeed = flowSpeed
        self.cornerRadius = cornerRadius
    }
}

/// One frame of the edge glow: a horizontal gradient of five stops round the whole edge. Yours on
/// the left, theirs on the right, blending in a band that leans away from whoever is speaking; all
/// yours while dictating; gone once the call is blotted.
public struct EdgeGlowFrame: Equatable, Sendable {
    /// One stop: where (0...1, left to right) and its colour, sRGB with alpha.
    public struct Stop: Equatable, Sendable {
        public var location: Double
        public var color: SIMD4<Double>
    }

    public var stops: [Stop]

    /// Where the blend's centre may go, whoever speaks.
    static let centreRange = 0.28...0.72
    /// How fast the left end shifts between your two shades, radians per second.
    static let hueSpeed = 0.9

    /// The frame for weights `w` (dictating, meeting, blotting, problem), the live levels and the
    /// time `t` in seconds; nil when nothing would show.
    public static func compute(
        w: SIMD4<Double>, you: Double, them: Double, t: Double, palette: OrbPalette, style: EdgeGlowStyle
    ) -> EdgeGlowFrame? {
        let d = w.x, m = w.y, b = w.z
        let a1 = d * (0.3 + 0.7 * you) + m * (1 - b) * (0.25 + 0.75 * you)
        let a2 = m * (1 - b) * (0.25 + 0.75 * them)
        let aR = a1 + (a2 - a1) * m
        guard a1 >= 0.01 || aR >= 0.01 else { return nil }
        let c = min(centreRange.upperBound, max(centreRange.lowerBound, 0.5 + style.lean * (you - them) * m))
        let flow = sin(t * style.flowSpeed) * style.flowAmplitude
        let hue = 0.5 + 0.5 * sin(t * hueSpeed)
        func mix(_ a: SIMD3<Float>, _ b: SIMD3<Float>, _ k: Double) -> SIMD3<Double> {
            let a = SIMD3<Double>(a), b = SIMD3<Double>(b)
            return a + (b - a) * k
        }
        func stop(_ at: Double, _ rgb: SIMD3<Double>, _ alpha: Double) -> Stop {
            Stop(location: at, color: SIMD4(rgb, min(1, max(0, alpha))))
        }
        let p = palette
        var stops = [
            stop(0, mix(p.yA, p.yB, hue), a1),
            stop(c - style.band + flow, mix(p.yB, p.yA, hue), a1),
            stop(c, mix(p.yB, p.tA, 0.5 * m), (a1 + aR) / 2),
            stop(c + style.band + flow, mix(p.yA, p.tA, m), aR),
            stop(1, mix(p.yB, p.tB, m), aR),
        ]
        // Stops stay in 0...1 and in order (a gradient takes nothing else).
        var floor = 0.0
        for i in stops.indices {
            stops[i].location = min(1, max(floor, stops[i].location))
            floor = stops[i].location
        }
        return EdgeGlowFrame(stops: stops)
    }
}
