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

/// What a model logged.
private final class LoggedLines: @unchecked Sendable {
    private let lock = NSLock()
    private var all: [String] = []
    var lines: [String] { lock.withLock { all } }
    var log: ScreenLog { ScreenLog { [self] line in lock.withLock { all.append(line) } } }
}

private struct NoTitle: CallTitles {
    func titleNow(_ now: Date) -> String? { nil }
}

/// No app is installed: names come from the core, else from AppIdentity.
private struct NoApps: AppDirectory {
    func app(bundleID: String) -> (name: String, icon: NSImage)? { nil }
}

/// The waits a model schedules, held until the test lets them end (`fire`); a cancelled one ends
/// at once, throwing, as Task.sleep does. Nothing here sleeps for real.
private final class Sleeps: @unchecked Sendable {
    private let lock = NSLock()
    private var count = 0
    private var waiting: [Int: CheckedContinuation<Void, Error>] = [:]
    private(set) var asked: [Duration] = []
    var scheduled: Int { lock.withLock { count } }
    var pending: Int { lock.withLock { waiting.count } }

    var sleep: @Sendable (Duration) async throws -> Void {
        { [self] duration in
            let id = lock.withLock {
                count += 1
                asked.append(duration)
                return count
            }
            try await withTaskCancellationHandler {
                try await withCheckedThrowingContinuation { continuation in
                    let cancelled = lock.withLock { () -> Bool in
                        if Task.isCancelled { return true }
                        waiting[id] = continuation
                        return false
                    }
                    if cancelled { continuation.resume(throwing: CancellationError()) }
                }
            } onCancel: {
                lock.withLock { waiting.removeValue(forKey: id) }?.resume(throwing: CancellationError())
            }
        }
    }

    /// Ends every wait still held, as their deadlines passing would.
    func fire() {
        let all = lock.withLock { () -> [CheckedContinuation<Void, Error>] in
            let all = Array(waiting.values)
            waiting = [:]
            return all
        }
        all.forEach { $0.resume() }
    }
}

