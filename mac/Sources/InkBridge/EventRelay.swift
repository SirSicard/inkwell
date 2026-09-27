// The hop from the core's event thread to the main actor (inkwell.h, THREADS 1: hop to the main
// thread and return promptly; never wait on it).
//
// Events are queued under a lock and handed over in batches: the first event after a hand-over
// queues one main-queue block, and every event that arrives before that block runs rides along.
// However fast the core talks, the main actor wakes at most once per turn of its run loop, and
// nothing wakes it at all while the core is quiet. No timer is involved (architecture rule 9).
import Foundation
import Synchronization

/// Carries events from the core's event thread to the main actor, in order, in batches.
///
/// `push` is what `InkSession.start`'s `onEvent` calls. It never waits for the main thread.
public final class EventRelay: Sendable {
    /// Receives each batch on the main actor, oldest event first. Never called with an empty batch.
    public typealias Deliver = @MainActor @Sendable ([InkEvent]) -> Void

    private struct Waiting {
        var events: [InkEvent] = []
        /// A main-queue block is on its way to take `events`.
        var handOverQueued = false
        /// Main-queue blocks queued so far.
        var handOvers = 0
    }

    private let waiting = Mutex(Waiting())
    private let deliver: Deliver

    public init(deliver: @escaping Deliver) {
        self.deliver = deliver
    }

    /// **Event thread** (any thread). Queues `event` for the main actor and returns at once.
    ///
    /// A `meeting.partial` replaces the partial still waiting for the same record and channel: a
    /// partial is the engine's current guess and the next one supersedes it (partials are
    /// ephemeral, architecture rule 4). The newer one goes to the back of the queue, so it still
    /// arrives after any final that was queued before it. Every other event is delivered as is.
    public func push(_ event: InkEvent) {
        let queueHandOver = waiting.withLock { waiting -> Bool in
            if case .meetingPartial(let partial) = event {
                waiting.events.removeAll { Self.isPartial($0, of: partial) }
            }
            waiting.events.append(event)
            if waiting.handOverQueued {
                return false
            }
            waiting.handOverQueued = true
            waiting.handOvers += 1
            return true
        }
        if queueHandOver {
            DispatchQueue.main.async {
                MainActor.assumeIsolated { self.handOver() }
            }
        }
    }

    /// Events queued and not yet delivered.
    public var waitingCount: Int {
        waiting.withLock { $0.events.count }
    }

    /// Main-queue blocks queued so far: how often the core woke the main thread (tests and
    /// diagnostics).
    public var handOvers: Int {
        waiting.withLock { $0.handOvers }
    }

    @MainActor
    private func handOver() {
        let batch = waiting.withLock { waiting -> [InkEvent] in
            // Cleared together with taking the events: an event pushed after this point queues a
            // hand-over of its own, which runs after this one on the serial main queue.
            waiting.handOverQueued = false
            var batch: [InkEvent] = []
            swap(&batch, &waiting.events)
            return batch
        }
        if !batch.isEmpty {
            deliver(batch)
        }
    }

    private static func isPartial(_ event: InkEvent, of newer: MeetingPartial) -> Bool {
        guard case .meetingPartial(let older) = event else { return false }
        return older.record == newer.record && older.channel == newer.channel
    }
}
