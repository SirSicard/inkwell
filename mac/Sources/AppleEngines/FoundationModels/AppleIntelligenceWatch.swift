// Notices when Apple Intelligence becomes available or stops being available while the app runs
// (turned on or off in System Settings, or its model finishing its download), so polish is
// registered or let go of then (`AppleEngines.syncPolish`), not only at launch.
//
// No polling: the system model's `availability` is observable (SystemLanguageModel conforms to
// Observable), so the watch re-arms an observation after each change. The shell can also ask it to
// look again (`recheck`) when the app becomes active, which is when a change made in System
// Settings is most likely.
import Foundation
import Synchronization
#if canImport(FoundationModels)
    import FoundationModels
#endif

/// Where the availability comes from: the system model, or a test's.
public protocol AppleIntelligenceSource: Sendable {
    /// The availability now.
    func current() -> AppleIntelligence
    /// Calls `changed` once, from any thread, the next time the availability may have changed.
    func observeNextChange(_ changed: @escaping @Sendable () -> Void)
}

/// The system model's availability.
public struct SystemAppleIntelligence: AppleIntelligenceSource {
    public init() {}

    public func current() -> AppleIntelligence {
        AppleIntelligence.current()
    }

    public func observeNextChange(_ changed: @escaping @Sendable () -> Void) {
        #if canImport(FoundationModels)
            withObservationTracking {
                _ = SystemLanguageModel.default.availability
            } onChange: {
                // Observation calls this before the value changes (will-set): look afterwards.
                DispatchQueue.global(qos: .utility).async { changed() }
            }
        #endif
    }
}

/// Calls `onChange` with the new availability each time it differs from the last one seen.
public final class AppleIntelligenceWatch: Sendable {
    private let source: any AppleIntelligenceSource
    private let onChange: @Sendable (AppleIntelligence) -> Void
    /// The last availability seen; nil once stopped.
    private let last: Mutex<AppleIntelligence?>

    /// Starts watching from `source`'s availability now, which `onChange` is not called for (the
    /// caller has just acted on it).
    public init(
        source: any AppleIntelligenceSource = SystemAppleIntelligence(),
        onChange: @escaping @Sendable (AppleIntelligence) -> Void
    ) {
        self.source = source
        self.onChange = onChange
        last = Mutex(source.current())
        arm()
    }

    /// Looks again now (the app became active, for one).
    public func recheck() {
        let now = source.current()
        let changed = last.withLock { last -> Bool in
            guard let seen = last, seen != now else { return false }
            last = now
            return true
        }
        if changed {
            onChange(now)
        }
    }

    /// Stops calling `onChange`.
    public func stop() {
        last.withLock { $0 = nil }
    }

    private func arm() {
        source.observeNextChange { [weak self] in
            guard let self, self.last.withLock({ $0 != nil }) else { return }
            self.recheck()
            self.arm()
        }
    }
}
