// Per-app call recording on the Drop: the offer sets an app's policy (Always for, Never for), a
// call its app's Always recorded shows as such with Stop, and Stop and delete for its first minute
// (one scheduled wake ends it), and an Always app the core asks about instead says why. Wording
// and layout included.
import AppKit
import Foundation
import InkBridge
import InkRenderer
import SwiftUI
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
private final class Sent {
    var commands: [CoreCommand] = []
    lazy var send: SendCommand = { [unowned self] in self.commands.append($0) }
}

private struct FakeCalendar: CalendarAccess {
    func state() -> CardState { .notAsked }
    func request(done: @escaping @MainActor @Sendable () -> Void) {}
}

private struct NoTitle: CallTitles {
    func titleNow(_ now: Date) -> String? { nil }
}

/// No app is installed: names come from the core, else from AppIdentity.
private struct NoApps: AppDirectory {
    func app(bundleID: String) -> (name: String, icon: NSImage)? { nil }
}

/// Counts the waits a model schedules, and sleeps for real (shortened deadlines in the tests).
private final class Sleeps: @unchecked Sendable {
    private let lock = NSLock()
    private var count = 0
    var scheduled: Int { lock.withLock { count } }

    var sleep: @Sendable (Duration) async throws -> Void {
        { [self] duration in
            lock.withLock { count += 1 }
            try await Task.sleep(for: duration)
        }
    }
}

private let zoom = "us.zoom.xos"

