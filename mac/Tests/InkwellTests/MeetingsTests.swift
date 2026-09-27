// Meetings in the shell (S2.8): the consent Drop and its answers, the watchdog's warning (and the
// system-audio probe's, which comes sooner), Live's title, Stop and Ask through the core, the
// ledger's window in memory, Owed's recipients and looks-done, a record's cited decisions, and the
// meeting settings.
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
private final class Sent {
    var commands: [CoreCommand] = []
    lazy var send: SendCommand = { [unowned self] in self.commands.append($0) }
}

private struct FakeCalendar: CalendarAccess {
    func state() -> CardState { .notAsked }
    func request(done: @escaping @MainActor @Sendable () -> Void) {}
}

private struct FixedTitle: CallTitles {
    let title: String?
    func titleNow(_ now: Date) -> String? { title }
}

private func checked(system: String) -> InkEvent {
    event(#"{"type":"permissions.checked","microphone":"granted","system_audio":"\#(system)","accessibility":"granted","input_monitoring":"denied"}"#)
}

@MainActor
final class ConsentDropTests: XCTestCase {
    /// Detection offers; the Drop asks, honestly, and records only when the user says so.
    func testAnOfferShowsTheDropWithAnHonestLineAndTwoAnswers() {
        let store = CoreStore()
        let ink = ShellInk(store: store)
        let drop = DropController(ink: ink)
        var pressed: [DropText.Action] = []
        drop.onAction = { pressed.append($0) }
        store.apply([event(#"{"type":"meeting.detected","app":"com.example.call","app_name":"Example Call"}"#)])
        drop.update()
        XCTAssertTrue(drop.isShown, "an offer shows the Drop")
        XCTAssertEqual(drop.inkState, .idle, "nothing is recorded yet: the ink is still")
        let text = try! XCTUnwrap(drop.shownText)
        XCTAssertEqual(text.title, "Example Call opened the microphone")
        XCTAssertTrue(text.detail.contains("Tell the others"), "consent: the others should know")
        for word in ["invisible", "undetectable", "hidden", "secret"] {
            XCTAssertFalse((text.title + text.detail).lowercased().contains(word), word)
        }
        XCTAssertEqual(text.actions, [.record(app: "com.example.call"), .dismiss(app: "com.example.call")])
        XCTAssertEqual(text.actions.map(\.title), ["Record this call", "Not this one"])
        XCTAssertFalse(drop.panelIsKey, "asking never takes focus")

        // The answer withdrawn (the app let go of the mic): the Drop goes.
        store.apply([event(#"{"type":"meeting.detection_ended","app":"com.example.call","dismissed":false}"#)])
        drop.update()
        XCTAssertFalse(drop.isShown)
        XCTAssertTrue(pressed.isEmpty, "nothing recorded without a click")
    }

    func testTheDropsAnswersBecomeMeetingCommands() {
        let sent = Sent()
        let meetings = MeetingModel(send: sent.send, titles: FixedTitle(title: "Weekly sync"))
        let permissions = PermissionsModel(send: sent.send, calendar: FakeCalendar())
        meetings.perform(.record(app: "com.example.call"), permissions: permissions)
        meetings.perform(.dismiss(app: "com.example.call"), permissions: permissions)
        meetings.perform(.allowSystemAudio, permissions: permissions)
        XCTAssertEqual(sent.commands, [
            .meetingStart(app: "com.example.call", title: "Weekly sync"),
            .meetingDismiss(app: "com.example.call"),
            .permissionRequest(.systemAudio),
        ])
        XCTAssertEqual(
            CoreCommand.meetingStart(app: "com.example.call", title: "Weekly sync").json,
            #"{"app":"com.example.call","cmd":"meeting.start","id":"meeting.start","title":"Weekly sync"}"#)
        XCTAssertEqual(CoreCommand.meetingStart(app: nil, title: nil).json, #"{"cmd":"meeting.start","id":"meeting.start"}"#)
    }

    func testAnOfferDuringAMeetingIsNeverShown() {
        let store = CoreStore()
        store.apply([event(#"{"type":"meeting.started","record":"r1"}"#)])
        store.apply([event(#"{"type":"meeting.detected","app":"a","app_name":"A"}"#)])
        XCTAssertNil(store.offer)
        store.apply([event(#"{"type":"meeting.finished","record":"r1","revision":2}"#)])
        store.apply([event(#"{"type":"meeting.detected","app":"a","app_name":"A"}"#)])
        store.apply([event(#"{"type":"meeting.started","record":"r2","app":"a","app_name":"A"}"#)])
        XCTAssertNil(store.offer, "taken: the meeting replaces the offer")
    }

    func testTheRecordingDropNamesTheAppAndShowsTheLatestLine() {
        let store = CoreStore()
        let ink = ShellInk(store: store)
        store.apply([event(#"{"type":"meeting.started","record":"r1","app":"com.example.call","app_name":"Example Call","title":"Weekly sync"}"#)])
        XCTAssertEqual(ink.dropText.title, "● REC · Example Call")
        XCTAssertEqual(ink.dropText.detail, "Recording this meeting")
        store.apply([event(#"{"type":"meeting.final","record":"r1","channel":"far","start_ms":0,"end_ms":900,"text":"the fourteenth is the earliest start"}"#)])
        XCTAssertEqual(ink.dropText.detail, "the fourteenth is the earliest start")
        XCTAssertEqual(ink.dropText.tone, .recording)
        XCTAssertTrue(ink.dropText.actions.isEmpty)
    }
}

@MainActor
final class WatchdogWarningTests: XCTestCase {
    /// Revoking system audio mid-call: the core's watchdog reports the far end's zeros within 10 s
    /// (ink-pipeline's `a_far_end_of_digital_zeros_is_reported_within_ten_seconds`), and the Drop
    /// says so at once, with the way to fix it.
    func testTheFarEndGoingSilentTurnsTheDropIntoTheWarning() {
        let store = CoreStore()
        let ink = ShellInk(store: store)
        store.apply([event(#"{"type":"meeting.started","record":"r1","app_name":"Example Call"}"#)])
        store.apply([event(#"{"type":"meeting.side_state","record":"r1","channel":"far","state":"zeros"}"#)])
        XCTAssertEqual(ink.state, .problem)
        XCTAssertEqual(ink.dropText.title, "The other side is silent")
        XCTAssertEqual(ink.dropText.detail, "System audio is off, so only your voice is being recorded.")
        XCTAssertEqual(ink.dropText.tone, .alert)
        XCTAssertEqual(ink.dropText.actions, [.allowSystemAudio])
        store.apply([event(#"{"type":"meeting.side_state","record":"r1","channel":"far","state":"ok"}"#)])
        XCTAssertEqual(ink.state, .meeting)
    }

    /// The reaction to a probe change: a system-audio check that reads "denied" during a meeting
    /// (the user came back from System Settings, or a screen checked) shows the warning at once,
    /// before ten seconds of silence could.
    func testASystemAudioProbeThatSaysOffWarnsDuringAMeeting() {
        let store = CoreStore()
        let permissions = PermissionsModel(send: { _ in }, calendar: FakeCalendar())
        let ink = ShellInk(store: store, permissions: permissions)
        permissions.apply(checked(system: "denied"))
        XCTAssertEqual(ink.state, .idle, "no meeting: nothing to warn about in the Drop")
        store.apply([event(#"{"type":"meeting.started","record":"r1"}"#)])
        XCTAssertEqual(ink.state, .problem)
        XCTAssertEqual(ink.dropText.title, "System audio is off")
        XCTAssertEqual(ink.dropText.actions, [.allowSystemAudio])
        permissions.apply(checked(system: "granted"))
        XCTAssertEqual(ink.state, .meeting, "allowed again")
    }

    func testTodayListsTheLiveWarningAndTheNewNotices() {
        let store = CoreStore()
        store.apply([event(#"{"type":"meeting.started","record":"r1"}"#)])
        store.apply([event(#"{"type":"meeting.side_state","record":"r1","channel":"far","state":"zeros"}"#)])
        store.apply([event(#"{"type":"meeting.recovered","record":"r0","trimmed":1,"rebuilt":0,"unrecoverable":0,"recorded_ms":12000}"#)])
        store.apply([event(#"{"type":"meeting.detection","listening":false,"message":"the audio server stopped answering"}"#)])
        let items = NeedsYou.items(
            permission: { _ in .allowed }, farEnd: .unknown, meeting: store.meeting,
            notices: store.notices, now: Date(), calendar: .current)
        let titles = items.map(\.title)
        XCTAssertEqual(titles.first, "Inkwell can't hear the other side of this call")
        XCTAssertTrue(titles.contains("A meeting was finished after Inkwell quit unexpectedly"))
        XCTAssertTrue(titles.contains("Inkwell stopped listening for calls"))
    }
}

@MainActor
final class LiveMeetingTests: XCTestCase {
    func testAskGoesToTheCoreAndItsAnswerIsShownAsWords() {
        let sent = Sent()
        let live = LiveModel(send: sent.send)
        live.apply(event(#"{"type":"meeting.started","record":"r1"}"#))
        live.askText = "  What did they say about security? "
        live.submitAsk(context: [])
        XCTAssertEqual(sent.commands.last, .meetingAsk(question: "What did they say about security?", ref: "ask:0"))
        XCTAssertNil(live.asked.first?.answer, "thinking")
        live.apply(event(#"{"type":"meeting.answered","record":"r1","ref":"ask:0","text":"They want it reviewed first."}"#))
        XCTAssertEqual(live.asked.first?.answer, .answer("They want it reviewed first."))

        live.askText = "What do I owe?"
        live.submitAsk(context: [])
        live.apply(event(#"{"type":"command.failed","command":"meeting.ask","id":"ask:1","message":"no language model is available to answer on this Mac"}"#))
        XCTAssertEqual(live.asked.first?.answer, .unavailable("Answers need Apple Intelligence, which is off or not ready on this Mac."))
        live.askText = "And now?"
        live.submitAsk(context: [])
        live.apply(event(#"{"type":"command.failed","command":"meeting.ask","id":"ask:2","message":"the model could not answer: engine failed"}"#))
        XCTAssertEqual(live.asked.first?.answer, .unavailable("Couldn't answer that. Try asking again."))
    }

    /// Carried into S2.8: the ledger keeps a window of the newest finals in memory, never the
    /// session (the record keeps every line), and counts what it holds for the dogfood week.
    func testTheLedgerKeepsAWindowAndCountsIt() {
        let store = CoreStore()
        store.apply([event(#"{"type":"meeting.started","record":"r1"}"#)])
        let total = CoreStore.LiveMeeting.finalsKept + 120
        var batch: [InkEvent] = []
        for i in 0..<total {
            batch.append(event(#"{"type":"meeting.final","record":"r1","channel":"far","start_ms":\#(i * 1000),"end_ms":\#(i * 1000 + 900),"text":"line \#(i)"}"#))
        }
        store.apply(batch)
        let meeting = try! XCTUnwrap(store.meeting)
        XCTAssertEqual(meeting.finals.count, CoreStore.LiveMeeting.finalsKept)
        XCTAssertEqual(meeting.finals.first?.text, "line 120", "the oldest went first")
        XCTAssertEqual(meeting.ledger.seen, total)
        XCTAssertEqual(meeting.ledger.dropped, 120)
        XCTAssertEqual(meeting.ledger.bytes, meeting.finals.reduce(0) { $0 + $1.text.utf8.count })
        XCTAssertGreaterThanOrEqual(meeting.ledger.peakBytes, meeting.ledger.bytes)
        store.apply([event(#"{"type":"meeting.finished","record":"r1","revision":2}"#)])
        XCTAssertEqual(store.lastLedger?.record, "r1")
        XCTAssertEqual(store.lastLedger?.stats.seen, total)
    }

    func testTheMeetingIsNamedByItsTitleOrItsApp() {
        let store = CoreStore()
        store.apply([event(#"{"type":"meeting.started","record":"r1","title":"Weekly sync","app":"com.example.call","app_name":"Example Call","mic_name":"MacBook Pro Microphone","mic_transport":"built_in","mic_reason":"built_in_for_bluetooth_output"}"#)])
        let meeting = try! XCTUnwrap(store.meeting)
        XCTAssertEqual(meeting.title, "Weekly sync")
        XCTAssertEqual(meeting.appName, "Example Call")
        XCTAssertEqual(meeting.micReason, .builtInForBluetoothOutput)
    }

    func testRecordNowNamesTheCallFromTheCalendarAndAFailureIsSaid() {
        let sent = Sent()
        let meetings = MeetingModel(send: sent.send, titles: FixedTitle(title: nil))
        meetings.recordNow()
        XCTAssertEqual(sent.commands, [.meetingStart(app: nil, title: nil)])
        meetings.apply(event(#"{"type":"command.failed","command":"meeting.start","id":"meeting.start","message":"the microphone: no input device"}"#))
        XCTAssertEqual(meetings.failure(on: .recordNow), "Couldn't start recording: the microphone: no input device")
        meetings.apply(event(#"{"type":"meeting.started","record":"r1"}"#))
        XCTAssertNil(meetings.failure(on: .recordNow))
        meetings.stop()
        XCTAssertEqual(sent.commands.last, .meetingStop)
    }

    func testTheCalendarTitleIsTheEventOnNow() {
        let now = Date(timeIntervalSince1970: 1_800_000_000)
        let events: [(title: String, start: Date, end: Date)] = [
            ("Earlier", now.addingTimeInterval(-7200), now.addingTimeInterval(-3600)),
            ("Weekly sync", now.addingTimeInterval(-120), now.addingTimeInterval(1800)),
            ("Starting soon", now.addingTimeInterval(240), now.addingTimeInterval(3600)),
            ("Later", now.addingTimeInterval(3600), now.addingTimeInterval(7200)),
        ]
        XCTAssertEqual(EventKitCallTitles.pick(events, now: now), "Weekly sync")
        XCTAssertEqual(EventKitCallTitles.pick(Array(events.dropFirst(1).dropFirst()), now: now), "Starting soon")
        XCTAssertNil(EventKitCallTitles.pick([events[0], events[3]], now: now))
    }
}

@MainActor
final class MeetingSettingsTests: XCTestCase {
    func testTheSettingsReadAndWriteTheCoresKeys() {
        let sent = Sent()
        let meetings = MeetingModel(send: sent.send)
        meetings.load()
        XCTAssertEqual(sent.commands, [.settingGet(.meetingsDetect), .settingGet(.meetingsHeadsetMic), .settingGet(.retentionDays)])
        XCTAssertNil(meetings.retention, "not known until the core answers")
        meetings.apply(event(#"{"type":"setting.value","key":"meetings.detect","value":"off"}"#))
        meetings.apply(event(#"{"type":"setting.value","key":"meetings.headset_mic","value":"on"}"#))
        meetings.apply(event(#"{"type":"setting.value","key":"retention.days"}"#))
        XCTAssertFalse(meetings.detect)
        XCTAssertTrue(meetings.headsetMic)
        XCTAssertEqual(meetings.retention, .forever, "never set: forever")
        meetings.setRetention(.month)
        XCTAssertEqual(sent.commands.last, .settingSet(.retentionDays, "30"))
        meetings.setDetect(true)
        XCTAssertEqual(sent.commands.last, .settingSet(.meetingsDetect, "on"))
        meetings.apply(event(#"{"type":"command.failed","command":"setting.set","id":"setting:retention.days","message":"x"}"#))
        XCTAssertTrue(meetings.settingsFailed)
        XCTAssertEqual(Set(Retention.allCases.map(\.rawValue)), ["forever", "7", "30", "90", "365"], "the core's whitelist")
    }
}

@MainActor
final class OwedRecipientTests: XCTestCase {
    private func listed(_ items: String) -> InkEvent {
        event(#"{"type":"commitments.listed","items":[\#(items)]}"#)
    }

    /// Carried into S2.8: Owed groups by the person a promise is owed to, when the meeting said.
    func testPromisesGroupByWhoTheyAreOwedTo() {
        let owed = OwedModel(send: { _ in })
        owed.apply(listed(#"""
            {"id":"c1","record":"r1","record_title":"Partner call","record_started_at_unix_ms":1800000000000,"text":"Send the deck","recipient":"Dana","merged":0},
            {"id":"c2","record":"r2","record_title":"Design review","record_started_at_unix_ms":1800000000000,"text":"Share the notes","recipient":"dana","merged":0},
            {"id":"c3","record":"r1","record_title":"Partner call","record_started_at_unix_ms":1800000000000,"text":"Book the room","merged":0}
            """#))
        let groups = owed.groups(now: Date(timeIntervalSince1970: 1_800_000_000))
        XCTAssertEqual(groups.map(\.title), ["To Dana", "Partner call"])
        XCTAssertEqual(groups[0].rows.map(\.id), ["c1", "c2"])
        XCTAssertEqual(groups[0].rows.map(\.meeting), ["Partner call", "Design review"])
    }

    /// Carried into S2.8: "looks done" suggestions come from the core; "Not yet" tells it.
    func testLooksDoneComesFromTheCoreAndNotYetTellsIt() {
        let sent = Sent()
        let owed = OwedModel(send: sent.send)
        owed.apply(listed(#"""
            {"id":"c1","record":"r1","record_started_at_unix_ms":1800000000000,"text":"Send the deck","merged":0,
             "looks_done":{"record":"r9","record_title":"Design review","record_started_at_unix_ms":1800086400000,
             "span":{"channel":"mic","start_ms":1334000,"end_ms":1337000},"text":"I already sent Dana the deck."}}
            """#))
        XCTAssertEqual(owed.suggestions.count, 1)
        let suggestion = owed.suggestions[0]
        XCTAssertEqual(suggestion.commitment, "c1")
        XCTAssertEqual(suggestion.text, "in Design review you said \u{201C}I already sent Dana the deck.\u{201D}")
        XCTAssertEqual(suggestion.source, "Design review ▸ 22:14")
        owed.notYet(suggestion)
        XCTAssertTrue(owed.suggestions.isEmpty)
        XCTAssertEqual(sent.commands.last, .commitmentNotYet(id: "c1"))
        XCTAssertEqual(CoreCommand.commitmentNotYet(id: "c1").json, #"{"cmd":"commitment.not_yet","commitment":"c1"}"#)
        owed.apply(event(#"{"type":"meeting.looks_done","record":"r9","suggested":1}"#))
        XCTAssertEqual(sent.commands.last, .commitmentsList, "a meeting found some done: list again")
    }

    /// Review (S2.8): a "Mark done" or "Not yet" the core refused puts the promise back and says
    /// so, until the next answer; it is never only a silent reload.
    func testAnAnswerTheCoreRefusedIsSaidAndThePromiseComesBack() {
        let sent = Sent()
        let owed = OwedModel(send: sent.send)
        owed.markDone("c1")
        owed.apply(event(#"{"type":"command.failed","command":"commitment.set_done","message":"the library is read-only"}"#))
        XCTAssertEqual(owed.failure, "Couldn't mark it done: the library is read-only")
        XCTAssertEqual(sent.commands.last, .commitmentsList, "put back as the core has it")
        owed.apply(event(#"{"type":"commitments.listed","items":[]}"#))
        XCTAssertNotNil(owed.failure, "the reload does not hide it")
        owed.notYet(LooksDone(id: "looks-done:c2", commitment: "c2", text: "", source: ""))
        XCTAssertNil(owed.failure, "the next answer clears it")
        owed.apply(event(#"{"type":"command.failed","command":"commitment.not_yet","message":"no such commitment"}"#))
        XCTAssertEqual(owed.failure, "Couldn't keep it open: no such commitment")
    }
}

final class CitedDecisionTests: XCTestCase {
    /// Carried into S2.8 from S2.5: a decision shows the line it cites, from the item's span.
    func testASummarysDecisionsCarryTheirCitedLines() throws {
        let json = #"""
        {"type":"library.record","ref":"x","record":{"record":"r1","kind":"meeting","started_at_unix_ms":0,"revision":2,"has_audio":false},
         "segments":[{"channel":"far","start_ms":0,"end_ms":2000,"text":"Let us start on the fourteenth."},
                     {"channel":"mic","start_ms":3000,"end_ms":5000,"text":"Agreed, the fourteenth it is."}],
         "notes":[],"commitments":[],"speakers":[],
         "summary":{"text":"Start date set.","model":"m","created_at_unix_ms":1,
                    "items":[{"kind":"decision","text":"Start on the fourteenth","span":{"channel":"mic","start_ms":3000,"end_ms":5000}},
                             {"kind":"action","text":"Book the room","span":{"channel":"far","start_ms":0,"end_ms":2000}}]}}
        """#
        guard case .libraryRecord(let answer) = try InkEvent.decode(Data(json.utf8)) else {
            return XCTFail("not a record")
        }
        let document = RecordDocument(answer)
        XCTAssertEqual(document.summaryItems.map(\.kind), [.decision, .action])
        XCTAssertEqual(document.summaryItems[0].citedLine?.text, "Agreed, the fourteenth it is.")
        XCTAssertEqual(document.summaryItems[0].citedLine?.speaker.isYou, true)
        XCTAssertEqual(document.summaryItems[1].citedLine?.text, "Let us start on the fourteenth.")
    }
}

@MainActor
final class MeetingFailureTests: XCTestCase {
    private final class Lines: @unchecked Sendable {
        var lines: [String] = []
    }

    /// Review (S2.8): a failed Record now shows where it was pressed (Today, and Live's Record
    /// now), and nowhere else; it is logged by command name, never with the core's words.
    func testARecordNowFailureShowsWhereItWasPressed() {
        let sent = Sent()
        let lines = Lines()
        let meetings = MeetingModel(
            send: sent.send, titles: FixedTitle(title: nil), log: ScreenLog { lines.lines.append($0) })
        meetings.recordNow()
        meetings.apply(event(#"{"type":"command.failed","command":"meeting.start","id":"meeting.start","message":"the microphone: no input device"}"#))
        XCTAssertEqual(meetings.failure(on: .recordNow), "Couldn't start recording: the microphone: no input device")
        XCTAssertNil(meetings.failure(on: .drop))
        XCTAssertNil(meetings.failure(on: .liveStop))
        XCTAssertEqual(lines.lines, ["command.failed for a meeting.start command; shown where it was asked"])
    }

    /// The Drop's own buttons: a failed "Record this call" or "Not this one" shows in the Drop,
    /// which keeps its buttons so the user can try again.
    func testADropAnswerThatFailsShowsInTheDrop() {
        let sent = Sent()
        let store = CoreStore()
        let meetings = MeetingModel(send: sent.send, titles: FixedTitle(title: nil))
        let ink = ShellInk(store: store, meetings: meetings)
        store.apply([event(#"{"type":"meeting.detected","app":"com.example.call","app_name":"Example Call"}"#)])
        meetings.record(app: "com.example.call")
        meetings.apply(event(#"{"type":"command.failed","command":"meeting.start","id":"meeting.start","message":"the other side's sound: permission denied"}"#))
        XCTAssertEqual(ink.dropText.title, "Example Call opened the microphone")
        XCTAssertEqual(ink.dropText.detail, "Couldn't start recording: the other side's sound: permission denied")
        XCTAssertEqual(ink.dropText.tone, .alert)
        XCTAssertEqual(ink.dropText.actions, [.record(app: "com.example.call"), .dismiss(app: "com.example.call")])
        XCTAssertNil(meetings.failure(on: .recordNow), "not claimed on Today")
        meetings.dismiss(app: "com.example.call")
        XCTAssertEqual(ink.dropText.detail, "Recording keeps both sides on this Mac. Tell the others you are recording.", "a new answer clears it")
        meetings.apply(event(#"{"type":"command.failed","command":"meeting.dismiss","id":"meeting.dismiss","message":"that app is not being offered"}"#))
        XCTAssertEqual(ink.dropText.detail, "Couldn't dismiss the offer: that app is not being offered")
    }

    /// Live's Stop: a failed stop shows in Live's header, beside the button.
    func testAStopThatFailsShowsInLive() {
        let sent = Sent()
        let meetings = MeetingModel(send: sent.send, titles: FixedTitle(title: nil))
        meetings.stop()
        meetings.apply(event(#"{"type":"command.failed","command":"meeting.stop","id":"meeting.stop","message":"the meeting is already stopping"}"#))
        XCTAssertEqual(meetings.failure(on: .liveStop), "Couldn't stop: the meeting is already stopping")
        XCTAssertNil(meetings.failure(on: .recordNow))
        XCTAssertNil(meetings.failure(on: .drop))
    }
}

@MainActor
final class FarEndHonestyTests: XCTestCase {
    /// Review (S2.8, security): a call whose app can't be heard alone records everything this Mac
    /// plays; the Drop says so until the first line, and Live's header says so throughout.
    func testACallThatCantBeHeardAloneSaysItRecordsEverything() {
        let store = CoreStore()
        let ink = ShellInk(store: store)
        store.apply([event(#"{"type":"meeting.started","record":"r1","app":"com.example.call","app_name":"Example Call","far_end":"everything"}"#)])
        XCTAssertEqual(ink.dropText.detail, "Recording this meeting", "not yet told")
        store.apply([event(#"{"type":"meeting.far_end_fallback","record":"r1","app":"com.example.call","app_name":"Example Call","message":"no audio process for that app"}"#)])
        XCTAssertEqual(ink.dropText.detail, "Inkwell couldn't hear Example Call alone, so it is recording everything this Mac plays")
        let meeting = try! XCTUnwrap(store.meeting)
        let line = try! XCTUnwrap(LiveMeetingView.farEndLine(meeting))
        XCTAssertTrue(line.alert)
        XCTAssertTrue(line.text.contains("everything this Mac plays"))
        store.apply([event(#"{"type":"meeting.final","record":"r1","channel":"far","start_ms":0,"end_ms":900,"text":"shall we start"}"#)])
        XCTAssertEqual(ink.dropText.detail, "shall we start")
        XCTAssertNotNil(LiveMeetingView.farEndLine(try! XCTUnwrap(store.meeting)), "Live keeps saying it")
    }

    /// Record now records everything this Mac plays by design: Live says so, plainly; a call
    /// recorded from the offer says nothing more.
    func testRecordNowSaysWhatItRecordsAndAnOfferedCallDoesNot() {
        let store = CoreStore()
        store.apply([event(#"{"type":"meeting.started","record":"r1","far_end":"everything"}"#)])
        let now = try! XCTUnwrap(LiveMeetingView.farEndLine(try! XCTUnwrap(store.meeting)))
        XCTAssertFalse(now.alert)
        XCTAssertTrue(now.text.contains("everything this Mac plays"))
        store.apply([event(#"{"type":"meeting.finished","record":"r1","revision":2}"#)])
        store.apply([event(#"{"type":"meeting.started","record":"r2","app":"a","app_name":"A","far_end":"app"}"#)])
        XCTAssertNil(LiveMeetingView.farEndLine(try! XCTUnwrap(store.meeting)))
    }
}

@MainActor
final class DetectionStateTests: XCTestCase {
    /// Review (S2.8): Today follows the core's detection state: a setting the core could not read
    /// is announced as not listening, with why, and Today says so however the setting reads.
    func testTodayFollowsTheCoresDetectionStateNotTheSetting() {
        let store = CoreStore()
        XCTAssertEqual(RecordControls.listeningText(recording: false, listening: store.listening), " ")
        store.apply([event(#"{"type":"meeting.detection","listening":false,"message":"couldn't read the detection setting: database is locked"}"#)])
        XCTAssertEqual(store.listening, false)
        XCTAssertEqual(RecordControls.listeningText(recording: false, listening: store.listening), "Not listening for meetings")
        XCTAssertEqual(store.notices.last?.kind, .detectionUnavailable)
        store.apply([event(#"{"type":"meeting.detection","listening":true}"#)])
        XCTAssertEqual(RecordControls.listeningText(recording: false, listening: store.listening), "Listening for meetings")
        XCTAssertEqual(RecordControls.listeningText(recording: true, listening: true), "Recording")
    }
}

@MainActor
final class RecoveryNoticeTests: XCTestCase {
    /// Review (S2.8): recovery that could not even look for interrupted meetings says so once on
    /// Today; one that looked and found none says nothing.
    func testRecoveryThatCouldNotLookIsSaidOnToday() {
        let store = CoreStore()
        store.apply([event(#"{"type":"meetings.recovered","meetings":0}"#)])
        XCTAssertTrue(store.notices.isEmpty)
        store.apply([event(#"{"type":"meetings.recovered","meetings":0,"message":"couldn't look for meetings a crash interrupted: Not a directory (os error 20)"}"#)])
        let items = NeedsYou.items(
            permission: { _ in .allowed }, farEnd: .unknown, meeting: store.meeting,
            notices: store.notices, now: Date(), calendar: .current)
        XCTAssertEqual(items.map(\.title), ["Inkwell couldn't check for an unfinished meeting"])
    }

    /// Review (S2.8): a meeting that runs without its crash marker says so on Today.
    func testAMeetingWithoutItsCrashMarkerSaysItIsNotProtected() {
        let store = CoreStore()
        store.apply([event(#"{"type":"meeting.started","record":"r1"}"#)])
        store.apply([event(#"{"type":"meeting.warning","record":"r1","kind":"not_crash_protected","message":"Is a directory (os error 21)"}"#)])
        let items = NeedsYou.items(
            permission: { _ in .allowed }, farEnd: .unknown, meeting: store.meeting,
            notices: store.notices, now: Date(), calendar: .current)
        XCTAssertTrue(items.map(\.title).contains("This meeting isn't protected against a crash"))
    }
}
