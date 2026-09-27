// The Live, Owed and Settings screens' models and the first-run state: what each shows for the
// core's events, and what it sends. The views are checked by hand (mac/SCREENS-B-CHECKLIST.md).
import AppKit
import AppleEngines
import Foundation
import SQLite3
import Synchronization
import InkBridge
import SwiftUI
import XCTest

@testable import Inkwell

private func event(_ json: String, file: StaticString = #filePath, line: UInt = #line) -> InkEvent {
    guard let decoded = try? InkEvent.decode(Data(json.utf8)) else {
        XCTFail("not an event: \(json)", file: file, line: line)
        return .unknown(type: "")
    }
    if case .undecodable = decoded {
        XCTFail("not this build's event: \(json)", file: file, line: line)
    }
    return decoded
}

/// What was logged.
private final class Logged: Sendable {
    private let lines = Mutex<[String]>([])
    var messages: [String] { lines.withLock { $0 } }
    var log: ScreenLog { ScreenLog { [self] message in lines.withLock { $0.append(message) } } }
}

/// What a model sent.
@MainActor
private final class Sent {
    var commands: [CoreCommand] = []
    lazy var send: SendCommand = { [unowned self] in self.commands.append($0) }
}

private struct FakeCalendar: CalendarAccess {
    var answer: CardState = .notAsked
    func state() -> CardState { answer }
    func request(done: @escaping @MainActor @Sendable () -> Void) {}
}

