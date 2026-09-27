// The count of frames the ink has drawn, for the shell budget (I7): idle must draw none. The
// renderer calls `tick` once per frame it presents; scripts/idle-budget.sh reads the count
// through the shell's measurement marks. Lock-free, so a render callback can call it.
import Synchronization

/// Frames drawn since launch.
public final class FrameCounter: Sendable {
    private let frames = Atomic<UInt64>(0)

    public init() {}

    /// Counts one presented frame. Any thread; never blocks or allocates.
    public func tick() {
        frames.add(1, ordering: .relaxed)
    }

    /// Frames counted so far.
    public var count: UInt64 {
        frames.load(ordering: .relaxed)
    }
}

extension InkRenderer {
    /// Every frame the ink presents, whichever view drew it.
    public static let frames = FrameCounter()
}
