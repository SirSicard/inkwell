// Apple Intelligence coming or going mid-session re-registers polish (plan: syncPolish must run
// again on an availability change). The watch follows a scripted source; the registration it
// drives is the real one, into a running core.
@testable import AppleEngines
import Foundation
import InkBridge
import Synchronization
import XCTest

/// A source whose availability the test sets, and whose "may have changed" the test fires.
private final class ScriptedSource: AppleIntelligenceSource {
    private let state = Mutex<(now: AppleIntelligence, waiting: [@Sendable () -> Void])>((.available, []))

    func set(_ availability: AppleIntelligence, notify: Bool = true) {
        let waiting = state.withLock { state -> [@Sendable () -> Void] in
            state.now = availability
            defer { state.waiting = [] }
            return notify ? state.waiting : []
        }
        waiting.forEach { $0() }
    }

    var observers: Int { state.withLock { $0.waiting.count } }

    func current() -> AppleIntelligence {
        state.withLock { $0.now }
    }

    func observeNextChange(_ changed: @escaping @Sendable () -> Void) {
        state.withLock { $0.waiting.append(changed) }
    }
}

final class AppleIntelligenceWatchTests: XCTestCase {
    func testEachChangeIsReportedOnceAndTheWatchReArms() {
        let source = ScriptedSource()
        let seen = Mutex<[AppleIntelligence]>([])
        let watch = AppleIntelligenceWatch(source: source) { availability in
            seen.withLock { $0.append(availability) }
        }
        XCTAssertEqual(source.observers, 1)
        source.set(.unavailable(.notEnabled))
        XCTAssertEqual(seen.withLock { $0 }, [.unavailable(.notEnabled)])
        XCTAssertEqual(source.observers, 1, "re-armed")
        // A notification with no change reports nothing.
        source.set(.unavailable(.notEnabled))
        XCTAssertEqual(seen.withLock { $0 }.count, 1)
        // A change the observation missed is found by a recheck (the app becoming active).
        source.set(.available, notify: false)
        watch.recheck()
        XCTAssertEqual(seen.withLock { $0 }, [.unavailable(.notEnabled), .available])
        watch.recheck()
        XCTAssertEqual(seen.withLock { $0 }.count, 2)
        watch.stop()
        source.set(.unavailable(.modelNotReady))
        XCTAssertEqual(seen.withLock { $0 }.count, 2, "stopped")
    }

    func testTurningAppleIntelligenceOffAndOnMidSessionLetsGoOfPolishAndRegistersItAgain() throws {
        let core = try TestCore.start()
        defer { core.stop() }
        let engines = AppleEngines(session: core.session)
        let source = ScriptedSource()
        XCTAssertEqual(engines.syncPolish(availability: source.current()), .registered)
        XCTAssertNotNil(core.events.wait(5) { if case .engineRegistered(let e) = $0, e.kind == .llm { e } else { nil } })
        let states = Mutex<[AppleEngineState]>([])
        let watch = AppleIntelligenceWatch(source: source) { availability in
            let state = engines.syncPolish(availability: availability)
            states.withLock { $0.append(state) }
        }
        defer { watch.stop() }

        source.set(.unavailable(.notEnabled))
        XCTAssertNotNil(core.events.wait(5) { if case .engineUnregistered(let e) = $0 { e.id } else { nil } })
        source.set(.available)
        let until = Date().addingTimeInterval(5)
        while core.registered.filter({ $0.kind == .llm }).count < 2, Date() < until {
            Thread.sleep(forTimeInterval: 0.01)
        }
        XCTAssertEqual(core.registered.filter { $0.kind == .llm }.count, 2, "registered again when Apple Intelligence came back")
        XCTAssertEqual(states.withLock { $0 }, [.unavailable(code: 2), .registered])
    }
}
