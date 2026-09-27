// GPU time per presented frame, for the shell budget: the renderer's GPU cost is part of what the
// Mac pays while the ink is live. Off unless a measurement turns it on (INK_MEASURE); then each
// frame's command buffer reports its GPU start and end, and a mark reads the spread since the last
// mark. Metal reports on its own completion thread, so the samples sit behind a lock that nothing
// holds for longer than an append or a copy.
import Metal
import Synchronization

/// GPU milliseconds per frame since the last `drain`.
public final class GPUFrameTimes: Sendable {
    /// The spread of the frames' GPU times.
    public struct Summary: Equatable, Sendable {
        public var frames: Int
        public var p50: Double
        public var p95: Double
        public var max: Double
    }

    private let enabled = Atomic<Bool>(false)
    private let samples = Mutex<[Double]>([])
    /// Enough for 10 minutes at 60 fps between two marks; later frames are not recorded.
    static let capacity = 36_000

    public init() {}

    /// Starts recording.
    public func start() {
        samples.withLock { $0.reserveCapacity(Self.capacity) }
        enabled.store(true, ordering: .relaxed)
    }

    /// Whether frames are being recorded.
    public var isRecording: Bool { enabled.load(ordering: .relaxed) }

    /// Records `commandBuffer`'s GPU time once it completes, when recording.
    public func observe(_ commandBuffer: any MTLCommandBuffer) {
        guard isRecording else { return }
        commandBuffer.addCompletedHandler { [self] finished in
            let ms = (finished.gpuEndTime - finished.gpuStartTime) * 1000
            // A buffer that never ran on the GPU reports zeros.
            guard ms > 0, ms.isFinite else { return }
            record(ms)
        }
    }

    func record(_ ms: Double) {
        samples.withLock { all in
            if all.count < Self.capacity { all.append(ms) }
        }
    }

    /// The frames since the last call, and forgets them. Nil when none completed.
    public func drain() -> Summary? {
        let taken = samples.withLock { all -> [Double] in
            let copy = all
            all.removeAll(keepingCapacity: true)
            return copy
        }
        guard !taken.isEmpty else { return nil }
        let sorted = taken.sorted()
        func at(_ q: Double) -> Double { sorted[Int((Double(sorted.count - 1) * q).rounded(.down))] }
        return Summary(frames: sorted.count, p50: at(0.5), p95: at(0.95), max: sorted[sorted.count - 1])
    }
}