private func detected(_ app: String = zoom, name: String = "Zoom", message: String? = nil) -> InkEvent {
    let extra = message.map { #","message":"\#($0)""# } ?? ""
    return event(#"{"type":"meeting.detected","app":"\#(app)","app_name":"\#(name)"\#(extra)}"#)
}

private func started(auto: Bool, deleteUntil: Date?, record: String = "r1") -> InkEvent {
    let until = deleteUntil.map { #","delete_until_unix_ms":\#(Int64($0.timeIntervalSince1970 * 1000))"# } ?? ""
    let autoField = auto ? #","auto":true"# : ""
    return event(#"{"type":"meeting.started","record":"\#(record)","app":"\#(zoom)","app_name":"Zoom","far_end":"app"\#(autoField)\#(until)}"#)
}

private func callsEvent(_ apps: String, default policy: String = "ask", message: String? = nil, ref: String? = nil) -> InkEvent {
    var extra = message.map { #","message":"\#($0)""# } ?? ""
    extra += ref.map { #","ref":"\#($0)""# } ?? ""
    return event(#"{"type":"meetings.calls","default":"\#(policy)","apps":[\#(apps)]\#(extra)}"#)
}

@MainActor
final class CallPolicyDropTests: XCTestCase {
    private func models(_ sent: Sent, now: @escaping () -> Date = Date.init, sleep: Sleeps = Sleeps())
        -> (store: CoreStore, ink: ShellInk, screens: ScreenModels)
    {
        let store = CoreStore()
        let screens = ScreenModels(send: sent.send, calendar: FakeCalendar(), apps: NoApps(), callTitles: NoTitle())
        // The screens' meeting model, with the clock and the wait under the test's control.
        let meetings = MeetingModel(send: sent.send, titles: NoTitle(), now: now, sleep: sleep.sleep)
        let ink = ShellInk(store: store, meetings: meetings, calls: screens.calls)
        return (store, ink, screens)
    }

    /// The offer: Record and Not this one, then the app's policy, by its name.
    func testTheOfferSetsTheAppsPolicyFromTheDrop() {
        let sent = Sent()
        let (store, ink, screens) = models(sent)
        store.apply([detected()])
        let text = ink.dropText
        XCTAssertEqual(text.title, "Zoom opened the microphone")
        XCTAssertEqual(text.detail, DropText.consentLine)
        XCTAssertEqual(text.actions.map(\.title), ["Record this call", "Not this one", "Always for Zoom", "Never for Zoom"])
        XCTAssertEqual(text.actions.compactMap(\.hint).count, 2, "VoiceOver hears what Always and Never do")

        // Always for Zoom: from now on, and this call now (the core keeps it offered until started).
        screens.performDropAction(.always(app: zoom, name: "Zoom")) { _ in }
        XCTAssertEqual(sent.commands.suffix(2), [
            .meetingsCallsSet(app: zoom, policy: "always", replaceUnreadable: false, ref: "calls:1"),
            .meetingStart(app: zoom, title: nil),
        ])
        // Never for Zoom: only the policy; the core withdraws the offer.
        sent.commands = []
        screens.performDropAction(.never(app: zoom, name: "Zoom")) { _ in }
        XCTAssertEqual(sent.commands, [.meetingsCallsSet(app: zoom, policy: "never", replaceUnreadable: false, ref: "calls:2")])
        XCTAssertEqual(
            CoreCommand.meetingsCallsSet(app: zoom, policy: "never", replaceUnreadable: false, ref: "calls:2").json,
            #"{"app":"us.zoom.xos","cmd":"meetings.calls.set","id":"calls:2","policy":"never"}"#)
        XCTAssertEqual(
            CoreCommand.meetingsCallsSet(app: zoom, policy: "ask", replaceUnreadable: true, ref: "calls:3").json,
            #"{"app":"us.zoom.xos","cmd":"meetings.calls.set","id":"calls:3","policy":"ask","replace_unreadable":true}"#)
        XCTAssertEqual(CoreCommand.meetingsCallsList(ref: "calls:4").json, #"{"cmd":"meetings.calls.list","id":"calls:4"}"#)
        XCTAssertEqual(CoreCommand.meetingDiscard.json, #"{"cmd":"meeting.discard","id":"meeting.discard"}"#)
    }

    /// An app that is Always already (offered because the user stopped by hand, or chose Always
    /// during this call) is never offered "Always for" again.
    func testAnAppAlreadyAlwaysIsNotOfferedAlwaysAgain() {
        let sent = Sent()
        let (store, ink, screens) = models(sent)
        screens.calls.apply(callsEvent(#"{"app":"us.zoom.xos","app_name":"Zoom","policy":"always","chosen":true}"#))
        store.apply([detected()])
        XCTAssertEqual(ink.dropText.actions.map(\.title), ["Record this call", "Not this one", "Never for Zoom"])
    }

    /// NotAlone: an Always app whose own sound can't be recorded alone is asked about, saying
    /// why in the shell's words, with the reminder to tell the others.
    func testAnAlwaysAppThatCantBeHeardAloneIsAskedAboutAndSaysWhy() {
        let sent = Sent()
        let (store, ink, _) = models(sent)
        store.apply([detected(message: DropText.notAloneMessage)])
        let text = ink.dropText
        XCTAssertEqual(text.title, "Zoom opened the microphone")
        XCTAssertEqual(text.detail, "Inkwell can't hear it alone: recording takes in everything this Mac plays. Tell the others you are recording.")
        XCTAssertEqual(text.actions.map(\.title), ["Record this call", "Not this one", "Never for Zoom"])
        // Another reason (a start that failed): said shortly, the platform's words left to the log.
        store.apply([detected(message: "couldn't start recording by itself: the tap failed")])
        XCTAssertEqual(ink.dropText.detail, "Inkwell couldn't start recording it by itself. Tell the others you are recording.")
        for word in ["invisible", "undetectable", "hidden", "secret"] {
            XCTAssertFalse((text.title + text.detail).lowercased().contains(word), word)
        }
    }

    /// The core names an app with no name "an app": the Drop says it as a sentence, and "this app".
    func testANamelessAppReadsAsThisApp() {
        let sent = Sent()
        let (store, ink, _) = models(sent)
        store.apply([detected("com.example.x", name: "an app")])
        XCTAssertEqual(ink.dropText.title, "An app opened the microphone")
        XCTAssertEqual(ink.dropText.actions.suffix(2).map(\.title), ["Always for this app", "Never for this app"])
    }

    /// A call its app's Always started: said as such, the reminder kept, Stop and Stop and delete
    /// for the first minute; then Stop alone and the latest line, ended by one scheduled wake.
    func testAnAutoStartedCallShowsStopAndDeleteUntilItsDeadline() async throws {
        let sent = Sent()
        let sleeps = Sleeps()
        let start = Date()
        let (store, ink, _) = models(sent, now: { start }, sleep: sleeps)
        let meetings = try XCTUnwrap(ink.meetings)
        let first = [started(auto: true, deleteUntil: start.addingTimeInterval(0.3))]
        store.apply(first)
        first.forEach(meetings.apply)
        var text = ink.dropText
        XCTAssertEqual(text.title, "● Recording Zoom automatically")
        XCTAssertEqual(text.detail, "Always is on for Zoom. Tell the others you are recording.")
        XCTAssertEqual(text.tone, .recording)
        XCTAssertEqual(text.actions, [.stop, .stopAndDelete])
        XCTAssertEqual(text.actions.map(\.title), ["Stop", "Stop and delete"])
        // A line said in the first minute: the reminder stays while Stop and delete is there.
        let line = event(#"{"type":"meeting.final","record":"r1","channel":"far","start_ms":0,"end_ms":900,"text":"can everyone hear me"}"#)
        store.apply([line])
        XCTAssertEqual(ink.dropText.detail, "Always is on for Zoom. Tell the others you are recording.")

        try await Task.sleep(for: .milliseconds(700))
        text = ink.dropText
        XCTAssertEqual(text.actions, [.stop], "past the minute only Stop is left")
        XCTAssertEqual(text.detail, "can everyone hear me")
        XCTAssertEqual(text.title, "● Recording Zoom automatically", "still said as automatic")
        XCTAssertEqual(sleeps.scheduled, 1, "one deadline, never a poll")

        // Stop from the Drop is meeting.stop.
        ink.meetings?.perform(.stop, permissions: PermissionsModel(send: sent.send, calendar: FakeCalendar()))
        XCTAssertEqual(sent.commands.last, .meetingStop)
    }

    /// A start the user made is shown as before: no Stop and delete, whatever the core allows.
    func testAUserStartedCallHasNoStopAndDelete() {
        let sent = Sent()
        let start = Date()
        let (store, ink, _) = models(sent, now: { start })
        let events = [started(auto: false, deleteUntil: start.addingTimeInterval(60))]
        store.apply(events)
        events.forEach { ink.meetings?.apply($0) }
        XCTAssertEqual(ink.dropText.title, "● REC · Zoom")
        XCTAssertTrue(ink.dropText.actions.isEmpty)
        XCTAssertFalse(ink.meetings?.canDiscard("r1") ?? true)
    }

    /// Stop and delete: the Drop says it is deleting through the stop, and the meeting is gone
    /// when the core says so, never the last record.
    func testStopAndDeleteDeletesAndTheDropGoes() {
        let sent = Sent()
        let start = Date()
        let (store, ink, _) = models(sent, now: { start })
        let meetings = try! XCTUnwrap(ink.meetings)
        let drop = DropController(ink: ink)
        func both(_ e: InkEvent) {
            store.apply([e])
            meetings.apply(e)
            drop.update()
        }
        both(started(auto: true, deleteUntil: start.addingTimeInterval(60)))
        XCTAssertTrue(drop.isShown)
        drop.onAction = { meetings.perform($0, permissions: PermissionsModel(send: sent.send, calendar: FakeCalendar())) }
        drop.contentView.press(.stopAndDelete)
        XCTAssertEqual(sent.commands.last, .meetingDiscard)
        drop.update()
        XCTAssertEqual(drop.shownText?.title, "Stop and delete")
        XCTAssertEqual(drop.shownText?.detail, "Deleting this recording")
        XCTAssertEqual(drop.shownText?.actions, [])
        both(event(#"{"type":"meeting.stopped","record":"r1"}"#))
        XCTAssertEqual(drop.shownText?.detail, "Deleting this recording", "not the final pass: none runs")
        both(event(#"{"type":"meeting.discarded","record":"r1","audio_left":false,"scrubbed":true}"#))
        XCTAssertNil(store.meeting)
        XCTAssertNil(store.lastRecord, "a deleted meeting is never the last record")
        XCTAssertFalse(drop.isShown)
        XCTAssertNil(meetings.discarding)
    }

    /// Past the minute the core refuses it: said in the Drop, and only Stop is left.
    func testAStopAndDeleteRefusedAfterTheMinuteLeavesOnlyStop() {
        let sent = Sent()
        let start = Date()
        let (store, ink, _) = models(sent, now: { start })
        let meetings = try! XCTUnwrap(ink.meetings)
        let first = started(auto: true, deleteUntil: start.addingTimeInterval(60))
        store.apply([first])
        meetings.apply(first)
        meetings.discard()
        meetings.apply(event(#"{"type":"command.failed","command":"meeting.discard","id":"meeting.discard","code":"delete_window_over","message":"the first minute is over: stop the meeting, then delete it from the library"}"#))
        XCTAssertEqual(ink.dropText.detail, "The first minute is over. Stop it, then delete it in the Library.")
        XCTAssertEqual(ink.dropText.tone, .alert)
        XCTAssertEqual(ink.dropText.actions, [.stop])
        meetings.discard()
        XCTAssertEqual(sent.commands.filter { $0 == .meetingDiscard }.count, 1, "not offered, not sent")
    }

    /// The far end going silent in an automatic call keeps its Stop and Stop and delete.
    func testTheWarningKeepsAnAutoCallsStop() {
        let sent = Sent()
        let start = Date()
        let (store, ink, _) = models(sent, now: { start })
        let first = started(auto: true, deleteUntil: start.addingTimeInterval(60))
        store.apply([first, event(#"{"type":"meeting.side_state","record":"r1","channel":"far","state":"zeros"}"#)])
        ink.meetings?.apply(first)
        XCTAssertEqual(ink.state, .problem)
        XCTAssertEqual(ink.dropText.actions, [.stop, .stopAndDelete])
    }

    /// A deadline already past when the start arrives offers no Stop and delete at all.
    func testADeadlineAlreadyPastOffersNoDelete() {
        let sent = Sent()
        let sleeps = Sleeps()
        let start = Date()
        let (store, ink, _) = models(sent, now: { start }, sleep: sleeps)
        let first = started(auto: true, deleteUntil: start.addingTimeInterval(-1))
        store.apply([first])
        ink.meetings?.apply(first)
        XCTAssertEqual(ink.dropText.actions, [.stop])
        XCTAssertEqual(sleeps.scheduled, 0)
    }

    /// A choice from the Drop that failed is said there; a new offer clears it.
    func testADropChoiceThatFailedIsSaidInTheDrop() {
        let sent = Sent()
        let (store, ink, screens) = models(sent)
        store.apply([detected()])
        screens.performDropAction(.never(app: zoom, name: "Zoom")) { _ in }
        screens.calls.apply(event(#"{"type":"command.failed","command":"meetings.calls.set","id":"calls:1","message":"database is locked"}"#))
        XCTAssertEqual(ink.dropText.detail, "Couldn't save that: database is locked")
        XCTAssertNil(screens.calls.failure, "said where it was asked")
        XCTAssertTrue(screens.handles(CommandFailed.decodeForTest(command: "meetings.calls.set")))
        screens.calls.apply(detected())
        XCTAssertEqual(ink.dropText.detail, DropText.consentLine)
    }

    /// The offer's four buttons, with a long name, fit the Drop in two rows: none past its edge,
    /// none over another, and the panel tall enough for them. INK_DROP_RENDER draws the states.
    func testTheOffersButtonsFitTheDrop() throws {
        let store = CoreStore()
        let ink = ShellInk(store: store)
        let drop = DropController(ink: ink)
        let renders = ProcessInfo.processInfo.environment["INK_DROP_RENDER"].map { URL(fileURLWithPath: $0, isDirectory: true) }
        if let renders { try FileManager.default.createDirectory(at: renders, withIntermediateDirectories: true) }
        let cases: [(String, [InkEvent])] = [
            ("offer", [detected()]),
            ("offer-long", [detected("com.microsoft.teams2", name: "Microsoft Teams (work or school)")]),
            ("start-failed", [detected("com.microsoft.teams2", name: "Microsoft Teams (work or school)", message: "couldn't start recording by itself: the other side's sound could not be opened")]),
            ("not-alone", [detected("com.microsoft.teams2", name: "Microsoft Teams (work or school)", message: DropText.notAloneMessage)]),
        ]
        for (name, events) in cases {
            store.apply([event(#"{"type":"meeting.detection","listening":false}"#)])
            store.apply(events)
            drop.update()
            let content = drop.contentView
            let size = content.fittingPanelSize
            content.setFrameSize(size)
            content.layoutSubtreeIfNeeded()
            let buttons = content.shownButtons
            XCTAssertEqual(buttons.count, drop.shownText?.actions.count, name)
            let frames = buttons.map { $0.convert($0.bounds, to: content) }
            for (button, frame) in zip(buttons, frames) {
                XCTAssertTrue(content.bounds.insetBy(dx: 0, dy: 4).contains(frame), "\(name): \(button.title) at \(frame) is outside \(content.bounds)")
                XCTAssertGreaterThanOrEqual(frame.minX, DropLayout.inkWidth - 0.5, "\(name): \(button.title) is over the orb")
            }
            for (i, a) in frames.enumerated() {
                for b in frames[(i + 1)...] {
                    XCTAssertFalse(a.insetBy(dx: 1, dy: 1).intersects(b), "\(name): buttons overlap")
                }
            }
            XCTAssertGreaterThan(size.height, DropLayout.sizeWithActions.height, "\(name): two rows of buttons")
            let lines = content.detailLines
            XCTAssertLessThanOrEqual(lines.needed, lines.allowed, "\(name): the line is cut, and the reminder with it")
            if let renders {
                for appearance in [NSAppearance.Name.aqua, .darkAqua] {
                    content.appearance = NSAppearance(named: appearance)
                    let rep = try XCTUnwrap(content.bitmapImageRepForCachingDisplay(in: content.bounds))
                    content.cacheDisplay(in: content.bounds, to: rep)
                    let file = renders.appendingPathComponent("drop-\(name)-\(appearance == .aqua ? "light" : "dark").png")
                    try XCTUnwrap(rep.representation(using: .png, properties: [:])).write(to: file)
                }
            }
        }
    }

    /// Renders the automatic call's Drop (INK_DROP_RENDER), in its first minute and after.
    func testRenderTheAutomaticCallsDrop() throws {
        guard let folder = ProcessInfo.processInfo.environment["INK_DROP_RENDER"] else {
            throw XCTSkip("INK_DROP_RENDER is not set")
        }
        let out = URL(fileURLWithPath: folder, isDirectory: true)
        try FileManager.default.createDirectory(at: out, withIntermediateDirectories: true)
        let content = DropContentView()
        let texts = [
            ("auto", DropText.for(.meeting, dictation: .idle, meeting: autoMeeting(), offer: nil, systemAudioOff: false, deletable: true)),
            ("auto-later", DropText.for(.meeting, dictation: .idle, meeting: autoMeeting(), offer: nil, systemAudioOff: false)),
        ]
        for (name, text) in texts {
            content.show(text)
            content.setFrameSize(content.fittingPanelSize)
            content.layoutSubtreeIfNeeded()
            for appearance in [NSAppearance.Name.aqua, .darkAqua] {
                content.appearance = NSAppearance(named: appearance)
                let rep = try XCTUnwrap(content.bitmapImageRepForCachingDisplay(in: content.bounds))
                content.cacheDisplay(in: content.bounds, to: rep)
                try XCTUnwrap(rep.representation(using: .png, properties: [:]))
                    .write(to: out.appendingPathComponent("drop-\(name)-\(appearance == .aqua ? "light" : "dark").png"))
            }
        }
    }

    private func autoMeeting() -> CoreStore.LiveMeeting {
        var meeting = CoreStore.LiveMeeting(record: "r1")
        meeting.app = zoom
        meeting.appName = "Zoom"
        meeting.auto = true
        return meeting
    }
}

private extension CommandFailed {
    static func decodeForTest(command: String) -> CommandFailed {
        let data = Data(#"{"type":"command.failed","command":"\#(command)","message":"x"}"#.utf8)
        return try! JSONDecoder().decode(CommandFailed.self, from: data)
    }
}