/// Lets the main actor run what a wake or an answer queued.
@MainActor
private func settle() async {
    for _ in 0..<5 { await Task.yield() }
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
        XCTAssertEqual(sent.commands.last, .meetingsCallsSet(app: zoom, policy: "always", replaceUnreadable: false, ref: "calls:1"))
        screens.calls.apply(callsEvent(#"{"app":"us.zoom.xos","app_name":"Zoom","policy":"always","chosen":true}"#, ref: "calls:1"))
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
        let first = [started(auto: true, deleteUntil: start.addingTimeInterval(60))]
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

        await settle()
        XCTAssertEqual(sleeps.scheduled, 1, "one deadline, never a poll")
        XCTAssertEqual(sleeps.asked, [.seconds(60)], "until the core's delete_until_unix_ms")
        sleeps.fire()
        await settle()
        text = ink.dropText
        XCTAssertEqual(text.actions, [.stop], "past the minute only Stop is left")
        XCTAssertEqual(text.detail, "can everyone hear me")
        XCTAssertEqual(text.title, "● Recording Zoom automatically", "still said as automatic")
        XCTAssertEqual(sleeps.scheduled, 1, "nothing woke again")

        // Stop from the Drop is meeting.stop.
        ink.meetings?.perform(.stop, permissions: PermissionsModel(send: sent.send, calendar: FakeCalendar()))
        XCTAssertEqual(sent.commands.last, .meetingStop)
    }

    /// A second meeting while the first's minute is pending: the first's wake is cancelled, and
    /// Stop and delete follows the new one. A stop before the deadline cancels it too.
    func testANewMeetingOrAStopCancelsThePendingWake() async throws {
        let sent = Sent()
        let sleeps = Sleeps()
        let start = Date()
        let (_, ink, _) = models(sent, now: { start }, sleep: sleeps)
        let meetings = try XCTUnwrap(ink.meetings)
        meetings.apply(started(auto: true, deleteUntil: start.addingTimeInterval(60), record: "r1"))
        await settle()
        meetings.apply(started(auto: true, deleteUntil: start.addingTimeInterval(60), record: "r2"))
        await settle()
        XCTAssertEqual(sleeps.scheduled, 2)
        XCTAssertEqual(sleeps.pending, 1, "the first wake was cancelled")
        XCTAssertFalse(meetings.canDiscard("r1"))
        XCTAssertTrue(meetings.canDiscard("r2"))
        meetings.apply(event(#"{"type":"meeting.stopped","record":"r2"}"#))
        await settle()
        XCTAssertFalse(meetings.canDiscard("r2"), "stopped: only the final pass is left")
        XCTAssertEqual(sleeps.pending, 0, "its wake was cancelled")
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

    /// Notes typed in a meeting being stopped and deleted are never saved into it.
    func testNotesAreNotSavedIntoAMeetingBeingDeleted() {
        let sent = Sent()
        let start = Date()
        let screens = ScreenModels(send: sent.send, calendar: FakeCalendar(), apps: NoApps(), callTitles: NoTitle())
        let first = started(auto: true, deleteUntil: start.addingTimeInterval(60))
        screens.apply([first])
        screens.live.notesEdited("a thought of my own", caretParagraph: 0)
        screens.meetings.discard()
        XCTAssertEqual(sent.commands.last, .meetingDiscard)
        screens.apply([event(#"{"type":"meeting.stopped","record":"r1"}"#)])
        screens.apply([event(#"{"type":"meeting.discarded","record":"r1","audio_left":false,"scrubbed":true}"#)])
        XCTAssertFalse(sent.commands.contains { if case .noteAdd = $0 { true } else { false } }, "no note.add for a record being deleted")
        // An ordinary stop still saves them.
        sent.commands = []
        screens.apply([started(auto: true, deleteUntil: start.addingTimeInterval(60), record: "r2")])
        screens.live.notesEdited("kept", caretParagraph: 0)
        screens.apply([event(#"{"type":"meeting.stopped","record":"r2"}"#)])
        XCTAssertTrue(sent.commands.contains { if case .noteAdd = $0 { true } else { false } })
    }

    /// A delete refused after the stop keeps the meeting, and its notes are saved then.
    func testNotesAreSavedWhenTheDeleteIsRefusedAfterTheStop() {
        let sent = Sent()
        let start = Date()
        let screens = ScreenModels(send: sent.send, calendar: FakeCalendar(), apps: NoApps(), callTitles: NoTitle())
        screens.apply([started(auto: true, deleteUntil: start.addingTimeInterval(60))])
        screens.live.notesEdited("kept after all", caretParagraph: 0)
        screens.meetings.discard()
        screens.apply([event(#"{"type":"meeting.stopped","record":"r1"}"#)])
        XCTAssertFalse(sent.commands.contains { if case .noteAdd = $0 { true } else { false } }, "held while the delete is pending")
        screens.apply([event(#"{"type":"command.failed","command":"meeting.discard","id":"meeting.discard","message":"the meeting had already stopped, or failed: delete it from the library once it is there"}"#)])
        XCTAssertTrue(sent.commands.contains { if case .noteAdd = $0 { true } else { false } }, "refused: saved")
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
        // The transcript moving on takes the refusal's words away; the next offer never shows them.
        let line = event(#"{"type":"meeting.final","record":"r1","channel":"far","start_ms":0,"end_ms":900,"text":"next item"}"#)
        store.apply([line])
        meetings.apply(line)
        XCTAssertEqual(ink.dropText.detail, "next item")
        meetings.discard()
        let finished = event(#"{"type":"meeting.finished","record":"r1","revision":2}"#)
        store.apply([finished, detected()])
        meetings.apply(finished)
        meetings.apply(detected())
        XCTAssertEqual(ink.dropText.detail, DropText.consentLine)
    }

    /// Another refusal (the store failed) keeps the button for its minute; one that comes after
    /// the meeting ended shows nowhere; a core that stopped while deleting leaves nothing pending.
    func testOtherDiscardRefusalsKeepTheButtonAndLateOnesShowNowhere() {
        let sent = Sent()
        let start = Date()
        let logged = LoggedLines()
        let store = CoreStore()
        let meetings = MeetingModel(send: sent.send, titles: NoTitle(), now: { start }, log: logged.log, sleep: Sleeps().sleep)
        let ink = ShellInk(store: store, meetings: meetings)
        let first = started(auto: true, deleteUntil: start.addingTimeInterval(60))
        store.apply([first])
        meetings.apply(first)
        meetings.discard()
        meetings.apply(event(#"{"type":"command.failed","command":"meeting.discard","id":"meeting.discard","message":"database is locked"}"#))
        XCTAssertEqual(ink.dropText.detail, "Couldn't delete it: database is locked")
        XCTAssertEqual(ink.dropText.actions, [.stop, .stopAndDelete], "pressed again, it may pass")
        meetings.discard()
        meetings.apply(event(#"{"type":"meeting.failed","record":"r1","message":"capture stopped"}"#))
        XCTAssertNil(meetings.discarding)
        meetings.apply(event(#"{"type":"command.failed","command":"meeting.discard","id":"meeting.discard","message":"the meeting had already stopped, or failed"}"#))
        XCTAssertNil(meetings.failure(on: .drop), "its meeting had ended")
        XCTAssertTrue(logged.lines.last?.contains("nothing shows it") == true)
        // A core that stops while deleting.
        let second = started(auto: true, deleteUntil: start.addingTimeInterval(60), record: "r2")
        meetings.apply(second)
        meetings.discard()
        meetings.apply(event(#"{"type":"core.stopped"}"#))
        XCTAssertNil(meetings.discarding)
        XCTAssertFalse(meetings.canDiscard("r2"))
    }

    /// "Always for": the call is recorded once Always is saved; a save that failed records nothing
    /// and says so on the offer, which stays; over a list the core can't read nothing is sent.
    func testAlwaysForRecordsOnlyOnceItIsSaved() async {
        let sent = Sent()
        let (store, ink, screens) = models(sent)
        store.apply([detected()])
        screens.calls.apply(detected())
        screens.performDropAction(.always(app: zoom, name: "Zoom")) { _ in }
        XCTAssertEqual(sent.commands, [.meetingsCallsSet(app: zoom, policy: "always", replaceUnreadable: false, ref: "calls:1")])
        screens.calls.apply(callsEvent(#"{"app":"us.zoom.xos","app_name":"Zoom","policy":"always","chosen":true}"#, ref: "calls:1"))
        XCTAssertEqual(sent.commands.last, .meetingStart(app: zoom, title: nil), "saved: this call is recorded")

        sent.commands = []
        screens.performDropAction(.always(app: zoom, name: "Zoom")) { _ in }
        screens.calls.apply(event(#"{"type":"command.failed","command":"meetings.calls.set","id":"calls:2","message":"database is locked"}"#))
        XCTAssertFalse(sent.commands.contains(.meetingStart(app: zoom, title: nil)), "not saved: nothing recorded")
        XCTAssertEqual(ink.dropText.detail, "Couldn't save that: database is locked")

        // Recorded by hand after all: the failure goes with the offer, and never shows on a later
        // call's Drop.
        let taken = started(auto: false, deleteUntil: nil)
        store.apply([taken])
        screens.calls.apply(taken)
        XCTAssertNil(screens.calls.dropFailure)
        let finished = event(#"{"type":"meeting.finished","record":"r1","revision":2}"#)
        let auto = started(auto: true, deleteUntil: nil, record: "r2")
        store.apply([finished, auto])
        screens.calls.apply(auto)
        XCTAssertEqual(ink.dropText.detail, DropText.autoReminder("Zoom"))

        // The call ended before Always was saved: nothing is recorded for it.
        let over = event(#"{"type":"meeting.finished","record":"r2","revision":2}"#)
        store.apply([over, detected()])
        screens.calls.apply(detected())
        sent.commands = []
        screens.performDropAction(.always(app: zoom, name: "Zoom")) { _ in }
        let ended = event(#"{"type":"meeting.detection_ended","app":"us.zoom.xos","dismissed":false}"#)
        store.apply([ended])
        screens.calls.apply(ended)
        guard case .meetingsCallsSet(_, _, _, let ref)? = sent.commands.last else { return XCTFail("no set sent") }
        screens.calls.apply(callsEvent(#"{"app":"us.zoom.xos","app_name":"Zoom","policy":"always","chosen":true}"#, ref: ref))
        XCTAssertEqual(sent.commands.count, 1, "saved, not recorded")

        sent.commands = []
        store.apply([detected()])
        screens.calls.apply(detected())
        screens.calls.apply(callsEvent("", message: "the stored choices cannot be read"))
        screens.performDropAction(.always(app: zoom, name: "Zoom")) { _ in }
        XCTAssertTrue(sent.commands.isEmpty)
        XCTAssertEqual(ink.dropText.detail, CallPolicyModel.unreadableFromDrop)
    }

    /// A failed Stop stays said while its call records; a refused Stop and delete only until the
    /// transcript moves on; an earlier meeting's end never takes the live one's.
    func testDropFailuresLastAsLongAsTheyMean() {
        let sent = Sent()
        let start = Date()
        let (store, ink, _) = models(sent, now: { start })
        let meetings = try! XCTUnwrap(ink.meetings)
        func both(_ e: InkEvent) {
            store.apply([e])
            meetings.apply(e)
        }
        both(started(auto: true, deleteUntil: start.addingTimeInterval(60)))
        meetings.perform(.stop, permissions: PermissionsModel(send: sent.send, calendar: FakeCalendar()))
        both(event(#"{"type":"command.failed","command":"meeting.stop","id":"meeting.stop","message":"the capture did not answer"}"#))
        XCTAssertEqual(ink.dropText.detail, "Couldn't stop: the capture did not answer")
        both(event(#"{"type":"meeting.final","record":"r1","channel":"far","start_ms":0,"end_ms":900,"text":"still talking"}"#))
        XCTAssertEqual(ink.dropText.detail, "Couldn't stop: the capture did not answer", "the call still records: said until it ends")
        // The previous meeting's final pass ending changes nothing here.
        meetings.apply(event(#"{"type":"meeting.finished","record":"r0","revision":2}"#))
        XCTAssertEqual(ink.dropText.detail, "Couldn't stop: the capture did not answer")
        both(event(#"{"type":"meeting.finished","record":"r1","revision":2}"#))
        XCTAssertNil(meetings.failure(on: .drop))
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

    /// A double click on "Always for": the second press does nothing while the first is saved, and
    /// the buttons stay where they were (no "Never for" slides under the pointer).
    func testADoubleClickOnAlwaysSavesOnce() {
        let sent = Sent()
        let (store, ink, screens) = models(sent)
        store.apply([detected()])
        screens.calls.apply(detected())
        let before = ink.dropText.actions
        screens.performDropAction(.always(app: zoom, name: "Zoom")) { _ in }
        XCTAssertEqual(ink.dropText.actions, before)
        screens.performDropAction(.always(app: zoom, name: "Zoom")) { _ in }
        screens.performDropAction(.never(app: zoom, name: "Zoom")) { _ in }
        XCTAssertEqual(sent.commands.filter { if case .meetingsCallsSet = $0 { true } else { false } }.count, 1)
    }

    /// Stop on the Drop takes Stop and delete away at once; a second Stop and delete sends nothing.
    func testStopTakesStopAndDeleteAwayAtOnce() {
        let sent = Sent()
        let start = Date()
        let (store, ink, _) = models(sent, now: { start })
        let meetings = try! XCTUnwrap(ink.meetings)
        let first = started(auto: true, deleteUntil: start.addingTimeInterval(60))
        store.apply([first])
        meetings.apply(first)
        meetings.discard()
        meetings.discard()
        XCTAssertEqual(sent.commands.filter { $0 == .meetingDiscard }.count, 1)
        let second = started(auto: true, deleteUntil: start.addingTimeInterval(60), record: "r2")
        store.apply([second])
        meetings.apply(second)
        meetings.perform(.stop, permissions: PermissionsModel(send: sent.send, calendar: FakeCalendar()))
        XCTAssertEqual(ink.dropText.actions, [.stop])
    }

    /// Sound's mic lines in a call its app's Always recorded: a mic that went is said (in the first
    /// minute too) until a line comes after it; past the minute, before any line, the stand-in for
    /// a chosen mic that isn't connected; then the latest line.
    func testAnAutoCallSaysItsMicLines() {
        let store = CoreStore()
        func line(deletable: Bool) -> String {
            DropText.for(
                .meeting, dictation: .idle, meeting: store.meeting, offer: nil, systemAudioOff: false,
                micFallback: store.micFallback, deletable: deletable
            ).detail
        }
        store.apply([
            event(#"{"type":"audio.input_fallback","wanted":{"id":"pods","name":"AirPods Pro","transport":"bluetooth"},"mic_name":"MacBook Pro Microphone","mic_transport":"built_in"}"#),
            started(auto: true, deleteUntil: nil),
        ])
        XCTAssertEqual(line(deletable: true), DropText.autoReminder("Zoom"), "the first minute: the reminder")
        XCTAssertEqual(line(deletable: false), "AirPods Pro isn't connected. Using MacBook Pro Microphone.", "past it, before a line: the stand-in")

        store.apply([event(#"{"type":"meeting.mic_switched","record":"r1","from_name":"MacBook Pro Microphone","from_transport":"built_in","mic_name":"Studio Mic","mic_transport":"usb","mic_reason":"default_input"}"#)])
        let switched = "MacBook Pro Microphone went. Now recording with Studio Mic."
        XCTAssertEqual(line(deletable: true), switched)
        XCTAssertEqual(line(deletable: false), switched)

        // A line after the switch: the switch is old news.
        store.apply([event(#"{"type":"meeting.final","record":"r1","channel":"far","start_ms":0,"end_ms":900,"text":"can you hear me now"}"#)])
        XCTAssertEqual(line(deletable: true), DropText.autoReminder("Zoom"))
        XCTAssertEqual(line(deletable: false), "can you hear me now")
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