private func checked(mic: String = "granted", system: String, ax: String = "granted") -> InkEvent {
    event(#"{"type":"permissions.checked","microphone":"\#(mic)","system_audio":"\#(system)","accessibility":"\#(ax)","input_monitoring":"denied"}"#)
}

// MARK: - Permissions

/// A core whose probe answers each check about a second later (the tone probe's time), with what
/// the probe reads when the check was sent.
@MainActor
private final class FakeProbeCore {
    var systemAudio = "granted"
    var checks = 0
    weak var model: PermissionsModel?

    lazy var send: SendCommand = { [unowned self] command in
        guard command == .permissionsCheck else { return }
        self.checks += 1
        let answer = checked(system: self.systemAudio)
        Task { @MainActor [weak self] in
            try? await Task.sleep(for: .milliseconds(900))
            self?.model?.apply(answer)
        }
    }
}

@MainActor
final class PermissionsModelTests: XCTestCase {
    /// Verify: revoking system audio turns its card red within 5 s. The core's half (a check
    /// answers within 5 s even while a model update holds the command thread) is ink-ffi's
    /// queries test; this is the shell's: coming back to the app re-checks, and the answer turns the
    /// card red. The TCC half is in SCREENS-B-CHECKLIST.md.
    func testRevokingSystemAudioTurnsItsCardRedWithinFiveSecondsOfComingBack() async throws {
        let core = FakeProbeCore()
        let model = PermissionsModel(send: core.send, calendar: FakeCalendar(answer: .allowed))
        core.model = model
        model.screenAppeared()
        try await waitUntil { model.state(.hearTheOthers) == .allowed }
        XCTAssertFalse(model.state(.hearTheOthers).isAlert)

        // Revoked in System Settings; the user comes back to Inkwell.
        core.systemAudio = "denied"
        let back = ContinuousClock.now
        model.appBecameActive()
        try await waitUntil { model.state(.hearTheOthers) == .off }
        XCTAssertLessThan(ContinuousClock.now - back, .seconds(5))
        XCTAssertTrue(model.state(.hearTheOthers).isAlert, "red")
        XCTAssertEqual(model.offCards, [.hearTheOthers])
        XCTAssertEqual(PermissionCard.hearTheOthers.offDetail, "System audio is off. Meetings record only your voice.")
        XCTAssertEqual(core.checks, 2)
    }

    func testChecksRunOnlyWhenAScreenShowsTheCardsOrAfterLaunchNeverOnATimer() {
        let sent = Sent()
        let model = PermissionsModel(send: sent.send, calendar: FakeCalendar())
        model.appBecameActive()
        XCTAssertEqual(sent.commands, [], "no card on screen: activation checks nothing")
        model.screenAppeared()
        XCTAssertEqual(sent.commands, [.permissionsCheck])
        model.appBecameActive()
        XCTAssertEqual(sent.commands, [.permissionsCheck, .permissionsCheck])
        model.screenDisappeared()
        model.appBecameActive()
        XCTAssertEqual(sent.commands.count, 2)
    }

    func testAFailedCheckLeavesNoCardGreenOrRed() {
        let model = PermissionsModel(send: { _ in }, calendar: FakeCalendar(answer: .allowed))
        model.refresh()
        model.apply(checked(mic: "granted", system: "denied", ax: "granted"))
        XCTAssertEqual(model.state(.hearYou), .allowed)
        model.refresh()
        model.apply(event(#"{"type":"command.failed","command":"permissions.check","message":"a bug in the core stopped this command"}"#))
        for card in [PermissionCard.hearYou, .hearTheOthers, .typeForYou] {
            XCTAssertEqual(model.state(card), .unknown, "\(card): neither green nor red after a failed check")
        }
        XCTAssertEqual(model.state(.knowYourMeetings), .allowed, "the calendar is read by the shell, not the check")
        XCTAssertFalse(model.checking)
    }

    func testEachStateReadsAsItsCardAndAllowAsksTheCoreOrTheCalendar() {
        let sent = Sent()
        let model = PermissionsModel(send: sent.send, calendar: FakeCalendar(answer: .off))
        XCTAssertEqual(model.state(.hearYou), .checking)
        model.refresh()
        model.apply(checked(mic: "not_determined", system: "unknown", ax: "denied"))
        XCTAssertEqual(model.state(.hearYou), .notAsked)
        XCTAssertEqual(model.state(.hearTheOthers), .unknown)
        XCTAssertEqual(model.state(.typeForYou), .off)
        XCTAssertEqual(model.state(.knowYourMeetings), .off, "EventKit's answer, read by the shell")
        model.request(.hearTheOthers)
        XCTAssertEqual(sent.commands.last, .permissionRequest(.systemAudio))
        XCTAssertEqual(
            PermissionCard.allCases.map(\.title),
            ["Hear you", "Hear the others", "Type for you", "Know your meetings"])
    }

    private func waitUntil(_ timeout: Duration = .seconds(5), _ done: () -> Bool) async throws {
        let start = ContinuousClock.now
        while !done() {
            if ContinuousClock.now - start > timeout {
                return XCTFail("not within \(timeout)")
            }
            try await Task.sleep(for: .milliseconds(20))
        }
    }
}

// MARK: - Polish

@MainActor
final class PolishModelTests: XCTestCase {
    /// Verify: the Polish toggle can't read "on" without a working engine.
    func testTheToggleNeverReadsOnWithoutAWorkingEngine() {
        let sent = Sent()
        let polish = PolishModel(send: sent.send)
        polish.apply(event(#"{"type":"setting.value","key":"dictation.polish","value":"on"}"#))
        XCTAssertEqual(polish.preference, true)
        XCTAssertFalse(polish.isOn, "wished for, but nothing can polish")
        XCTAssertFalse(polish.canToggle)
        polish.setOn(true)
        XCTAssertEqual(sent.commands, [], "a toggle with no engine sends nothing")

        // Apple Intelligence is off: the engines say why, and it still reads off.
        polish.appleEnginesReported(.unavailable(code: AppleIntelligence.Reason.notEnabled.rawValue))
        XCTAssertFalse(polish.isOn)
        XCTAssertEqual(polish.status, "Polish needs Apple Intelligence. Turn it on in System Settings.")
        // The engines registering it is not enough: the core has to confirm it.
        polish.appleEnginesReported(.registered)
        XCTAssertFalse(polish.isOn)

        polish.apply(event(#"{"type":"engine.registered","id":"apple-foundation-models","kind":"llm","jobs":[]}"#))
        XCTAssertTrue(polish.isOn)
        XCTAssertTrue(polish.canToggle)

        // A speech engine is not a language model.
        polish.apply(event(#"{"type":"engine.unregistered","id":"apple-foundation-models"}"#))
        polish.apply(event(#"{"type":"engine.registered","id":"parakeet","kind":"streaming","jobs":[{"job":"live_partials","wer":21.3}]}"#))
        XCTAssertFalse(polish.isOn, "let go of when Apple Intelligence went away")

        polish.apply(event(#"{"type":"engine.registered","id":"apple-foundation-models","kind":"llm","jobs":[]}"#))
        polish.apply(event(#"{"type":"core.stopped"}"#))
        XCTAssertFalse(polish.isOn, "a stopped core holds no engine")
    }

    func testSwitchingItSavesTheWishAndAnUnsetWishIsOff() {
        let sent = Sent()
        let polish = PolishModel(send: sent.send)
        polish.load()
        XCTAssertEqual(sent.commands, [.settingGet(.dictationPolish)])
        polish.apply(event(#"{"type":"engine.registered","id":"apple-foundation-models","kind":"llm","jobs":[]}"#))
        polish.apply(event(#"{"type":"setting.value","key":"dictation.polish"}"#))
        XCTAssertEqual(polish.preference, false)
        XCTAssertFalse(polish.isOn)
        polish.setOn(true)
        XCTAssertTrue(polish.isOn)
        XCTAssertEqual(sent.commands.last, .settingSet(.dictationPolish, "on"))
    }

    /// The §6 point: a timeout reads apart from an ordinary cancel or failure.
    func testPolishThatKeepsTimingOutSaysSoAndAPlainFailureDoesNot() {
        let polish = PolishModel(send: { _ in })
        polish.apply(event(#"{"type":"engine.registered","id":"apple-foundation-models","kind":"llm","jobs":[]}"#))
        polish.apply(event(#"{"type":"setting.value","key":"dictation.polish","value":"on"}"#))
        func take(_ warning: String?) {
            polish.apply(event(#"{"type":"dictation.started"}"#))
            if let warning { polish.apply(event(warning)) }
            polish.apply(event(#"{"type":"dictation.inserted","text":"x","outcome":"pasted"}"#))
        }
        take(#"{"type":"dictation.warning","kind":"polish_failed","message":"cancelled"}"#)
        take(#"{"type":"dictation.warning","kind":"polish_failed","message":"cancelled"}"#)
        XCTAssertFalse(polish.keepsTimingOut, "a cancel is not a timeout")
        take(#"{"type":"dictation.warning","kind":"polish_timed_out"}"#)
        XCTAssertFalse(polish.keepsTimingOut, "once is not keeps")
        take(#"{"type":"dictation.warning","kind":"polish_timed_out"}"#)
        XCTAssertTrue(polish.keepsTimingOut)
        XCTAssertEqual(polish.status, "Polish keeps timing out, so your words go in as you said them.")
        XCTAssertTrue(polish.isOn, "still on: the engine works, it is slow")
        take(nil)
        XCTAssertFalse(polish.keepsTimingOut, "a take that polished in time ends the run")
    }
}

// MARK: - Models

@MainActor
final class CatalogueModelTests: XCTestCase {
    /// engine.routed answers only engine.route: the screen asks again when the answer may change.
    func testTheScreenAsksAgainAfterAnInstallOrWhenAnEngineComesOrGoes() {
        let sent = Sent()
        let catalogue = CatalogueModel(send: sent.send)
        let asked: [CoreCommand] = [.modelsList, .engineRoute(.dictationFinal), .engineRoute(.meetingFinal), .engineRoute(.livePartials)]
        catalogue.requery()
        XCTAssertEqual(sent.commands, asked)
        catalogue.apply(event(#"{"type":"model.update_finished","id":"qwen3-asr-1.7b-q8","next":"qwen3-asr-1.7b-q8","ok":true,"no_model_warm":false}"#))
        XCTAssertEqual(sent.commands, asked + asked, "a model installed: ask again")
        catalogue.apply(event(#"{"type":"engine.registered","id":"apple-foundation-models","kind":"llm","jobs":[]}"#))
        XCTAssertEqual(sent.commands.count, 12, "Apple Intelligence came: ask again")
        catalogue.apply(event(#"{"type":"engine.unregistered","id":"apple-foundation-models"}"#))
        XCTAssertEqual(sent.commands.count, 16, "and when it went")
        catalogue.apply(event(#"{"type":"model.warmed","id":"qwen3-asr-1.7b-q8","job":"dictation_final"}"#))
        XCTAssertEqual(sent.commands.count, 16, "warming changes no answer")
    }

    func testAFailedListReadsAsFailedNotAsNothingInstalled() {
        let catalogue = CatalogueModel(send: { _ in })
        catalogue.apply(event(#"{"type":"command.failed","command":"models.list","message":"the registry could not be read"}"#))
        XCTAssertTrue(catalogue.failed)
        XCTAssertEqual(CatalogueModel.failedText, "The model list could not be read.")
        catalogue.apply(event(#"{"type":"models.listed","models":[]}"#))
        XCTAssertFalse(catalogue.failed, "a list that arrives clears it")
    }

    func testEachJobShowsWhatServesItAndItsMeasuredAccuracy() {
        let catalogue = CatalogueModel(send: { _ in })
        XCTAssertFalse(catalogue.line(.dictationFinal).known)
        catalogue.apply(event(#"{"type":"models.listed","models":[{"id":"qwen3-asr-1.7b-q8","licence":"Apache-2.0","size_bytes":2500000000,"installed":true,"jobs":[{"job":"dictation_final","wer":4.59},{"job":"meeting_final","wer":16.08}]}]}"#))
        catalogue.apply(event(#"{"type":"engine.registered","id":"fluidaudio-parakeet-tdt-0.6b-v3","kind":"streaming","jobs":[{"job":"live_partials","wer":21.3}]}"#))
        catalogue.apply(event(#"{"type":"engine.routed","job":"dictation_final","id":"qwen3-asr-1.7b-q8","source":"registry"}"#))
        catalogue.apply(event(#"{"type":"engine.routed","job":"live_partials","id":"fluidaudio-parakeet-tdt-0.6b-v3","source":"shell"}"#))
        catalogue.apply(event(#"{"type":"engine.routed","job":"meeting_final"}"#))
        let dictation = catalogue.line(.dictationFinal)
        XCTAssertEqual(dictation.engine, "Qwen3-ASR 1.7B")
        XCTAssertEqual(dictation.wer, 4.59)
        XCTAssertEqual(dictation.accuracy, "95.4 % of words right (4.6 % word error rate)")
        XCTAssertEqual(catalogue.line(.livePartials).engine, "Parakeet TDT v3")
        XCTAssertEqual(catalogue.line(.livePartials).wer, 21.3)
        XCTAssertTrue(catalogue.line(.meetingFinal).known)
        XCTAssertNil(catalogue.line(.meetingFinal).engine, "nothing fills it")
    }
}

// MARK: - Modes

private struct Apps: AppDirectory {
    func app(bundleID: String) -> (name: String, icon: NSImage)? {
        bundleID == "com.example.installed" ? ("Example Writer", NSImage()) : nil
    }
}

@MainActor
final class ModesModelTests: XCTestCase {
    private static let bundleID = /^[A-Za-z0-9-]+(\.[A-Za-z0-9-]+)+$/

    /// Verify: no mode shows a raw bundle ID.
    func testNoModeShowsARawBundleID() throws {
        let identities = [
            "com.example.installed", "net.whatsapp.WhatsApp", "com.example.missing.app", "slack",
            "com.apple.finder", " us.zoom.xos ", "COM.TINYSPECK.SLACKMACGAP",
        ]
        let apps = try JSONSerialization.data(withJSONObject: identities)
        let json = #"{"type":"modes.listed","default_id":"d","modes":[{"id":"chat","name":"Chat","style":"casual","polish":false,"remove_fillers":true,"apps":\#(String(decoding: apps, as: UTF8.self))},{"id":"d","name":"Default","style":"formal","polish":true,"remove_fillers":true,"apps":[]}]}"#
        for directory in [Apps(), WorkspaceApps()] as [any AppDirectory] {
            let modes = ModesModel(send: { _ in }, apps: directory)
            modes.apply(event(json))
            let shown = modes.rows.flatMap { [$0.name] + $0.traits + $0.apps.map(\.name) }
            XCTAssertFalse(shown.isEmpty)
            for text in shown {
                XCTAssertNil(text.wholeMatch(of: Self.bundleID), "\(text) is a bundle id")
                for identity in identities {
                    XCTAssertFalse(
                        text.localizedCaseInsensitiveContains(identity.trimmingCharacters(in: .whitespaces))
                            && AppIdentity.isBundleID(identity.trimmingCharacters(in: .whitespaces)),
                        "\(text) shows \(identity)")
                }
            }
        }
        let modes = ModesModel(send: { _ in }, apps: Apps())
        modes.apply(event(json))
        XCTAssertEqual(
            modes.rows[0].apps.map(\.name),
            ["Example Writer", "WhatsApp", "An app not on this Mac", "Slack", "An app not on this Mac", "Zoom", "Slack"])
        XCTAssertEqual(modes.rows[0].traits, ["Casual", "Clean up speech"])
        XCTAssertEqual(modes.rows[1].traits, ["Formal", "Clean up speech", "Polish"])
        XCTAssertTrue(modes.rows[1].isDefault, "the default is listed last")
    }

    func testAnInstalledAppIsNamedAndIconedByTheWorkspace() {
        let finder = AppIdentity.label("com.apple.finder", apps: WorkspaceApps())
        XCTAssertEqual(finder.name, "Finder")
        XCTAssertTrue(finder.installed)
        XCTAssertNotNil(finder.icon)
    }
}

// MARK: - Owed

@MainActor
final class OwedModelTests: XCTestCase {
    private func listed(_ items: String) -> InkEvent {
        event(#"{"type":"commitments.listed","items":[\#(items)]}"#)
    }

    private static let day: Int64 = 86_400_000

    func testPromisesAreGroupedByPersonOrMeetingWithOverdueAndSaidTwice() throws {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = try XCTUnwrap(TimeZone(identifier: "UTC"))
        let now = Date(timeIntervalSince1970: 1_790_500_000)
        let nowMs = Int64(now.timeIntervalSince1970 * 1_000)
        let owed = OwedModel(send: { _ in }, calendar: calendar)
        owed.apply(listed("""
            {"id":"a","record":"r1","record_title":"Planning call","record_started_at_unix_ms":\(nowMs - 3_600_000),"text":"Share the scorecard draft","due_at_unix_ms":\(nowMs - 2 * Self.day),"merged":0},
            {"id":"b","record":"r1","record_title":"Planning call","record_started_at_unix_ms":\(nowMs - 3_600_000),"text":"Send the pilot deck","due_at_unix_ms":\(nowMs + Self.day),"said_at_ms":2332000,"channel":"mic","merged":1},
            {"id":"c","record":"r2","record_started_at_unix_ms":\(nowMs - 5 * Self.day),"text":"Review the budget","owner":"Sam","merged":2},
            {"id":"d","record":"r3","record_started_at_unix_ms":\(nowMs - 5 * Self.day),"text":"Book a room","merged":0}
            """))
        let groups = owed.groups(now: now)
        XCTAssertEqual(groups.map(\.title), ["Planning call", "Sam", "A meeting on " + Date(timeIntervalSince1970: Double(nowMs - 5 * Self.day) / 1_000).formatted(.dateTime.weekday(.abbreviated).day().month(.abbreviated))])
        XCTAssertEqual(groups[0].subtitle, "today")
        XCTAssertEqual(groups[0].rows.map(\.id), ["a", "b"])
        XCTAssertEqual(groups[0].rows[0].due, .overdue(days: 2))
        XCTAssertEqual(groups[0].rows[0].due.text(calendar: calendar), "2 days overdue")
        XCTAssertEqual(groups[0].rows[1].due, .tomorrow)
        XCTAssertEqual(groups[0].rows[1].mergedText, "Said twice · merged")
        XCTAssertEqual(groups[1].rows[0].mergedText, "Said 3 times · merged")
        XCTAssertEqual(groups[1].rows[0].meeting, "A meeting on " + Date(timeIntervalSince1970: Double(nowMs - 5 * Self.day) / 1_000).formatted(.dateTime.weekday(.abbreviated).day().month(.abbreviated)))
        XCTAssertEqual(groups[2].rows[0].due, .undated)
        XCTAssertNil(groups[2].subtitle, "an untitled meeting is named by its day once")
        XCTAssertEqual(owed.summary(now: now), "4 open · 1 due this week · 1 overdue")
    }

    func testMarkingDoneTakesItOffAndTheCoresListReplacesIt() {
        let sent = Sent()
        let owed = OwedModel(send: sent.send)
        owed.apply(listed(#"{"id":"a","record":"r","record_started_at_unix_ms":0,"text":"x","merged":0}"#))
        owed.markDone("a")
        XCTAssertTrue(owed.items.isEmpty)
        XCTAssertEqual(sent.commands, [.commitmentSetDone(id: "a", done: true)])
        owed.apply(event(#"{"type":"commitment.updated","commitment":"a","done":true}"#))
        XCTAssertEqual(sent.commands.last, .commitmentsList, "listed again from the core")
        owed.apply(event(#"{"type":"meeting.commitments","record":"r2","filed":2,"merged":0}"#))
        XCTAssertEqual(sent.commands.count, 3, "a meeting filed new ones: listed again")
    }
}

// MARK: - Live

@MainActor
final class LiveModelTests: XCTestCase {
    private func meeting(_ live: LiveModel) {
        live.apply(event(#"{"type":"meeting.started","record":"rec"}"#))
    }

    func testTheLedgerShowsFinalsDryAndPartialsWetAndKeepsNothingOfAPartial() {
        let store = CoreStore()
        store.apply([
            event(#"{"type":"meeting.started","record":"rec"}"#),
            event(#"{"type":"meeting.final","record":"rec","channel":"far","start_ms":738000,"end_ms":741000,"text":"The review takes a week."}"#),
            event(#"{"type":"meeting.partial","record":"rec","channel":"far","text":"so realistically the"}"#),
            event(#"{"type":"meeting.partial","record":"rec","channel":"mic","text":"   "}"#),
        ])
        let lines = LiveLine.ledger(store.meeting!)
        XCTAssertEqual(lines.map(\.wet), [false, true], "a blank partial is not a line")
        // The far end settles a line said earlier than the last one of yours: it goes in its place.
        let mixed = CoreStore()
        mixed.apply([
            event(#"{"type":"meeting.started","record":"r"}"#),
            event(#"{"type":"meeting.final","record":"r","channel":"mic","start_ms":9000,"end_ms":9500,"text":"b"}"#),
            event(#"{"type":"meeting.final","record":"r","channel":"far","start_ms":5000,"end_ms":6000,"text":"a"}"#),
            event(#"{"type":"meeting.final","record":"r","channel":"far","start_ms":9000,"end_ms":9900,"text":"c"}"#),
        ])
        XCTAssertEqual(LiveLine.ledger(mixed.meeting!).map(\.text), ["a", "b", "c"], "by time, ties in arrival order")
        XCTAssertEqual(lines[0].atMs, 738_000)
        XCTAssertNil(lines[1].atMs)
        store.apply([event(#"{"type":"meeting.final","record":"rec","channel":"far","start_ms":741000,"end_ms":744000,"text":"So realistically the fourteenth."}"#)])
        XCTAssertEqual(LiveLine.ledger(store.meeting!).map(\.wet), [false, false], "the final replaced the wet line")
        XCTAssertEqual(liveClock(ms: 738_000), "12:18")
        XCTAssertEqual(liveClock(ms: 3_723_000), "1:02:03")
    }

    func testTheStackHoldsTheFarEndsQuestionsNewestFirstUpToFour() {
        let live = LiveModel(send: { _ in })
        meeting(live)
        func far(_ text: String, _ channel: String = "far") {
            live.apply(event(#"{"type":"meeting.final","record":"rec","channel":"\#(channel)","start_ms":1000,"end_ms":2000,"text":"\#(text)"}"#))
        }
        far("Can we run the kickoff in parallel? I think so.")
        far("Could you send the deck by Friday?", "mic")
        far("Right? What did security say about the data?")
        far("Can we run the kickoff in parallel?")
        far("Who owns the rollout plan? When is the review? Where do the models run?")
        XCTAssertEqual(live.stack.questions.map(\.text), [
            "Where do the models run?", "When is the review?", "Who owns the rollout plan?",
            "What did security say about the data?",
        ])
    }

    func testAskingSaysPlainlyWhenNothingCanAnswer() async throws {
        let live = LiveModel(send: { _ in })
        meeting(live)
        live.askText = "  What do I owe so far? "
        live.submitAsk(context: [])
        XCTAssertEqual(live.askText, "")
        XCTAssertEqual(live.asked.first?.question, "What do I owe so far?")
        for _ in 0..<100 where live.asked.first?.answer == nil {
            try await Task.sleep(for: .milliseconds(10))
        }
        guard case .unavailable = live.asked.first?.answer else {
            return XCTFail("no made-up answer: \(String(describing: live.asked.first?.answer))")
        }
    }

    func testEachNoteLineIsSavedWhenTheUserMovesOnAndEditsAndDeletesFollowIt() {
        let sent = Sent()
        var now = Date(timeIntervalSince1970: 1_000)
        let live = LiveModel(send: sent.send, now: { now })
        meeting(live)
        now += 754
        live.notesEdited("Pilot: two teams", caretParagraph: 0)
        XCTAssertEqual(sent.commands, [], "the line being typed is not saved yet")
        now += 10
        live.notesEdited("Pilot: two teams\n", caretParagraph: 1)
        XCTAssertEqual(sent.commands, [.noteAdd(record: "rec", atMs: 754_000, text: "Pilot: two teams", ref: "rec:line:0")],
                       "stamped with when it was started, not when it was left")
        live.notesEdited("Pilot: two teams\nSecurity review first", caretParagraph: 1)
        // The first line is edited before the core answered its add.
        live.notesEdited("Pilot: two teams, six weeks\nSecurity review first", caretParagraph: 1)
        XCTAssertEqual(sent.commands.count, 1, "an add on its way is not sent twice")
        live.apply(event(#"{"type":"note.added","record":"rec","note":"n1","at_ms":754000,"ref":"rec:line:0"}"#))
        XCTAssertEqual(sent.commands.last, .noteUpdate(note: "n1", text: "Pilot: two teams, six weeks", ref: "rec:line:0:update"))
        // The user leaves the editor: the second line is saved too.
        live.notesLeft()
        XCTAssertEqual(sent.commands.last, .noteAdd(record: "rec", atMs: 764_000, text: "Security review first", ref: "rec:line:1"))
        live.apply(event(#"{"type":"note.added","record":"rec","note":"n2","at_ms":764000,"ref":"rec:line:1"}"#))
        // Deleting the first line deletes its note; the second keeps its own.
        live.notesEdited("Security review first", caretParagraph: 0)
        XCTAssertEqual(sent.commands.last, .noteDelete(note: "n1", ref: "rec:line:0:delete"))
        let before = sent.commands.count
        live.apply(event(#"{"type":"meeting.stopped","record":"rec"}"#))
        XCTAssertEqual(sent.commands.count, before, "nothing unsaved is left when capture stops")
    }

    /// One line typed and added as note n1: the line under test.
    private func savedLine(_ sent: Sent) -> LiveModel {
        let live = LiveModel(send: sent.send, now: { Date(timeIntervalSince1970: 1_000) })
        meeting(live)
        live.notesEdited("Alpha\nBeta", caretParagraph: 1)
        live.apply(event(#"{"type":"note.added","record":"rec","note":"n1","at_ms":0,"ref":"rec:line:0"}"#))
        sent.commands = []
        return live
    }

    func testAFailingUpdateKeepsTheLineUnsavedAndRetriesOnTheNextLeave() {
        let sent = Sent()
        let live = savedLine(sent)
        live.notesEdited("Alpha two\nBeta", caretParagraph: 1)
        XCTAssertEqual(sent.commands, [.noteUpdate(note: "n1", text: "Alpha two", ref: "rec:line:0:update")])
        live.apply(event(#"{"type":"command.failed","command":"note.update","id":"rec:line:0:update","message":"the library is busy"}"#))
        XCTAssertEqual(sent.commands.count, 1, "no retry loop: tried again when the user moves on")
        live.notesLeft()
        XCTAssertTrue(sent.commands.dropFirst().contains(.noteUpdate(note: "n1", text: "Alpha two", ref: "rec:line:0:update")),
                      "the line was not marked saved: \(sent.commands)")
        live.apply(event(#"{"type":"note.updated","note":"n1","ref":"rec:line:0:update"}"#))
        live.notesLeft()
        XCTAssertEqual(sent.commands.filter { $0.name == "note.update" }.count, 2, "saved now: nothing more to send")
    }

    func testAFailingDeleteKeepsTheNoteSoARetypeUpdatesAndNeverDuplicates() {
        let sent = Sent()
        let live = savedLine(sent)
        live.notesEdited("\nBeta", caretParagraph: 1)
        XCTAssertEqual(sent.commands, [.noteDelete(note: "n1", ref: "rec:line:0:delete")])
        // Retyped while the delete is on its way: nothing is sent until the delete settles.
        live.notesEdited("Alpha again\nBeta", caretParagraph: 1)
        XCTAssertEqual(sent.commands.count, 1)
        live.apply(event(#"{"type":"command.failed","command":"note.delete","id":"rec:line:0:delete","message":"the library is busy"}"#))
        XCTAssertEqual(sent.commands.last, .noteUpdate(note: "n1", text: "Alpha again", ref: "rec:line:0:update"),
                       "the note still exists: updated, never added again")
        XCTAssertFalse(sent.commands.contains { $0.name == "note.add" })
    }

    func testAConfirmedDeleteLetsARetypedLineBeAddedAfresh() {
        let sent = Sent()
        let live = savedLine(sent)
        live.notesEdited("\nBeta", caretParagraph: 1)
        live.notesEdited("Alpha again\nBeta", caretParagraph: 1)
        live.apply(event(#"{"type":"note.deleted","note":"n1","ref":"rec:line:0:delete"}"#))
        XCTAssertEqual(sent.commands.last, .noteAdd(record: "rec", atMs: 0, text: "Alpha again", ref: "rec:line:0"))
    }

    func testARemovedLinesFailedDeleteIsTriedAgainOnTheNextLeave() {
        let sent = Sent()
        let live = savedLine(sent)
        live.notesEdited("Beta", caretParagraph: 0)
        XCTAssertEqual(sent.commands, [.noteDelete(note: "n1", ref: "rec:line:0:delete")])
        live.apply(event(#"{"type":"command.failed","command":"note.delete","id":"rec:line:0:delete","message":"the library is busy"}"#))
        live.notesLeft()
        XCTAssertEqual(sent.commands.filter { $0 == .noteDelete(note: "n1", ref: "rec:line:0:delete") }.count, 2,
                       "the note the user deleted does not stay in the record")
        live.apply(event(#"{"type":"note.deleted","note":"n1","ref":"rec:line:0:delete"}"#))
        live.notesLeft()
        XCTAssertEqual(sent.commands.filter { $0.name == "note.delete" }.count, 2, "done")
    }

    func testPartialsNeverBecomeNotesOrCommands() {
        let sent = Sent()
        let live = LiveModel(send: sent.send)
        meeting(live)
        live.apply(event(#"{"type":"meeting.partial","record":"rec","channel":"far","text":"so realistically"}"#))
        live.apply(event(#"{"type":"meeting.partial","record":"rec","channel":"mic","text":"send the"}"#))
        XCTAssertEqual(sent.commands, [])
    }

    func testTheNotesEditorIsTextKit2() {
        let textView = NotesEditor.makeTextView()
        XCTAssertNotNil(textView.textLayoutManager, "TextKit 2")
        XCTAssertFalse(textView.isRichText)
        XCTAssertEqual(textView.accessibilityLabel(), "Your notes")
        XCTAssertEqual(NotesEditor.paragraph(of: 0, in: "a\nb"), 0)
        XCTAssertEqual(NotesEditor.paragraph(of: 2, in: "a\nb"), 1)
        XCTAssertEqual(NotesEditor.paragraph(of: 99, in: "a\nb\n"), 2)
    }
}

// MARK: - First run and commands

@MainActor
final class OnboardingModelTests: XCTestCase {
    /// Quitting ends the first-run sheet (AppKit will not quit while a window has a sheet), and a
    /// sheet ended that way is neither completed nor skipped: the next launch shows it again.
    func testASheetEndedByQuittingIsNotRecordedAsSkipped() {
        let sent = Sent()
        let onboarding = OnboardingModel(send: sent.send)
        onboarding.load()
        onboarding.apply(event(#"{"type":"setting.value","key":"onboarding.done"}"#))
        XCTAssertTrue(onboarding.showing)
        onboarding.appQuitting()
        XCTAssertFalse(onboarding.showing, "the sheet goes, so AppKit can quit")
        // SwiftUI reports the sheet dismissed: that is not the user skipping it.
        onboarding.sheetDismissed()
        XCTAssertEqual(sent.commands, [.settingGet(.onboardingDone)], "nothing recorded")
        XCTAssertEqual(onboarding.completed, false, "still not completed: shown at the next launch")

        // Dismissed by the user (Escape) while the app runs: skipped, as before.
        let skipped = OnboardingModel(send: sent.send)
        skipped.apply(event(#"{"type":"setting.value","key":"onboarding.done"}"#))
        skipped.sheetDismissed()
        XCTAssertEqual(sent.commands.last, .settingSet(.onboardingDone, "true"))
        XCTAssertFalse(skipped.showing)
    }

    func testAFirstRunStateThatCannotBeReadIsShownAndLogged() {
        let logged = Logged()
        let sent = Sent()
        let onboarding = OnboardingModel(send: sent.send, log: logged.log)
        onboarding.load()
        XCTAssertEqual(sent.commands, [.settingGet(.onboardingDone)])
        let id = try? XCTUnwrap(
            (JSONSerialization.jsonObject(with: Data(CoreCommand.settingGet(.onboardingDone).json.utf8)) as? [String: Any])?["id"] as? String)
        XCTAssertEqual(id, "setting:onboarding.done")
        onboarding.apply(event(#"{"type":"command.failed","command":"setting.get","id":"setting:onboarding.done","message":"the library could not be read"}"#))
        XCTAssertTrue(onboarding.showing, "fails toward showing it")
        XCTAssertEqual(logged.messages.count, 1)
        XCTAssertTrue(logged.messages[0].contains("setting.get"), logged.messages[0])
        // Another setting's failure is not the first run's.
        let other = OnboardingModel(send: { _ in }, log: logged.log)
        other.apply(event(#"{"type":"command.failed","command":"setting.get","id":"setting:dictation.polish","message":"x"}"#))
        XCTAssertFalse(other.showing)
    }

    func testTheFirstRunStateShowsUntilItIsCompletedAndRemembersThat() {
        let sent = Sent()
        let onboarding = OnboardingModel(send: sent.send)
        XCTAssertFalse(onboarding.showing, "nothing until the store answers")
        onboarding.load()
        onboarding.apply(event(#"{"type":"setting.value","key":"onboarding.done"}"#))
        XCTAssertTrue(onboarding.showing, "never completed")
        for _ in OnboardingModel.Step.allCases { onboarding.next() }
        XCTAssertFalse(onboarding.showing)
        XCTAssertEqual(sent.commands, [.settingGet(.onboardingDone), .settingSet(.onboardingDone, "true")])
        let again = OnboardingModel(send: { _ in })
        again.apply(event(#"{"type":"setting.value","key":"onboarding.done","value":"true"}"#))
        XCTAssertFalse(again.showing)
    }
}

final class CoreCommandTests: XCTestCase {
    func testCommandsAreTheJSONTheCoreReads() throws {
        func fields(_ c: CoreCommand) throws -> [String: Any] {
            try XCTUnwrap(JSONSerialization.jsonObject(with: Data(c.json.utf8)) as? [String: Any])
        }
        let add = try fields(.noteAdd(record: "r", atMs: 754_000, text: "Pilot", ref: "note-line-0"))
        XCTAssertEqual(add["cmd"] as? String, "note.add")
        XCTAssertEqual(add["at_ms"] as? Int, 754_000, "a number, not a string")
        XCTAssertEqual(add["id"] as? String, "note-line-0")
        let done = try fields(.commitmentSetDone(id: "c", done: true))
        XCTAssertEqual(done["done"] as? Bool, true)
        XCTAssertEqual(try fields(.permissionRequest(.systemAudio))["permission"] as? String, "system_audio")
        XCTAssertEqual(try fields(.engineRoute(.livePartials))["job"] as? String, "live_partials")
        XCTAssertEqual(CoreCommand.noteAdd(record: "r", atMs: 1, text: "private words", ref: "x").name, "note.add",
                       "the name logged never carries the words")
    }
}

// MARK: - About

final class NoticesTests: XCTestCase {
    /// Every THIRD_PARTY.md row that reaches the Mac app has its notice in About.
    func testAboutCarriesTheNoticeOfEveryComponentTheAppShips() throws {
        let root = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        let table = try String(contentsOf: root.appendingPathComponent("THIRD_PARTY.md"), encoding: .utf8)
        let rows = table.split(separator: "\n").filter { $0.hasPrefix("| ") && !$0.hasPrefix("| Project") }
        let names = rows.compactMap { row -> String? in
            let cell = row.split(separator: "|").first.map(String.init)?.trimmingCharacters(in: .whitespaces) ?? ""
            // "[Name](url) by Author" or "Name, by Author": the name only.
            let name = cell.hasPrefix("[") ? String(cell.dropFirst().prefix { $0 != "]" }) : String(cell.prefix { $0 != "," })
            return name.isEmpty ? nil : name
        }
        XCTAssertGreaterThan(names.count, 15, "the table was read")
        // The legacy 0.2 app's base is not in the Mac app.
        let shipped = names.filter { $0 != "Handy" }
        let about = Notices.components.map { $0.name + " " + $0.text }.joined(separator: "\n")
        for name in shipped {
            let key = name.replacingOccurrences(of: " clustering", with: "").replacingOccurrences(of: " in C", with: "")
            XCTAssertTrue(about.localizedCaseInsensitiveContains(key), "About has no notice for \(name)")
        }
        let ids = Notices.components.map(\.id)
        XCTAssertEqual(Set(ids).count, ids.count)
        XCTAssertTrue(Notices.components.first { $0.id == "nemo-speech" }?.text.contains("NVIDIA CORPORATION & AFFILIATES") == true,
                      "NeMo-Speech.cpp's NOTICE")
        XCTAssertTrue(Notices.components.first { $0.id == "aec3" }?.text.localizedCaseInsensitiveContains("patent") == true,
                      "WebRTC's patent grant")
        XCTAssertTrue(Notices.components.allSatisfy { !$0.text.isEmpty })
        XCTAssertEqual(Notices.models.first { $0.id == "parakeet" }?.licence, "CC-BY-4.0")
        XCTAssertNotNil(Notices.models.first { $0.id == "parakeet" }?.notice, "CC-BY needs its credit")
    }
}

// MARK: - The commands against the real core

/// The commands the screens build are the ones the core reads, and its answers decode. A fresh
/// library in a temp directory; the permission check reads the real probe, which never prompts
/// (system audio reads "not determined" because this library never asked for it).
final class ScreensCoreContractTests: XCTestCase {
    func testEveryScreenCommandIsReadByTheCoreAndAnsweredWithAnEventThisShellDecodes() throws {
        let data = FileManager.default.temporaryDirectory.appendingPathComponent("inkwell-screens-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: data) }
        let events = Mutex<[InkEvent]>([])
        let session = try InkSession.start(InkConfig(dataDir: data.path, logLevel: "warn")) { event in
            events.withLock { $0.append(event) }
        }
        defer { session.shutdown() }
        func answer<T>(_ command: CoreCommand, _ pick: (InkEvent) -> T?) throws -> T? {
            let before = events.withLock { $0.count }
            try session.command(command.json)
            let until = Date().addingTimeInterval(10)
            while Date() < until {
                if let found = events.withLock({ Array($0.dropFirst(before)) }).lazy.compactMap(pick).first {
                    return found
                }
                Thread.sleep(forTimeInterval: 0.01)
            }
            return nil
        }
        let polish = try answer(.settingSet(.dictationPolish, "on")) { if case .settingValue(let v) = $0 { v } else { nil } }
        XCTAssertEqual(polish?.value, "on")
        let onboarding = try answer(.settingGet(.onboardingDone)) { if case .settingValue(let v) = $0 { v } else { nil } }
        XCTAssertEqual(onboarding?.key, "onboarding.done")
        XCTAssertNil(onboarding?.value, "a fresh library has not onboarded")
        let modes = try answer(.modesList) { if case .modesListed(let m) = $0 { m } else { nil } }
        XCTAssertEqual(modes?.modes.map(\.name), ["Default"])
        let owed = try answer(.commitmentsList) { if case .commitmentsListed(let c) = $0 { c } else { nil } }
        XCTAssertEqual(owed?.items, [])
        XCTAssertNotNil(try answer(.modelsList) { if case .modelsListed(let m) = $0 { m } else { nil } })
        XCTAssertNotNil(try answer(.engineRoute(.dictationFinal)) { if case .engineRouted(let r) = $0 { r } else { nil } })
        let permissions = try answer(.permissionsCheck) { if case .permissionsChecked(let p) = $0 { p } else { nil } }
        XCTAssertEqual(permissions?.systemAudio, .notDetermined, "never asked here, so never probed")
        let refused = try answer(.noteAdd(record: "no-such-record", atMs: 1_000, text: "private words", ref: "note-line-7")) {
            if case .commandFailed(let f) = $0 { f } else { nil }
        }
        XCTAssertEqual(refused?.id, "note-line-7", "a refused note is matched to its line")
        XCTAssertFalse(refused?.message.contains("private") ?? true, "the error never quotes the note")
        let undecodable = events.withLock { $0 }.filter { if case .undecodable = $0 { true } else { false } }
        XCTAssertEqual(undecodable, [])
    }
}

#if DEBUG
final class ReplayOnLaunchTests: XCTestCase {
    func testAReplayIsAskedForOnlyWithAbsolutePaths() {
        XCTAssertNil(ReplayOnLaunch.command(from: [:]))
        XCTAssertNil(ReplayOnLaunch.command(from: ["INK_REPLAY_MEETING": "relative.wav"]))
        XCTAssertNil(ReplayOnLaunch.command(from: ["INK_REPLAY_MEETING": "/m.wav,far.wav"]))
        XCTAssertEqual(ReplayOnLaunch.command(from: ["INK_REPLAY_MEETING": "/m.wav"])?["mic"], "/m.wav")
        let both = ReplayOnLaunch.command(from: ["INK_REPLAY_MEETING": "/m.wav, /f.wav"])
        XCTAssertEqual(both?["far"], "/f.wav")
        XCTAssertEqual(both?["cmd"], "replay_meeting")
    }
}
#endif

/// The window follows its content's minimum size (MainWindowController's hosting controller), so a
/// screen whose minimum grows with its content grows the window off the screen.
@MainActor
final class LiveLayoutTests: XCTestCase {
    func testALongMeetingAndLongNotesNeverRaiseTheLiveScreensMinimumSize() throws {
        let store = CoreStore()
        var batch = [event(#"{"type":"meeting.started","record":"rec"}"#)]
        for i in 0..<80 {
            let channel = i % 2 == 0 ? "far" : "mic"
            batch.append(event(#"{"type":"meeting.final","record":"rec","channel":"\#(channel)","start_ms":\#(i * 4000),"end_ms":\#(i * 4000 + 3000),"text":"A settled line of speech, number \#(i), long enough to wrap onto a second line in the ledger."}"#))
        }
        store.apply(batch)
        let live = LiveModel(send: { _ in })
        live.apply(batch[0])
        live.notesEdited(Array(repeating: "A note line", count: 60).joined(separator: "\n"), caretParagraph: 59)
        let meeting = try XCTUnwrap(store.meeting)
        let hosting = NSHostingController(rootView: LiveMeetingView(meeting: meeting, live: live))
        let minimum = hosting.sizeThatFits(in: .zero)
        XCTAssertLessThan(minimum.height, 460, "the window's minimum content height is 460")
        XCTAssertLessThan(minimum.width, 720)
    }

    func testOwedAndSettingsNeverRaiseTheWindowsMinimumSizeEither() {
        let screens = ScreenModels(send: { _ in }, calendar: FakeCalendar(), apps: WorkspaceApps())
        screens.owed.apply(event(#"{"type":"commitments.listed","items":[{"id":"a","record":"r","record_started_at_unix_ms":0,"text":"A promise long enough to wrap onto more than one line when the window is narrow","merged":1}]}"#))
        for view in [
            AnyView(OwedScreen()), AnyView(SettingsScreen()), AnyView(LiveScreen()),
        ] {
            let hosting = NSHostingController(
                rootView: view.environment(screens).environment(CoreStore()).environment(Updates(infoDictionary: nil)))
            let minimum = hosting.sizeThatFits(in: .zero)
            XCTAssertLessThan(minimum.height, 460)
            XCTAssertLessThan(minimum.width, 720)
        }
    }
}

/// The core's own timeout event, as ink-ffi's llm test shows it sent (`dictation.warning` with kind
/// `polish_timed_out` and no text), reaches the toggle through the path CoreController uses.
@MainActor
final class PolishTimeoutPathTests: XCTestCase {
    func testTheCoresTimeoutEventMakesTheToggleSayItKeepsTimingOut() throws {
        let timedOut = try InkEvent.decode(Data(#"{"type":"dictation.warning","kind":"polish_timed_out"}"#.utf8))
        guard case .dictationWarningEvent(let warning) = timedOut else {
            return XCTFail("not a dictation warning: \(timedOut)")
        }
        XCTAssertEqual(warning.kind, .polishTimedOut)
        XCTAssertNil(warning.message, "no text")

        let screens = ScreenModels(send: { _ in }, calendar: FakeCalendar(), apps: WorkspaceApps())
        screens.apply([
            event(#"{"type":"engine.registered","id":"apple-foundation-models","kind":"llm","jobs":[]}"#),
            event(#"{"type":"setting.value","key":"dictation.polish","value":"on"}"#),
        ])
        for _ in 0..<PolishModel.timeoutWarning {
            screens.apply([
                event(#"{"type":"dictation.started"}"#), timedOut,
                event(#"{"type":"dictation.inserted","text":"x","outcome":"pasted"}"#),
            ])
        }
        XCTAssertTrue(screens.polish.keepsTimingOut)
        XCTAssertEqual(screens.polish.status, "Polish keeps timing out, so your words go in as you said them.")
    }
}

// MARK: - The controller

@MainActor
final class CoreControllerCommandTests: XCTestCase {
    /// A line still under the caret when the app quits reaches the core before it stops: the
    /// flush runs while the session can still send, and the core runs queued commands before its
    /// shutdown returns.
    func testANoteStillUnderTheCaretIsSavedWhenTheAppQuits() async throws {
        let data = FileManager.default.temporaryDirectory.appendingPathComponent("inkwell-quit-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: data) }
        let logged = Logged()
        let core = CoreController(registersAppleEngines: false, commandLog: logged.log)
        core.start(environment: ["INK_DATA_DIR": data.path])
        try await until { if case .ready = core.store.status { true } else { false } }
        let fixtures = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .deletingLastPathComponent().appendingPathComponent("fixtures/ami")
        core.send([
            "cmd": "replay_meeting", "mic": fixtures.appendingPathComponent("IS1009a-mic.wav").path,
            "far": fixtures.appendingPathComponent("IS1009a-far.wav").path,
        ])
        try await until { core.screens.live.record != nil }
        // Typed, the caret still in it: nothing has been handed to the core yet.
        core.screens.live.notesEdited("A line still being typed", caretParagraph: 0)

        await withCheckedContinuation { (done: CheckedContinuation<Void, Never>) in
            core.stop { done.resume() }
        }
        XCTAssertEqual(try Self.notes(in: data), ["A line still being typed"])
        XCTAssertEqual(logged.messages, [], "nothing was dropped")

        // After the stop, a command has nowhere to go: said, never silently dropped.
        core.send(.modesList)
        XCTAssertEqual(logged.messages.count, 1)
        XCTAssertTrue(logged.messages[0].contains("modes.list"), logged.messages[0])
    }

    /// A screen command whose failure no screen handles is logged, by name only.
    func testAFailureNoScreenHandlesIsLoggedByNameOnly() throws {
        let logged = Logged()
        let core = CoreController(registersAppleEngines: false, commandLog: logged.log)
        core.received([event(#"{"type":"command.failed","command":"setting.set","id":"setting:dictation.polish","message":"the library could not be written: zebra"}"#)])
        XCTAssertEqual(logged.messages.count, 1)
        XCTAssertTrue(logged.messages[0].contains("setting.set"), logged.messages[0])
        XCTAssertFalse(logged.messages[0].contains("zebra"), "never the core's message or a field")
        XCTAssertFalse(logged.messages[0].contains("dictation.polish"))
        // Handled ones are the screens' to show.
        for handled in ["permissions.check", "models.list", "modes.list", "commitment.set_done", "note.add", "note.update", "note.delete"] {
            core.received([event(#"{"type":"command.failed","command":"\#(handled)","message":"x"}"#)])
        }
        // (Owed lists again after a failed set_done; with no core running that is logged as not sent.)
        func failures() -> [String] { logged.messages.filter { $0.hasPrefix("command.failed") } }
        XCTAssertEqual(failures().count, 1)
        core.received([event(#"{"type":"command.failed","command":"setting.get","id":"setting:dictation.polish","message":"x"}"#)])
        XCTAssertEqual(failures().count, 2, "a setting only onboarding reads is handled only for onboarding")
    }

    private func until(_ timeout: Duration = .seconds(20), _ done: () -> Bool) async throws {
        let start = ContinuousClock.now
        while !done() {
            if ContinuousClock.now - start > timeout {
                throw Timeout()
            }
            try await Task.sleep(for: .milliseconds(20))
        }
    }

    private struct Timeout: Error {}
    private struct Unreadable: Error {}

    /// The notes in the library at `data`, read-only.
    private static func notes(in data: URL) throws -> [String] {
        var db: OpaquePointer?
        let path = data.appendingPathComponent("library.sqlite").path
        guard sqlite3_open_v2(path, &db, SQLITE_OPEN_READONLY, nil) == SQLITE_OK else {
            throw Unreadable()
        }
        defer { sqlite3_close(db) }
        var statement: OpaquePointer?
        guard sqlite3_prepare_v2(db, "SELECT text FROM note ORDER BY seq", -1, &statement, nil) == SQLITE_OK else {
            throw Unreadable()
        }
        defer { sqlite3_finalize(statement) }
        var out: [String] = []
        while sqlite3_step(statement) == SQLITE_ROW {
            out.append(String(cString: sqlite3_column_text(statement, 0)))
        }
        return out
    }
}
