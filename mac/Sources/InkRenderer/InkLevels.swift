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
    /// smoothing, so this is a plain map. The same map serves your mic and the far end.
    ///
    /// Tuned in S2.8 on measured levels (the core's test
    /// `the_ink_level_map_rests_on_the_measured_fixture_levels` keeps the numbers true): the public
    /// AMI fixture's hops, as the core publishes them, sit at -59 to -57 dBFS in the room between
    /// words (p10, p25), and its speech at -41 (p75) to -25 (p90) for one talker, -35 to -29 for
    /// three mixed; the loudest (p99) reaches -19. So the floor moved from -60 to -55: the room
    /// between words reads 0, and the built-in mic's measured room floor (-50.6 dB, S0.3) about
    /// 0.1, a breath rather than a pulse. The ceiling stays at -20, so only the loudest speech
    /// saturates, and the droplet threshold (0.5) falls at -37.5 dBFS. Real meetings in the
    /// dogfood week check it on this Mac's own mic and on call audio (MEETINGS-CHECKLIST.md).
    public static func level(low: Float, mid: Float, high: Float) -> Double {
        // The bands do not overlap, so their powers add.
        let power = Double(low) * Double(low) + Double(mid) * Double(mid) + Double(high) * Double(high)
        guard power.isFinite, power > 0 else { return 0 }
        let db = 10 * log10(power)
        return min(1, max(0, (db - floorDB) / (ceilingDB - floorDB)))
    }

    /// Reads 0 at and below this RMS level (dBFS).
    public static let floorDB = -55.0
    /// Reads 1 at and above this RMS level (dBFS).
    public static let ceilingDB = -20.0
}
