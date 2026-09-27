// The store folds the core's events into what the screens show. The last test runs the real core:
// its event thread → the relay → the store on the main actor.
import Foundation
import InkBridge
import InkCore
import XCTest

final class CoreStoreTests: XCTestCase {
    @MainActor
    func testReadyStoppedAndAMismatchedCore() {
        let store = CoreStore()
        XCTAssertEqual(store.status, .starting)
        store.apply([event(#"{"type":"core.ready","abi":\#(INK_ABI_VERSION),"version":"1.2.3"}"#)])
        XCTAssertEqual(store.status, .ready(version: "1.2.3"))
        store.apply([event(#"{"type":"core.stopped"}"#)])
        XCTAssertEqual(store.status, .stopped)

        let mismatched = CoreStore()
        mismatched.apply([event(#"{"type":"core.ready","abi":\#(INK_ABI_VERSION + 1),"version":"9.9.9"}"#)])
        guard case .failed = mismatched.status else {
            return XCTFail("a core with another ABI is not ready: \(mismatched.status)")
        }
    }

    @MainActor
    func testAMeetingFromStartToFinish() {
        let store = CoreStore()
        store.apply([
            event(#"{"type":"meeting.started","record":"r1"}"#),
            event(#"{"type":"meeting.side_state","record":"r1","channel":"far","state":"zeros"}"#),
            partialEvent("hel"),
            partialEvent("they sa", channel: "far"),
        ])
        XCTAssertEqual(store.meeting?.record, "r1")
        XCTAssertEqual(store.meeting?.sides[.far], .zeros)
        XCTAssertEqual(store.meeting?.partials, [.mic: "hel", .far: "they sa"])

        store.apply([finalEvent(1)])
        XCTAssertEqual(store.meeting?.partials, [.far: "they sa"], "a final clears its channel's partial")
        XCTAssertEqual(store.meeting?.finals.count, 1)

        store.apply([event(#"{"type":"meeting.stopped","record":"r1"}"#)])
        XCTAssertEqual(store.meeting?.stopping, true, "blotting: still live until the final pass ends")
        XCTAssertEqual(store.meeting?.partials, [:])

        store.apply([event(#"{"type":"meeting.finished","record":"r1","revision":2}"#)])
        XCTAssertNil(store.meeting)
        XCTAssertEqual(store.lastRecord, "r1")
    }

    @MainActor
    func testAnEventForAnotherRecordLeavesTheLiveMeetingAlone() {
        let store = CoreStore()
        store.apply([event(#"{"type":"meeting.started","record":"r1"}"#)])
        store.apply([partialEvent("stray", record: "r0"), finalEvent(1, record: "r0")])
        store.apply([event(#"{"type":"meeting.finished","record":"r0"}"#)])
        XCTAssertEqual(store.meeting, CoreStore.LiveMeeting(record: "r1"))
    }

    @MainActor
    func testADictationFromKeyDownToATooShortTake() {
        let store = CoreStore()
        store.apply([event(#"{"type":"dictation.started"}"#)])
        XCTAssertEqual(store.dictation, .listening)
        store.apply([event(#"{"type":"dictation.stopped"}"#)])
        XCTAssertEqual(store.dictation, .transcribing)
        store.apply([event(#"{"type":"dictation.discarded","reason":"speech_too_short"}"#)])
        XCTAssertEqual(store.dictation, .idle)
        XCTAssertEqual(store.lastDictation, .discarded(.speechTooShort))
        XCTAssertTrue(store.notices.isEmpty, "a too-short take is not something the user must act on")
    }

    @MainActor
    func testModelsWarmFailAndUpdate() {
        let store = CoreStore()
        store.apply([event(#"{"type":"model.warmed","job":"dictation_final","id":"m1"}"#)])
        XCTAssertEqual(store.warmModels, [.dictationFinal: "m1"])

        store.apply([event(#"{"type":"model.update_started","id":"m1","next":"m2"}"#)])
        XCTAssertEqual(store.updatingModels, ["m1"])
        XCTAssertEqual(store.warmModels, [:], "the model is unloaded for its update")

        store.apply([event(#"{"type":"model.update_finished","id":"m1","next":"m2","ok":false,"no_model_warm":true,"message":"disk full"}"#)])
        XCTAssertEqual(store.updatingModels, [])
        XCTAssertEqual(store.notices.map(\.kind), [.modelUpdateFailed(model: "m1")])
        XCTAssertEqual(store.notices.first?.detail, "disk full")

        store.apply([event(#"{"type":"model.warm_failed","job":"dictation_final","message":"not installed"}"#)])
        XCTAssertEqual(store.notices.last?.kind, .modelWarmFailed(.dictationFinal))
    }

    @MainActor
    func testNoticesAreBoundedOldestFirstAndDismissable() {
        let store = CoreStore()
        let failures = (0..<(CoreStore.noticeLimit + 5)).map {
            event(#"{"type":"command.failed","command":"model.warm","message":"failure \#($0)"}"#)
        }
        store.apply(failures)
        XCTAssertEqual(store.notices.count, CoreStore.noticeLimit)
        XCTAssertEqual(store.notices.first?.detail, "failure 5", "the oldest go first")
        XCTAssertEqual(store.notices.map(\.id), store.notices.map(\.id).sorted())

        let first = store.notices[0].id
        store.dismissNotice(first)
        XCTAssertFalse(store.notices.contains { $0.id == first })
    }

    @MainActor
    func testAnEventThisBuildCannotReadIsAMismatchedBuild() {
        let store = CoreStore()
        store.apply([event(#"{"type":"from.the.future"}"#)])
        XCTAssertEqual(store.notices.map(\.kind), [.mismatchedBuild(type: "from.the.future")])
    }

    @MainActor
    func testCoreStoppedEndsTheMeetingAndTheDictation() {
        let store = CoreStore()
        store.apply([
            event(#"{"type":"meeting.started","record":"r1"}"#),
            event(#"{"type":"dictation.started"}"#),
            event(#"{"type":"core.stopped"}"#),
        ])
        XCTAssertNil(store.meeting)
        XCTAssertEqual(store.dictation, .idle)
    }

    /// The real core: ready, a command that fails (no engine in a build with the engine features
    /// off), and shutdown, all reaching the store on the main actor through the relay.
    @MainActor
    func testTheStoreFollowsTheRealCoreThroughTheRelay() throws {
        let data = FileManager.default.temporaryDirectory
            .appendingPathComponent("inkwell-store-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: data) }
        let store = CoreStore()
        let direct = Events()
        let relay = EventRelay { store.apply($0) }
        let session = try InkSession.start(
            InkConfig(dataDir: data.path, logLevel: "warn"),
            onEvent: { direct.record($0); relay.push($0) }
        )

        spin(until: { store.status != .starting })
        guard case .ready = store.status else {
            session.shutdown()
            return XCTFail("the core did not become ready: \(store.status)")
        }

        try session.command(["cmd": "model.warm", "job": "dictation_final"])
        spin(until: { store.notices.contains { $0.kind == .modelWarmFailed(.dictationFinal) } })
        XCTAssertEqual(store.warmModels, [:])

        session.shutdown()
        spin(until: { store.status == .stopped })
        XCTAssertEqual(store.status, .stopped)
        XCTAssertEqual(store.eventsApplied, direct.all.count, "every event the core sent reached the store")
        XCTAssertLessThanOrEqual(store.batchesApplied, store.eventsApplied)
    }

    /// Runs the main run loop until `done` or 10 s.
    @MainActor
    private func spin(until done: () -> Bool) {
        let deadline = Date().addingTimeInterval(10)
        while !done() && Date() < deadline {
            RunLoop.main.run(until: Date().addingTimeInterval(0.01))
        }
    }
}
