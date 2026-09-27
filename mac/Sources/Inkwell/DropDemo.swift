// The Drop's focus check (mac/DROP-FOCUS-CHECKLIST.md): with INK_DROP_DEMO=<seconds> the Drop
// cycles through every state, one every <seconds>, while the owner types in another app. A test
// harness, off unless that variable is set; its timer is the only one in the shell, and it exists
// only for the check.
import Foundation
import InkRenderer

@MainActor
final class DropDemo {
    /// The order the Drop shows the states in: each live state, then idle (the Drop hides and
    /// comes back, which must not take focus either).
    nonisolated static let sequence: [InkState] = [.dictating, .meeting, .blotting, .problem, .idle]

    /// The seconds per state INK_DROP_DEMO asks for, if it asks: a number of at least half a second.
    nonisolated static func interval(from environment: [String: String]) -> TimeInterval? {
        guard let raw = environment["INK_DROP_DEMO"], let seconds = TimeInterval(raw),
            seconds.isFinite, seconds >= 0.5
        else { return nil }
        return seconds
    }

    private let ink: ShellInk
    private var index = 0
    private var timer: Timer?

    init(ink: ShellInk, interval: TimeInterval) {
        self.ink = ink
        ink.held = Self.sequence[index]
        timer = Timer.scheduledTimer(withTimeInterval: interval, repeats: true) { [weak self] _ in
            // A scheduled timer fires on the run loop it was scheduled on: the main one.
            MainActor.assumeIsolated { self?.advance() }
        }
    }

    private func advance() {
        index = (index + 1) % Self.sequence.count
        ink.held = Self.sequence[index]
    }

    isolated deinit {
        timer?.invalidate()
    }
}
