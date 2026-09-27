// From the core's audio bands to the ink's voice. The core publishes three bands of the live
// audio (inkwell.h, InkBands: RMS amplitude per band, linear full scale) and leaves the scale to
// the renderer. The ink reads them once per frame, at vsync.
import Foundation

/// The live levels the ink answers, each 0...1.
public struct InkLevels: Equatable, Sendable {
    /// Your mic.
    public var near: Double
    /// The far end.
    public var far: Double

    public init(near: Double, far: Double) {
        self.near = near
        self.far = far
    }

    /// No audio.
    public static let silent = InkLevels(near: 0, far: 0)

    /// The level of one stream's bands, on a decibel scale: `floorDB` and below reads 0 (room
    /// noise and silence), `ceilingDB` and above reads 1. The ink's envelope follower does the
    /// smoothing, so this is a plain map.
    ///
    /// The scale is the one the prototype's stand-in voice implies: ordinary speech into a laptop
    /// mic (about -35 to -20 dBFS RMS) swings across the 0.5 at which the ink throws a droplet,
    /// and a quiet room (below -60) stays still. S2.7 and S2.8 check it against real takes.
    public static func level(low: Float, mid: Float, high: Float) -> Double {
        // The bands do not overlap, so their powers add.
        let power = Double(low) * Double(low) + Double(mid) * Double(mid) + Double(high) * Double(high)
        guard power.isFinite, power > 0 else { return 0 }
        let db = 10 * log10(power)
        return min(1, max(0, (db - floorDB) / (ceilingDB - floorDB)))
    }

    /// Reads 0 at and below this RMS level (dBFS).
    public static let floorDB = -60.0
    /// Reads 1 at and above this RMS level (dBFS).
    public static let ceilingDB = -20.0
}
