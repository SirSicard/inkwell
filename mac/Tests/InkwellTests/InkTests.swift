// The ink in the shell: which state the core's events put it in, the Drop that shows it without
// ever taking focus, and the measurement's live mode.
import AppKit
import Foundation
import InkBridge
import InkRenderer
import XCTest

@testable import Inkwell

private func event(_ json: String, file: StaticString = #filePath, line: UInt = #line) -> InkEvent {
    guard let decoded = try? InkEvent.decode(Data(json.utf8)) else {
        XCTFail("not an event: \(json)", file: file, line: line)
        return .unknown(type: "")
    }
    return decoded
}

@MainActor
final class ShellInkTests: XCTestCase {
    func testTheInkFollowsADictation() {
        let store = CoreStore()
        let ink = ShellInk(store: store)
        XCTAssertEqual(ink.state, .idle)
        store.apply([event(#"{"type":"dictation.started","take":0,"edit":false}"#)])
        XCTAssertEqual(ink.state, .dictating)
        store.apply([event(#"{"type":"dictation.stopped"}"#)])
        XCTAssertEqual(ink.state, .dictating, "still wet while the take is transcribed")
        store.apply([event(#"{"type":"dictation.discarded","reason":"speech_too_short"}"#)])
        XCTAssertEqual(ink.state, .idle)
    }

    func testTheInkFollowsAMeetingThroughItsFinalPass() {
        let store = CoreStore()
        let ink = ShellInk(store: store)
        store.apply([event(#"{"type":"meeting.started","record":"r1"}"#)])
        XCTAssertEqual(ink.state, .meeting)
        store.apply([event(#"{"type":"meeting.side_state","record":"r1","channel":"far","state":"zeros"}"#)])
        XCTAssertEqual(ink.state, .problem, "the far end is silent")
        store.apply([event(#"{"type":"meeting.side_state","record":"r1","channel":"far","state":"ok"}"#)])
        XCTAssertEqual(ink.state, .meeting)
        store.apply([event(#"{"type":"meeting.side_state","record":"r1","channel":"mic","state":"zeros"}"#)])
        XCTAssertEqual(ink.state, .meeting, "the problem state shows the far end dead, so only the far end sets it")
        store.apply([event(#"{"type":"meeting.side_state","record":"r1","channel":"far","state":"stopped"}"#)])
        XCTAssertEqual(ink.state, .problem)
        store.apply([event(#"{"type":"meeting.stopped","record":"r1"}"#)])
        XCTAssertEqual(ink.state, .blotting, "capture ended: the final pass")
        store.apply([event(#"{"type":"meeting.finished","record":"r1","revision":2}"#)])
        XCTAssertEqual(ink.state, .idle)
    }

    func testAMeetingOutranksADictation() {
        let store = CoreStore()
        let ink = ShellInk(store: store)
        store.apply([event(#"{"type":"meeting.started","record":"r1"}"#), event(#"{"type":"dictation.started","take":0,"edit":false}"#)])
        XCTAssertEqual(ink.state, .meeting)
    }

    func testAHeldStateWinsOverTheCore() {
        let store = CoreStore()
        let ink = ShellInk(store: store)
        ink.held = .blotting
        store.apply([event(#"{"type":"dictation.started","take":0,"edit":false}"#)])
        XCTAssertEqual(ink.state, .blotting)
        ink.held = nil
        XCTAssertEqual(ink.state, .dictating)
    }

    func testTheDropSaysWhatIsLive() {
        let store = CoreStore()
        let ink = ShellInk(store: store)
        store.apply([event(#"{"type":"dictation.started","take":0,"edit":false}"#)])
        XCTAssertEqual(ink.dropText, DropText(title: "Dictating", detail: "Listening"))
        store.apply([event(#"{"type":"dictation.stopped"}"#)])
        XCTAssertEqual(ink.dropText.detail, "Transcribing")
        for state in InkState.allCases where state.isLive {
            let text = DropText.for(state, dictation: .idle)
            XCTAssertFalse(text.title.isEmpty, "\(state)")
            XCTAssertFalse(text.detail.isEmpty, "\(state)")
        }
        XCTAssertEqual(DropText.for(.problem, dictation: .idle).tone, .alert)
        XCTAssertEqual(DropText.for(.meeting, dictation: .idle).tone, .recording)
        XCTAssertEqual(DropText.for(.blotting, dictation: .idle).tone, .plain)
    }
}

@MainActor
final class DropTests: XCTestCase {
    func testTheDropCanNeverTakeFocus() {
        let panel = DropPanel()
        XCTAssertFalse(panel.canBecomeKey, "typing stays in the app the user is in")
        XCTAssertFalse(panel.canBecomeMain)
        XCTAssertTrue(panel.styleMask.contains(.nonactivatingPanel), "a click does not activate Inkwell")
        XCTAssertFalse(panel.hidesOnDeactivate, "Inkwell is never the active app while the Drop shows")
        XCTAssertTrue(panel.collectionBehavior.contains(.canJoinAllSpaces))
        XCTAssertTrue(panel.collectionBehavior.contains(.fullScreenAuxiliary))
        XCTAssertTrue(panel.collectionBehavior.contains(.ignoresCycle), "not in Cmd-` window cycling")
        XCTAssertTrue(panel.isExcludedFromWindowsMenu)
    }

    func testIdleHidesTheDropAndEveryLiveStateShowsIt() {
        let store = CoreStore()
        let ink = ShellInk(store: store)
        let drop = DropController(ink: ink)
        XCTAssertFalse(drop.isShown)
        for state in InkState.allCases where state.isLive {
            ink.held = state
            drop.update()
            XCTAssertTrue(drop.isShown, "\(state)")
            XCTAssertEqual(drop.inkState, state)
            XCTAssertFalse(drop.panelIsKey, "\(state): shown without taking focus")
        }
        ink.held = .idle
        drop.update()
        XCTAssertFalse(drop.isShown)
        XCTAssertEqual(drop.inkState, .idle)
    }

    func testTheDropFollowsTheStoreWithoutBeingAsked() async throws {
        let store = CoreStore()
        let ink = ShellInk(store: store)
        let drop = DropController(ink: ink)
        store.apply([event(#"{"type":"meeting.started","record":"r1"}"#)])
        // Observation hands the change over on the next turn of the main queue.
        try await Task.sleep(for: .milliseconds(50))
        XCTAssertTrue(drop.isShown)
        XCTAssertEqual(drop.inkState, .meeting)
        store.apply([event(#"{"type":"meeting.finished","record":"r1"}"#)])
        try await Task.sleep(for: .milliseconds(50))
        XCTAssertFalse(drop.isShown)
    }
}

final class LiveMeasurementTests: XCTestCase {
    @MainActor
    func testLiveHoldsTheInkInAMeetingAndIdleHoldsNothing() throws {
        XCTAssertEqual(try XCTUnwrap(Inkwell.Measurement.fromEnvironment(["INK_MEASURE": "live"])).heldInk, .meeting)
        XCTAssertNil(try XCTUnwrap(Inkwell.Measurement.fromEnvironment(["INK_MEASURE": "idle"])).heldInk)
    }

    func testTheDropDemoCyclesEveryLiveStateAndIdle() {
        XCTAssertEqual(DropDemo.sequence.first, .dictating)
        XCTAssertEqual(Set(DropDemo.sequence), Set(InkState.allCases))
        XCTAssertEqual(DropDemo.interval(from: ["INK_DROP_DEMO": "3"]), 3)
        XCTAssertNil(DropDemo.interval(from: [:]))
        XCTAssertNil(DropDemo.interval(from: ["INK_DROP_DEMO": "0"]), "a zero interval would spin")
        XCTAssertNil(DropDemo.interval(from: ["INK_DROP_DEMO": "soon"]))
    }
}
