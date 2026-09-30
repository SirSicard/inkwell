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

/// `consent.state` for polish (or `feature`) as the core sends it.
private func polishState(
    on: Bool, allowed: Bool, to: String? = "on_device", name: String = "SystemLanguageModel.default",
    endpoint: String? = nil, allowedTo: String? = nil, error: String? = nil, feature: String = "polish"
) -> InkEvent {
    var fields: [String: Any] = ["type": "consent.state", "feature": feature, "on": on, "allowed": allowed]
    if let to { fields["to"] = to; fields["name"] = name }
    if let endpoint { fields["endpoint"] = endpoint }
    if let allowedTo { fields["allowed_to"] = allowedTo }
    if let error { fields["error"] = error }
    let data = try! JSONSerialization.data(withJSONObject: fields)
    return event(String(decoding: data, as: UTF8.self))
}

private let appleLLM = #"{"type":"engine.registered","id":"apple-foundation-models","kind":"llm","jobs":[]}"#

@MainActor
final class PolishModelTests: XCTestCase {
    /// Verify: the Polish toggle can't read "on" without a working engine.
    func testTheToggleNeverReadsOnWithoutAWorkingEngine() {
        let sent = Sent()
        let polish = PolishModel(send: sent.send)
        polish.apply(polishState(on: true, allowed: true, allowedTo: "on_device"))
        XCTAssertEqual(polish.preference, true)
        XCTAssertFalse(polish.isOn, "wished for, but nothing can polish")
        XCTAssertFalse(polish.canToggle)
        polish.setOn(true)
        XCTAssertNil(polish.pendingConsent, "a toggle with no engine asks nothing")
        XCTAssertEqual(sent.commands, [], "and sends nothing")

        // Apple Intelligence is off: the engines say why, and it still reads off.
        polish.appleEnginesReported(.unavailable(code: AppleIntelligence.Reason.notEnabled.rawValue))
        XCTAssertFalse(polish.isOn)
        XCTAssertEqual(polish.status, "Polish needs Apple Intelligence. Turn it on in System Settings.")
        // The engines registering it is not enough: the core has to confirm it.
        polish.appleEnginesReported(.registered)
        XCTAssertFalse(polish.isOn)

        polish.apply(event(appleLLM))
        XCTAssertTrue(polish.isOn)
        XCTAssertTrue(polish.canToggle)
        XCTAssertEqual(sent.commands, [.consentGet(.polish, ref: "consent.get:polish:1")], "a new model: where polish sends is read again")

        // A speech engine is not a language model.
        polish.apply(event(#"{"type":"engine.unregistered","id":"apple-foundation-models"}"#))
        polish.apply(event(#"{"type":"engine.registered","id":"parakeet","kind":"streaming","jobs":[{"job":"live_partials","wer":21.3}]}"#))
        XCTAssertFalse(polish.isOn, "let go of when Apple Intelligence went away")

        polish.apply(event(appleLLM))
        polish.apply(event(#"{"type":"core.stopped"}"#))
        XCTAssertFalse(polish.isOn, "a stopped core holds no engine")
    }

    /// Owner decision: turning polish on asks first, naming where the words go; Cancel sends
    /// nothing and leaves it off; Allow asks the core to record the consent.
    func testTurningItOnAsksFirstAndCancelLeavesItOff() {
        let sent = Sent()
        let polish = PolishModel(send: sent.send)
        polish.load()
        XCTAssertEqual(sent.commands, [.consentGet(.polish, ref: "consent.get:polish:1")])
        polish.apply(event(appleLLM))
        polish.apply(polishState(on: false, allowed: false))
        XCTAssertFalse(polish.isOn)
        XCTAssertEqual(polish.status, "Off. Your words go in as you said them.")

        polish.setOn(true)
        let asked = try? XCTUnwrap(polish.pendingConsent)
        XCTAssertEqual(asked?.label, "Apple's on-device model")
        XCTAssertEqual(sent.commands, [.consentGet(.polish, ref: "consent.get:polish:1"), .consentGet(.polish, ref: "consent.get:polish:2")], "asking sends nothing")
        let message = asked.map(PolishModel.consentMessage) ?? ""
        XCTAssertTrue(message.contains("sends what you dictate to a language model"), message)
        XCTAssertTrue(message.contains("Apple's on-device model, so your words stay on this Mac"), message)
        XCTAssertFalse(message.lowercased().contains("invisible") || message.lowercased().contains("undetectable"))

        polish.cancelConsent()
        XCTAssertNil(polish.pendingConsent)
        XCTAssertFalse(polish.isOn, "cancel leaves it off")
        XCTAssertEqual(sent.commands, [.consentGet(.polish, ref: "consent.get:polish:1"), .consentGet(.polish, ref: "consent.get:polish:2")], "and sends nothing")

        polish.setOn(true)
        polish.allowConsent()
        XCTAssertEqual(sent.commands.last, .consentAllow(feature: .polish, to: .onDevice, endpoint: nil, key: nil, ref: "consent.allow:polish:3"))
        XCTAssertFalse(polish.isOn, "on only once the core has recorded it")
        polish.apply(polishState(on: true, allowed: true, allowedTo: "on_device"))
        XCTAssertTrue(polish.isOn)
        XCTAssertEqual(polish.status, "On, with Apple's on-device model. Your words stay on this Mac.")
    }

    /// A cloud model is named, and the step says the words leave this Mac for it.
    func testACloudModelIsNamedAndTheStepSaysTheWordsLeaveThisMac() throws {
        let sent = Sent()
        let polish = PolishModel(send: sent.send)
        polish.apply(event(#"{"type":"engine.registered","id":"cloud","kind":"llm","jobs":[]}"#))
        polish.apply(polishState(on: false, allowed: false, to: "cloud", name: "Example Cloud", endpoint: "shell engine cloud"))
        polish.setOn(true)
        let asked = try XCTUnwrap(polish.pendingConsent)
        let message = PolishModel.consentMessage(asked)
        XCTAssertTrue(message.contains("your words leave this Mac and go to Example Cloud"), message)
        XCTAssertEqual(PolishModel.consentButton(asked), "Send to Example Cloud")
        polish.allowConsent()
        XCTAssertEqual(sent.commands.last, .consentAllow(feature: .polish, to: .cloud, endpoint: "shell engine cloud", key: nil, ref: "consent.allow:polish:2"))
        polish.apply(polishState(on: true, allowed: true, to: "cloud", name: "Example Cloud", endpoint: "shell engine cloud", allowedTo: "cloud"))
        XCTAssertEqual(polish.status, "On. Your words go to Example Cloud before they are typed.")
    }

    /// The model moves from this Mac to a cloud provider while polish is on: it reads off, says
    /// why, and turning it on asks about the new destination.
    func testAModelThatMovesPausesPolishAndSaysWhy() throws {
        let polish = PolishModel(send: { _ in })
        polish.apply(event(appleLLM))
        polish.apply(polishState(on: true, allowed: true, allowedTo: "on_device"))
        XCTAssertTrue(polish.isOn)
        polish.apply(event(#"{"type":"engine.registered","id":"cloud","kind":"llm","jobs":[]}"#))
        polish.apply(polishState(on: true, allowed: false, to: "cloud", name: "Example Cloud", endpoint: "shell engine cloud", allowedTo: "on_device"))
        XCTAssertFalse(polish.isOn, "nothing is polished")
        XCTAssertTrue(polish.isPaused)
        XCTAssertTrue(polish.isProblem)
        XCTAssertEqual(polish.status, "Paused: polish would now send your words to Example Cloud. Turn it on again to allow it.")
        polish.setOn(true)
        XCTAssertEqual(try XCTUnwrap(polish.pendingConsent).kind, .cloud(endpoint: "shell engine cloud"))
        // A consent that could not be read is none, and says so.
        polish.cancelConsent()
        polish.apply(polishState(on: true, allowed: false, allowedTo: nil, error: "couldn't read your polish consent"))
        XCTAssertEqual(polish.status, "Couldn't read your polish consent, so polish is off or paused. Turn it on again to allow it.")
    }

    /// The step asked about one destination and the model moved before Allow: the step closes
    /// (its words are never swapped under the user), nothing is sent, and asking again names the
    /// new destination.
    func testTheStepClosesIfTheModelMovesWhileItIsShown() throws {
        let sent = Sent()
        let polish = PolishModel(send: sent.send)
        polish.apply(event(appleLLM))
        polish.apply(polishState(on: false, allowed: false))
        polish.setOn(true, from: .onboarding)
        XCTAssertEqual(polish.consentHost, .onboarding, "shown where it was asked")
        polish.apply(polishState(on: false, allowed: false, to: "cloud", name: "Example Cloud", endpoint: "shell engine cloud"))
        XCTAssertNil(polish.pendingConsent)
        XCTAssertNil(polish.consentHost)
        polish.allowConsent()
        XCTAssertFalse(sent.commands.contains { if case .consentAllow = $0 { true } else { false } }, "nothing sent")
        polish.setOn(true)
        XCTAssertEqual(try XCTUnwrap(polish.pendingConsent).name, "Example Cloud")
        XCTAssertEqual(polish.consentHost, .settings)
    }

    func testSwitchingOffSendsOffAndAFailedSavePutsItBack() {
        let sent = Sent()
        let polish = PolishModel(send: sent.send)
        polish.apply(event(appleLLM))
        polish.apply(polishState(on: true, allowed: true, allowedTo: "on_device"))
        polish.setOn(false)
        XCTAssertEqual(sent.commands.last, .settingSet(.dictationPolish, "off"))
        XCTAssertFalse(polish.isOn)
        polish.apply(event(#"{"type":"command.failed","command":"setting.set","id":"setting:dictation.polish","message":"x"}"#))
        XCTAssertTrue(polish.isOn, "put back")
        XCTAssertEqual(polish.status, "Couldn't save the change, so polish stays as it was.")
        // A consent the core could not record says so.
        polish.apply(polishState(on: false, allowed: false))
        polish.setOn(true)
        polish.allowConsent()
        polish.apply(event(#"{"type":"command.failed","command":"consent.allow","id":"consent.allow:polish:2","message":"x"}"#))
        XCTAssertEqual(polish.status, "Couldn't turn polish on, so it stays off. Try again.")
    }

    /// The §6 point: a timeout reads apart from an ordinary cancel or failure.
    func testPolishThatKeepsTimingOutSaysSoAndAPlainFailureDoesNot() {
        let polish = PolishModel(send: { _ in })
        polish.apply(event(appleLLM))
        polish.apply(polishState(on: true, allowed: true, allowedTo: "on_device"))
        func take(_ warning: String?) {
            polish.apply(event(#"{"type":"dictation.started","take":0,"edit":false}"#))
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

    /// Answers can arrive out of order: only the answer to the newest request is applied, and a
    /// failure counts only for the request that is still the newest of its kind.
    func testOnlyTheAnswerToTheNewestRequestIsApplied() {
        let sent = Sent()
        let polish = PolishModel(send: sent.send)
        polish.apply(event(appleLLM))  // consent.get:polish:1
        polish.load()                   // consent.get:polish:2
        XCTAssertEqual(sent.commands, [.consentGet(.polish, ref: "consent.get:polish:1"), .consentGet(.polish, ref: "consent.get:polish:2")])
        func answer(_ ref: String, on: Bool) -> InkEvent {
            event(#"{"type":"consent.state","feature":"polish","on":\#(on),"allowed":\#(on),"to":"on_device","name":"SystemLanguageModel.default","ref":"\#(ref)"}"#)
        }
        polish.apply(answer("consent.get:polish:2", on: true))
        XCTAssertTrue(polish.isOn)
        polish.apply(answer("consent.get:polish:1", on: false))
        XCTAssertTrue(polish.isOn, "the older answer, arriving late, is not applied")
        polish.apply(event(#"{"type":"command.failed","command":"consent.get","id":"consent.get:polish:1","message":"x"}"#))
        XCTAssertNil(polish.failure, "nor is an older request's failure")
        // A state with no ref (the core's own, after a switch-off) always applies.
        polish.apply(polishState(on: false, allowed: false))
        XCTAssertFalse(polish.isOn)
    }

    /// A part of the state the core could not read is said whenever it is there: with polish off
    /// (an unread switch reads as off in the core) and with no working model.
    func testAReadErrorIsShownWhetherOrNotPolishIsOn() {
        let polish = PolishModel(send: { _ in })
        polish.apply(polishState(on: false, allowed: false, error: "couldn't read the switch"))
        XCTAssertEqual(polish.status, "Couldn't read the switch, so polish is off or paused. Turn it on again to allow it.")
        XCTAssertTrue(polish.isProblem)
        polish.apply(event(appleLLM))
        XCTAssertEqual(polish.status, "Couldn't read the switch, so polish is off or paused. Turn it on again to allow it.")
        polish.apply(polishState(on: false, allowed: false))
        XCTAssertEqual(polish.status, "Off. Your words go in as you said them.")
    }

    /// A take the core refused to polish says so in the Drop, and the toggle reads the state again.
    func testATakeRefusedForConsentSaysSo() {
        let sent = Sent()
        let polish = PolishModel(send: sent.send)
        let refused = event(#"{"type":"dictation.warning","kind":"polish_not_allowed","message":"Example Cloud"}"#)
        polish.apply(refused)
        XCTAssertEqual(sent.commands, [.consentGet(.polish, ref: "consent.get:polish:1")])
        let note = DictationModel.note(for: refused, hasLanguageModel: true)
        XCTAssertEqual(note?.title, "Not polished")
    }
}

// MARK: - Voice edit's consent

@MainActor
final class EditConsentTests: XCTestCase {
    private func screens(_ sent: Sent) -> ScreenModels {
        let screens = ScreenModels(send: sent.send, calendar: FakeCalendar(), apps: WorkspaceApps())
        screens.apply([
            event(appleLLM),
            polishState(on: false, allowed: false, feature: "edit"),
        ])
        return screens
    }

    /// Picking an edit key from Off asks first, naming where the selection goes; Cancel sends
    /// nothing; Allow sends the consent with the key.
    func testPickingAKeyAsksFirstAndCancelChangesNothing() throws {
        let sent = Sent()
        let screens = screens(sent)
        let before = sent.commands.count
        screens.chooseEditKey("right_command")
        let asked = try XCTUnwrap(screens.editConsent.pending)
        XCTAssertEqual(screens.editConsent.host, .settings)
        XCTAssertEqual(sent.commands.count, before, "asking sends nothing")
        let message = ConsentModel.message(.edit, asked)
        XCTAssertTrue(message.contains("sends the text you select and what you say to a language model"), message)
        XCTAssertTrue(message.contains("Apple's on-device model, so your words stay on this Mac"), message)
        XCTAssertEqual(ConsentModel.button(.edit, asked), "Turn On Voice Edit")
        XCTAssertNil(screens.dictation.editKey, "asking changes no key")
        screens.editConsent.cancel()
        XCTAssertNil(screens.editConsent.pending)
        XCTAssertEqual(sent.commands.count, before, "cancel sends nothing")
        XCTAssertNil(screens.dictation.editKey, "cancel leaves voice edit off")

        screens.chooseEditKey("right_command")
        screens.editConsent.allow()
        XCTAssertEqual(sent.commands.last, .consentAllow(feature: .edit, to: .onDevice, endpoint: nil, key: "right_command", ref: "consent.allow:edit:2"))
    }

    /// On and allowed: another key only changes the key. Off sends off (the core withdraws the
    /// consent with it).
    func testAKeyChangeWhileAllowedAsksNothingAndOffTurnsItOff() {
        let sent = Sent()
        let screens = screens(sent)
        screens.apply([
            event(#"{"type":"setting.value","key":"dictation.edit_key","value":"right_command"}"#),
            polishState(on: true, allowed: true, allowedTo: "on_device", feature: "edit"),
        ])
        screens.chooseEditKey("right_option")
        XCTAssertNil(screens.editConsent.pending)
        XCTAssertEqual(sent.commands.last, .settingSet(.dictationEditKey, "right_option"))
        screens.chooseEditKey(nil)
        XCTAssertEqual(sent.commands.last, .settingSet(.dictationEditKey, "off"))
        XCTAssertFalse(screens.editConsent.isAllowedOn)
    }

    /// The model moved to a cloud provider: voice edit is paused and says so, a key pick asks about
    /// the new destination, and a refused edit shows in the Drop and reads the state again.
    func testAMovedModelPausesVoiceEditAndAKeyPickAsksAgain() throws {
        let sent = Sent()
        let screens = screens(sent)
        screens.apply([
            event(#"{"type":"setting.value","key":"dictation.edit_key","value":"right_command"}"#),
            polishState(on: true, allowed: false, to: "cloud", name: "Example Cloud", endpoint: "shell engine cloud", allowedTo: "on_device", feature: "edit"),
        ])
        XCTAssertEqual(screens.editConsent.problem, "Paused: voice edit would now send your words to Example Cloud. Turn it on again to allow it.")
        screens.chooseEditKey("right_option")
        XCTAssertEqual(try XCTUnwrap(screens.editConsent.pending).kind, .cloud(endpoint: "shell engine cloud"))
        XCTAssertEqual(ConsentModel.button(.edit, try XCTUnwrap(screens.editConsent.pending)), "Send to Example Cloud")
        let refused = event(#"{"type":"dictation.edit_failed","reason":"not_allowed","message":"Example Cloud"}"#)
        let before = sent.commands.count
        screens.apply([refused])
        XCTAssertEqual(sent.commands.dropFirst(before).first, .consentGet(.edit, ref: "consent.get:edit:2"))
        XCTAssertEqual(DictationModel.note(for: refused, hasLanguageModel: true)?.title, "Not edited")
    }

    /// Polish's state never moves voice edit's, nor the reverse.
    func testEachFeatureReadsOnlyItsOwnState() {
        let sent = Sent()
        let screens = screens(sent)
        screens.apply([polishState(on: true, allowed: true, allowedTo: "on_device")])
        XCTAssertTrue(screens.polish.consent.isAllowedOn)
        XCTAssertFalse(screens.editConsent.isAllowedOn)
    }
}

// MARK: - Summaries and Ask's consent

@MainActor
final class MeetingsConsentTests: XCTestCase {
    private func screens(_ sent: Sent) -> ScreenModels {
        let screens = ScreenModels(send: sent.send, calendar: FakeCalendar(), apps: WorkspaceApps())
        screens.apply([
            event(appleLLM),
            polishState(on: false, allowed: false, feature: "meetings"),
        ])
        return screens
    }

    /// Owner decision: summaries and Ask are off until the user turns them on in Settings > AI
    /// through the consent step, which names where the transcript goes; Cancel sends nothing;
    /// Allow asks the core to record it; off sends off (the core withdraws the consent with it).
    /// While off, a record without a summary says why.
    func testTurningThemOnAsksFirstAndARecordSaysTheyAreOff() throws {
        let sent = Sent()
        let screens = screens(sent)
        XCTAssertFalse(screens.meetingsAIOn)
        XCTAssertTrue(screens.canToggleMeetingsAI)
        XCTAssertEqual(screens.meetingsAIStatus, "Off. Meetings are recorded and transcribed, with no summary, and Ask stays off.")
        XCTAssertEqual(screens.summaryOffNote, "Summaries are off until you allow them in Settings > AI. Meetings are still recorded and transcribed.")

        let before = sent.commands.count
        screens.setMeetingsAI(true)
        let asked = try XCTUnwrap(screens.meetingsConsent.pending)
        XCTAssertEqual(screens.meetingsConsent.host, .settings)
        XCTAssertEqual(sent.commands.count, before, "asking sends nothing")
        XCTAssertEqual(ConsentModel.title(.meetings), "Turn on summaries and Ask?")
        let message = ConsentModel.message(.meetings, asked)
        XCTAssertTrue(message.contains("send a meeting's transcript, what everyone in it said, to a language model"), message)
        XCTAssertTrue(message.contains("Apple's on-device model, so the transcript stays on this Mac"), message)
        XCTAssertEqual(ConsentModel.button(.meetings, asked), "Turn On Summaries and Ask")
        screens.meetingsConsent.cancel()
        XCTAssertNil(screens.meetingsConsent.pending)
        XCTAssertEqual(sent.commands.count, before, "cancel sends nothing")
        XCTAssertFalse(screens.meetingsAIOn, "cancel leaves them off")

        screens.setMeetingsAI(true)
        screens.meetingsConsent.allow()
        XCTAssertEqual(sent.commands.last, .consentAllow(feature: .meetings, to: .onDevice, endpoint: nil, key: nil, ref: "consent.allow:meetings:2"))
        XCTAssertFalse(screens.meetingsAIOn, "on only once the core has recorded it")
        screens.apply([polishState(on: true, allowed: true, allowedTo: "on_device", feature: "meetings")])
        XCTAssertTrue(screens.meetingsAIOn)
        XCTAssertNil(screens.summaryOffNote)
        XCTAssertEqual(screens.meetingsAIStatus, "On, with Apple's on-device model. Meeting transcripts stay on this Mac.")

        screens.setMeetingsAI(false)
        XCTAssertEqual(sent.commands.last, .settingSet(.meetingsLLM, "off"))
        XCTAssertFalse(screens.meetingsAIOn)
        XCTAssertNotNil(screens.summaryOffNote)
    }

    /// A cloud model is named, and the step says the transcript leaves this Mac for it.
    func testACloudModelIsNamedAndTheTranscriptLeavesThisMac() throws {
        let sent = Sent()
        let screens = screens(sent)
        screens.apply([polishState(on: false, allowed: false, to: "cloud", name: "Example Cloud", endpoint: "shell engine cloud", feature: "meetings")])
        screens.setMeetingsAI(true)
        let asked = try XCTUnwrap(screens.meetingsConsent.pending)
        let message = ConsentModel.message(.meetings, asked)
        XCTAssertTrue(message.contains("the transcript leaves this Mac and goes to Example Cloud"), message)
        XCTAssertEqual(ConsentModel.button(.meetings, asked), "Send to Example Cloud")
        screens.meetingsConsent.allow()
        XCTAssertEqual(sent.commands.last, .consentAllow(feature: .meetings, to: .cloud, endpoint: "shell engine cloud", key: nil, ref: "consent.allow:meetings:2"))
        screens.apply([polishState(on: true, allowed: true, to: "cloud", name: "Example Cloud", endpoint: "shell engine cloud", allowedTo: "cloud", feature: "meetings")])
        XCTAssertEqual(screens.meetingsAIStatus, "On. Meeting transcripts go to Example Cloud for summaries and Ask.")
    }

    /// A meeting that ended without a summary for want of consent reads the state again; a
    /// switch-off the core refused is shown and put back.
    func testARefusedSummaryReadsTheStateAgainAndAFailedOffIsShown() {
        let sent = Sent()
        let screens = screens(sent)
        let before = sent.commands.count
        screens.apply([event(#"{"type":"meeting.warning","record":"r1","kind":"summary_not_allowed","message":"a model on this machine"}"#)])
        XCTAssertEqual(sent.commands.dropFirst(before).first, .consentGet(.meetings, ref: "consent.get:meetings:2"))

        screens.apply([polishState(on: true, allowed: true, allowedTo: "on_device", feature: "meetings")])
        screens.setMeetingsAI(false)
        XCTAssertFalse(screens.meetingsAIOn)
        let failed = event(#"{"type":"command.failed","command":"setting.set","id":"setting:meetings.llm","message":"x"}"#)
        guard case .commandFailed(let value) = failed else { return XCTFail("not a failure") }
        XCTAssertTrue(screens.handles(value), "shown under the switch, not only logged")
        screens.apply([failed])
        XCTAssertTrue(screens.meetingsAIOn, "put back")
        XCTAssertEqual(screens.meetingsAIStatus, "Couldn't save the change, so summaries and Ask stay as they were.")
    }

    /// Each feature reads only its own state: summaries and Ask allowed allows neither polish nor
    /// voice edit, nor the reverse.
    func testEachFeatureReadsOnlyItsOwnState() {
        let sent = Sent()
        let screens = screens(sent)
        screens.apply([polishState(on: true, allowed: true, allowedTo: "on_device", feature: "meetings")])
        XCTAssertTrue(screens.meetingsAIOn)
        XCTAssertFalse(screens.polish.consent.isAllowedOn)
        XCTAssertFalse(screens.editConsent.isAllowedOn)
        screens.apply([
            polishState(on: false, allowed: false, feature: "meetings"),
            polishState(on: true, allowed: true, allowedTo: "on_device"),
            polishState(on: true, allowed: true, allowedTo: "on_device", feature: "edit"),
        ])
        XCTAssertFalse(screens.meetingsAIOn)
        XCTAssertNotNil(screens.summaryOffNote)
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
        XCTAssertFalse(catalogue.listed, "not answered yet: not known, which is not empty")
        catalogue.apply(event(#"{"type":"command.failed","command":"models.list","message":"the registry could not be read"}"#))
        XCTAssertTrue(catalogue.failed)
        XCTAssertEqual(CatalogueModel.failedText, "The model list could not be read.")
        catalogue.apply(event(#"{"type":"models.listed","models":[]}"#))
        XCTAssertFalse(catalogue.failed, "a list that arrives clears it")
        XCTAssertTrue(catalogue.listed)
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

    /// A route question that failed reads "couldn't", never "nothing installed", until its answer
    /// comes; it is matched to its line by the id the command carries.
    func testAFailedRouteQuestionSaysSoOnItsLine() {
        let catalogue = CatalogueModel(send: { _ in })
        guard case .commandFailed(let failed) = CoreCommand.engineRoute(.meetingFinal).notSent("couldn't send it: the core is not running")
        else { return XCTFail("not a command.failed") }
        XCTAssertEqual(CatalogueModel.routeJob(failed), .meetingFinal)
        catalogue.apply(.commandFailed(failed))
        XCTAssertEqual(catalogue.line(.meetingFinal).engineText, CatalogueModel.routeFailedText)
        XCTAssertEqual(catalogue.line(.dictationFinal).engineText, "Checking…")
        catalogue.apply(event(#"{"type":"engine.routed","job":"meeting_final"}"#))
        XCTAssertEqual(catalogue.line(.meetingFinal).engineText, "Nothing installed yet", "the answer clears it")

        // The core's own failure carries the same id.
        catalogue.apply(event(#"{"type":"command.failed","command":"engine.route","id":"engine.route:live_partials","message":"the command thread has stopped"}"#))
        XCTAssertTrue(catalogue.line(.livePartials).failed)
        catalogue.apply(event(#"{"type":"core.stopped"}"#))
        XCTAssertFalse(catalogue.line(.livePartials).failed, "a stopped core's answers are gone")

        // Settings shows the ones naming a job; one naming none is left to the controller's log.
        guard case .commandFailed(let noJob) = event(#"{"type":"command.failed","command":"engine.route","message":"x"}"#)
        else { return XCTFail("not a command.failed") }
        XCTAssertNil(CatalogueModel.routeJob(noJob))
        let screens = ScreenModels(send: { _ in }, calendar: FakeCalendar(), apps: WorkspaceApps())
        XCTAssertTrue(screens.handles(failed))
        XCTAssertFalse(screens.handles(noJob))
    }
}

/// The catalogue's downloads: only from a press, one at a time, each row with its own state.
@MainActor
final class ModelDownloadTests: XCTestCase {
    private let qwen = "qwen3-asr-1.7b-q8"
    private let parakeet = "parakeet-tdt-0.6b-v3-coreml"
    private let asked: [CoreCommand] = [.modelsList, .engineRoute(.dictationFinal), .engineRoute(.meetingFinal), .engineRoute(.livePartials)]

    /// The catalogue with Qwen3-ASR and the Mac's Parakeet, installed or not.
    private func listed(qwen qwenInstalled: Bool = false, parakeet parakeetInstalled: Bool = false) -> InkEvent {
        event(#"{"type":"models.listed","models":[{"id":"qwen3-asr-1.7b-q8","licence":"Apache-2.0","size_bytes":2520744288,"installed":\#(qwenInstalled),"jobs":[{"job":"dictation_final","wer":4.59},{"job":"meeting_final","wer":16.08}]},{"id":"parakeet-tdt-0.6b-v3-coreml","licence":"CC-BY-4.0","size_bytes":483105645,"installed":\#(parakeetInstalled),"jobs":[]}]}"#)
    }

    private func progress(_ id: String, next: String? = nil, _ done: Int64, of total: Int64) -> InkEvent {
        event(#"{"type":"model.update_progress","id":"\#(id)","next":"\#(next ?? id)","done_bytes":\#(done),"total_bytes":\#(total)}"#)
    }

    private func finished(_ id: String, ok: Bool = true, message: String? = nil) -> InkEvent {
        let why = message.map { #","message":"\#($0)""# } ?? ""
        return event(#"{"type":"model.update_finished","id":"\#(id)","next":"\#(id)","ok":\#(ok),"no_model_warm":false\#(why)}"#)
    }

    private func installs(_ sent: Sent) -> [CoreCommand] {
        sent.commands.filter { if case .modelInstall = $0 { true } else { false } }
    }

    private func entry(_ catalogue: CatalogueModel, _ id: String) throws -> CatalogueEntry {
        try XCTUnwrap(catalogue.models.first { $0.id == id })
    }

    /// Nothing downloads until the user presses Download: not at launch, not while the first run is
    /// walked through. The press then says what goes, smallest first, and sends one at a time.
    func testNothingIsSentBeforeThePressAndThenOneAtATimeSmallestFirst() throws {
        let sent = Sent()
        let screens = ScreenModels(send: sent.send, calendar: FakeCalendar(), apps: WorkspaceApps())
        screens.apply([
            event(#"{"type":"core.ready","version":"1.0.0","abi":2}"#),
            event(#"{"type":"setting.value","key":"onboarding.done"}"#), listed(),
        ])
        while screens.onboarding.step != .ready { screens.onboarding.next() }
        XCTAssertEqual(installs(sent), [], "nothing sent before the press")
        let catalogue = screens.catalogue
        XCTAssertEqual(catalogue.firstRunModels.map(\.id), [parakeet, qwen], "smallest first")
        XCTAssertEqual(catalogue.firstRunModels.map(\.sizeBytes).reduce(0, +), 3_003_849_933, "the total it states")
        XCTAssertEqual(CatalogueModel.sources(catalogue.firstRunModels), "huggingface.co")

        catalogue.downloadFirstRunModels()
        XCTAssertEqual(installs(sent), [.modelInstall(parakeet, ref: "model.update:1")], "one at a time")
        XCTAssertEqual(catalogue.download(of: try entry(catalogue, parakeet)), .downloading(nil))
        XCTAssertEqual(catalogue.download(of: try entry(catalogue, qwen)), .waiting)
        catalogue.downloadFirstRunModels()
        XCTAssertEqual(installs(sent).count, 1, "a second press queues nothing twice")
    }

    /// Each install waits for the one before to end; each end asks again what the catalogue holds
    /// and what serves each job, and a finished model stays listed in the first run, installed.
    func testTheNextInstallWaitsForTheOneBeforeAndEachEndAsksAgain() throws {
        let sent = Sent()
        let catalogue = CatalogueModel(send: sent.send)
        catalogue.apply(listed())
        catalogue.download([parakeet, qwen])
        catalogue.apply(progress(parakeet, 120_000_000, of: 483_105_645))
        XCTAssertEqual(
            catalogue.download(of: try entry(catalogue, parakeet)),
            .downloading(.init(done: 120_000_000, total: 483_105_645)))
        sent.commands = []
        catalogue.apply(progress(parakeet, 483_105_645, of: 483_105_645))
        catalogue.apply(finished(parakeet))
        XCTAssertEqual(sent.commands, asked + [.modelInstall(qwen, ref: "model.update:2")])
        catalogue.apply(listed(parakeet: true))
        XCTAssertEqual(catalogue.download(of: try entry(catalogue, parakeet)), .installed)
        XCTAssertEqual(catalogue.firstRunModels.map(\.id), [parakeet, qwen], "still listed, as downloaded")
        sent.commands = []
        catalogue.apply(finished(qwen))
        XCTAssertEqual(sent.commands, asked + [.modelWarm(.dictationFinal)], "nothing left to install; the dictation model kept warm")
    }

    /// A model that does dictation, once this screen installed it, is kept warm as the one at launch
    /// is (the first take after the download is not a cold load), and the warm goes before the next
    /// install, so it never waits for that download. Parakeet (the shell runs it: it does no job in
    /// the catalogue) and a failed download are not warmed.
    func testAnInstalledDictationModelIsKeptWarmBeforeTheNextInstall() throws {
        let sent = Sent()
        let catalogue = CatalogueModel(send: sent.send)
        catalogue.apply(listed())
        catalogue.download([qwen, parakeet])
        sent.commands = []
        catalogue.apply(finished(qwen))
        XCTAssertEqual(sent.commands, asked + [.modelWarm(.dictationFinal), .modelInstall(parakeet, ref: "model.update:2")])
        sent.commands = []
        catalogue.apply(finished(parakeet))
        XCTAssertEqual(sent.commands, asked, "Parakeet is not warmed: the shell loads it")
        catalogue.download([qwen])
        sent.commands = []
        catalogue.apply(finished(qwen, ok: false, message: "the connection was reset"))
        XCTAssertEqual(sent.commands, asked, "a failed download is not warmed")
    }

    /// Only the model being installed moves its bar: progress for another model, or for a
    /// replacement (another model updated to it), and another model's end, change nothing.
    func testProgressAndEndsForAnotherModelAreIgnored() throws {
        let sent = Sent()
        let catalogue = CatalogueModel(send: sent.send)
        catalogue.apply(listed())
        catalogue.download([parakeet, qwen])
        catalogue.apply(progress(qwen, 1_000, of: 2_520_744_288))
        catalogue.apply(progress("some-older-model", next: parakeet, 5_000, of: 483_105_645))
        XCTAssertEqual(catalogue.download(of: try entry(catalogue, parakeet)), .downloading(nil), "not its progress")
        XCTAssertEqual(catalogue.download(of: try entry(catalogue, qwen)), .waiting)
        catalogue.apply(finished(qwen))
        XCTAssertEqual(catalogue.installing, parakeet, "another model's end does not end this one")
        XCTAssertEqual(installs(sent).count, 1)
    }

    /// A failed download is shown in the core's words, and stays failed until Retry: the next one
    /// goes on, and nothing is tried again on its own.
    func testAFailedDownloadIsShownUntilRetryAndTheNextGoesOn() throws {
        let sent = Sent()
        let catalogue = CatalogueModel(send: sent.send)
        catalogue.apply(listed())
        catalogue.download([parakeet, qwen])
        catalogue.apply(finished(parakeet, ok: false, message: "the new files could not be installed: the connection was reset"))
        catalogue.apply(listed())
        XCTAssertEqual(
            catalogue.download(of: try entry(catalogue, parakeet)),
            .failed("the new files could not be installed: the connection was reset"))
        XCTAssertEqual(installs(sent), [.modelInstall(parakeet, ref: "model.update:1"), .modelInstall(qwen, ref: "model.update:2")])
        catalogue.apply(finished(qwen))
        XCTAssertEqual(installs(sent).count, 2, "never retried on its own")

        catalogue.download([parakeet])  // Retry
        XCTAssertEqual(installs(sent).last, .modelInstall(parakeet, ref: "model.update:3"))
        XCTAssertEqual(catalogue.download(of: try entry(catalogue, parakeet)), .downloading(nil), "the failure goes with the retry")
    }

    /// A download the core refused before it started (another update held the model) fails by its
    /// command's id; a refusal carrying any other id is not this one's.
    func testARefusedInstallIsMatchedByItsIDAndAStaleOneIsNot() throws {
        let sent = Sent()
        let catalogue = CatalogueModel(send: sent.send)
        catalogue.apply(listed())
        catalogue.download([qwen])
        catalogue.apply(event(#"{"type":"command.failed","command":"model.update","id":"model.update:7","message":"stale"}"#))
        XCTAssertEqual(catalogue.installing, qwen)
        catalogue.apply(event(#"{"type":"command.failed","command":"model.update","id":"model.update:1","message":"another update holds qwen3-asr-1.7b-q8"}"#))
        XCTAssertNil(catalogue.installing)
        XCTAssertEqual(catalogue.download(of: try entry(catalogue, qwen)), .failed("another update holds qwen3-asr-1.7b-q8"))
    }

    /// The first run's "the downloads keep going" holds only while one runs or waits: not before
    /// the press, and not once every one has ended, failed or done.
    func testDownloadingHoldsOnlyWhileADownloadRunsOrWaits() {
        let catalogue = CatalogueModel(send: { _ in })
        catalogue.apply(listed())
        XCTAssertFalse(catalogue.downloading, "nothing pressed")
        catalogue.download([parakeet, qwen])
        XCTAssertTrue(catalogue.downloading)
        catalogue.apply(finished(parakeet, ok: false, message: "offline"))
        XCTAssertTrue(catalogue.downloading, "Qwen3-ASR goes next")
        catalogue.apply(finished(qwen, ok: false, message: "offline"))
        XCTAssertFalse(catalogue.downloading, "every download failed: none keeps going")
        catalogue.download([qwen])
        catalogue.apply(finished(qwen))
        XCTAssertFalse(catalogue.downloading, "all done")
    }

    /// The core stopping ends what was queued: nothing is sent after it, and no row stays
    /// "downloading".
    func testTheCoreStoppingClearsTheQueue() throws {
        let sent = Sent()
        let catalogue = CatalogueModel(send: sent.send)
        catalogue.apply(listed())
        catalogue.download([parakeet, qwen])
        catalogue.apply(event(#"{"type":"core.stopped"}"#))
        XCTAssertNil(catalogue.installing)
        XCTAssertEqual(catalogue.download(of: try entry(catalogue, qwen)), .notInstalled)
        catalogue.apply(finished(parakeet))
        XCTAssertEqual(installs(sent).count, 1)
    }

    func testEveryDownloadableModelIsNamedWithWhereItComesFrom() {
        for id in ["qwen3-asr-1.7b-q8", "parakeet-tdt-0.6b-v3-coreml", "nemotron-3-diarization-q8", "silero-vad-v6-16k"] {
            XCTAssertNotEqual(CatalogueModel.name(id), id, "named: \(id)")
            XCTAssertNotNil(CatalogueModel.source(id), id)
        }
        XCTAssertEqual(CatalogueModel.source("silero-vad-v6-16k"), "raw.githubusercontent.com", "the host its row names")
        XCTAssertNil(CatalogueModel.source("a-model-this-build-does-not-know"), "never a host it cannot vouch for")
        let catalogue = CatalogueModel(send: { _ in })
        catalogue.apply(event(#"{"type":"models.listed","models":[{"id":"silero-vad-v6-16k","licence":"MIT","size_bytes":1289603,"installed":false,"jobs":[]},{"id":"qwen3-asr-1.7b-q8","licence":"Apache-2.0","size_bytes":2520744288,"installed":false,"jobs":[]}]}"#))
        XCTAssertEqual(CatalogueModel.sources(catalogue.firstRunModels), "huggingface.co and raw.githubusercontent.com", "most bytes first")
    }

    /// Where the screens say each model comes from is the host its row's URLs name in the core's
    /// registry (ink-engines), the host the download connects to: a firewall rule written from
    /// that sentence lets the download through.
    func testEachModelsSourceIsTheHostItsRowNamesInTheCore() throws {
        let engines = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .deletingLastPathComponent().appendingPathComponent("core/crates/ink-engines/src")
        let registry = try String(contentsOf: engines.appendingPathComponent("registry.rs"), encoding: .utf8)
        let rows = try String(contentsOf: engines.appendingPathComponent("rows.rs"), encoding: .utf8)
        for (id, source, row) in [
            ("qwen3-asr-1.7b-q8", registry, "fn qwen3_asr_1_7b_q8()"),
            ("parakeet-tdt-0.6b-v3-coreml", rows, "pub fn parakeet_tdt_v3_coreml()"),
            ("nemotron-3-diarization-q8", rows, "pub fn nemotron_3_diarization()"),
            ("silero-vad-v6-16k", rows, "pub fn silero_vad()"),
        ] {
            let start = try XCTUnwrap(source.range(of: row), "\(row) is not in the core's source")
            let url = try XCTUnwrap(source[start.upperBound...].firstMatch(of: /"https:\/\/([^\/"]+)\//), "no URL in \(row)")
            XCTAssertEqual(CatalogueModel.source(id), String(url.output.1), id)
        }
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

    func testAskingSaysPlainlyWhenNothingCanAnswer() {
        let live = LiveModel(send: { _ in })
        meeting(live)
        live.askText = "  What do I owe so far? "
        live.submitAsk(context: [])
        XCTAssertEqual(live.askText, "")
        XCTAssertEqual(live.asked.first?.question, "What do I owe so far?")
        XCTAssertNil(live.asked.first?.answer, "waiting for the core")
        // The core has no model to answer with: said in words, never a made-up answer.
        live.apply(event(#"{"type":"command.failed","command":"meeting.ask","id":"ask:0","message":"no language model is available to answer on this Mac"}"#))
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

    /// The speech models come right after the permissions, before polish.
    func testTheModelsStepFollowsThePermissions() {
        typealias Step = OnboardingModel.Step
        XCTAssertEqual(Step.models.rawValue, Step.permissions.rawValue + 1)
        XCTAssertLessThan(Step.models.rawValue, Step.polish.rawValue)
        // Continue leaves it whatever the downloads are doing: the step holds nothing back.
        let onboarding = OnboardingModel(send: { _ in })
        onboarding.step = .models
        onboarding.next()
        XCTAssertGreaterThan(onboarding.step.rawValue, Step.models.rawValue)
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
        let install = try fields(.modelInstall("qwen3-asr-1.7b-q8", ref: "model.update:1"))
        XCTAssertEqual(install["cmd"] as? String, "model.update")
        XCTAssertEqual(install["model"] as? String, "qwen3-asr-1.7b-q8")
        XCTAssertEqual(install["next"] as? String, "qwen3-asr-1.7b-q8", "its own next: the first download")
        XCTAssertEqual(install["id"] as? String, "model.update:1")
        XCTAssertEqual(CoreCommand.modelInstall("x", ref: "r").name, "model.update")
        let warm = try fields(.modelWarm(.dictationFinal))
        XCTAssertEqual(warm["cmd"] as? String, "model.warm")
        XCTAssertEqual(warm["job"] as? String, "dictation_final")
        XCTAssertEqual(warm["id"] as? String, "model.warm:dictation_final")
        XCTAssertEqual(CoreCommand.modelWarm(.dictationFinal).name, "model.warm")
        XCTAssertEqual(CoreCommand.noteAdd(record: "r", atMs: 1, text: "private words", ref: "x").name, "note.add",
                       "the name logged never carries the words")
    }

    /// A command that never reached the core fails as the core would have failed it: its name and
    /// id, and a message that never carries its fields.
    func testACommandThatNeverReachedTheCoreFailsWithItsNameAndId() {
        let ask = CoreCommand.meetingAsk(question: "a private question", ref: "ask:3")
        guard case .commandFailed(let failed) = ask.notSent("couldn't send it: the core is not running") else {
            return XCTFail("not a command.failed")
        }
        XCTAssertEqual(failed.type, "command.failed")
        XCTAssertEqual(failed.command, "meeting.ask")
        XCTAssertEqual(failed.id, "ask:3")
        XCTAssertEqual(failed.message, "couldn't send it: the core is not running")
        XCTAssertFalse(failed.message.contains("private"))
        guard case .commandFailed(let noID) = CoreCommand.modesList.notSent("x") else {
            return XCTFail("not a command.failed")
        }
        XCTAssertNil(noID.id)
        XCTAssertEqual(CoreCommand.engineRoute(.livePartials).commandID, "engine.route:live_partials")
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
        // The legacy 0.2 app's base is not in the Mac app, and wasapi-rs's pattern is in Windows-only
        // code, as are sherpa-onnx, ONNX Runtime and the code compiled into sherpa-onnx's library
        // (Windows' Parakeet).
        let windowsOnly: Set = [
            "wasapi-rs", "sherpa-onnx", "ONNX Runtime",
            "nlohmann/json", "kaldi-decoder", "kaldifst", "OpenFst", "simple-sentencepiece",
            "kaldi-native-fbank", "hclust-cpp",
        ]
        let shipped = names.filter { $0 != "Handy" && !windowsOnly.contains($0) }
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

/// The notices composed from a licence's standard text (their upstream file was not on hand) say
/// so in Notices.swift (`composed: true`), and mac/composed-notices.txt lists exactly those, each
/// with its upstream-check marker, which a release tag waits for (mac/scripts/notices-verified.sh).
final class ComposedNoticesTests: XCTestCase {
    /// The list's lines, by notice id: `<id> verified=<no|YYYY-MM-DD> <what to compare with>`.
    private func markers(_ list: String) -> [String: String] {
        var listed: [String: String] = [:]
        for line in list.split(separator: "\n") where !line.hasPrefix("#") && !line.trimmingCharacters(in: .whitespaces).isEmpty {
            let words = line.split(separator: " ", omittingEmptySubsequences: true)
            guard words.count >= 3 else { continue }
            listed[String(words[0])] = String(words[1])
        }
        return listed
    }

    /// What is wrong between the composed notices and the list: one declared without a line, one
    /// listed that is not declared, and a marker that is neither `verified=no` nor a date.
    private func problems(declared: Set<String>, list: String) -> [String] {
        let listed = markers(list)
        var out = declared.subtracting(listed.keys).sorted().map { "\($0) is composed but has no line (no verified= marker)" }
        out += Set(listed.keys).subtracting(declared).sorted().map { "\($0) is listed but not composed in Notices.swift" }
        out += listed.sorted { $0.key < $1.key }.compactMap { id, marker in
            marker == "verified=no" || marker.wholeMatch(of: /verified=\d{4}-\d{2}-\d{2}/) != nil ? nil : "\(id): \(marker)"
        }
        return out
    }

    func testEveryComposedNoticeIsListedWithItsUpstreamCheckAndNothingElseIs() throws {
        let root = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        let list = try String(contentsOf: root.appendingPathComponent("mac/composed-notices.txt"), encoding: .utf8)
        let declared = Notices.composedIDs
        XCTAssertFalse(declared.isEmpty, "Notices.swift declares its composed notices")
        XCTAssertEqual(problems(declared: declared, list: list), [])
        // Every line is well formed (a short line would drop out of the comparison above).
        for line in list.split(separator: "\n") where !line.hasPrefix("#") && !line.trimmingCharacters(in: .whitespaces).isEmpty {
            XCTAssertGreaterThanOrEqual(line.split(separator: " ").count, 3, "\(line)")
        }
    }

    /// The check fails when a composed notice has no line, and when a line names no composed notice.
    func testACheckThatCatchesAComposedNoticeWithoutItsMarker() {
        let list = "# header\nprotobuf-lite  verified=no  its file\nsilero-vad verified=2026-10-04 its file\n"
        XCTAssertEqual(problems(declared: ["protobuf-lite", "silero-vad"], list: list), [])
        XCTAssertEqual(
            problems(declared: ["protobuf-lite", "silero-vad", "new-one"], list: list),
            ["new-one is composed but has no line (no verified= marker)"])
        XCTAssertEqual(
            problems(declared: ["protobuf-lite"], list: list), ["silero-vad is listed but not composed in Notices.swift"])
        XCTAssertEqual(problems(declared: ["x"], list: "x verified=soon its file"), ["x: verified=soon"])
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
        let polish = try answer(.settingSet(.dictationPolish, "off")) { if case .settingValue(let v) = $0 { v } else { nil } }
        XCTAssertEqual(polish?.value, "off")
        let state = try answer(.consentGet(.polish, ref: "consent.get:polish:1")) { if case .consentState(let v) = $0 { v } else { nil } }
        XCTAssertEqual(state?.on, false)
        XCTAssertEqual(state?.allowed, false)
        // No language model here, so there is nothing to agree to: refused, matched by its id.
        let allow = try answer(.consentAllow(feature: .polish, to: .onDevice, endpoint: nil, key: nil, ref: "allow-1")) { if case .commandFailed(let f) = $0 { f } else { nil } }
        XCTAssertEqual(allow?.id, "allow-1")
        XCTAssertThrowsError(try session.command(#"{"cmd":"setting.set","key":"dictation.polish","value":"on"}"#),
                             "only the consent step turns polish on")
        let onboarding = try answer(.settingGet(.onboardingDone)) { if case .settingValue(let v) = $0 { v } else { nil } }
        XCTAssertEqual(onboarding?.key, "onboarding.done")
        XCTAssertNil(onboarding?.value, "a fresh library has not onboarded")
        let modes = try answer(.modesList) { if case .modesListed(let m) = $0 { m } else { nil } }
        XCTAssertEqual(modes?.modes.map(\.name), ["Default"])
        let owed = try answer(.commitmentsList) { if case .commitmentsListed(let c) = $0 { c } else { nil } }
        XCTAssertEqual(owed?.items, [])
        let catalogue = try XCTUnwrap(try answer(.modelsList) { if case .modelsListed(let m) = $0 { m } else { nil } })
        XCTAssertTrue(catalogue.models.contains { $0.id == ParakeetFiles.rowID && $0.jobs.isEmpty }, "the Mac's Parakeet, only downloaded")
        MainActor.assumeIsolated {
            for model in catalogue.models {
                XCTAssertNotEqual(CatalogueModel.name(model.id), model.id, "named: \(model.id)")
                XCTAssertNotNil(CatalogueModel.source(model.id), "where \(model.id) comes from")
            }
        }
        // An id the registry does not hold: refused before anything is fetched, matched by its id.
        // (Never a real model here: that would download it.)
        let install = try answer(.modelInstall("no-such-model", ref: "model.update:1")) { if case .commandFailed(let f) = $0 { f } else { nil } }
        XCTAssertEqual(install?.command, "model.update")
        XCTAssertEqual(install?.id, "model.update:1")
        XCTAssertNotNil(try answer(.engineRoute(.dictationFinal)) { if case .engineRouted(let r) = $0 { r } else { nil } })
        // Nothing installed here: nothing to keep warm, and it says so.
        let warm = try answer(.modelWarm(.dictationFinal)) { if case .modelWarmFailed(let f) = $0 { f } else { nil } }
        XCTAssertEqual(warm?.job, .dictationFinal)
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
        let hosting = NSHostingController(rootView: LiveMeetingView(
            meeting: meeting, live: live, meetings: MeetingModel(send: { _ in })))
        let minimum = hosting.sizeThatFits(in: .zero)
        XCTAssertLessThan(minimum.height, 460, "the window's minimum content height is 460")
        XCTAssertLessThan(minimum.width, 720)
    }

    func testOwedAndSettingsNeverRaiseTheWindowsMinimumSizeEither() {
        let screens = ScreenModels(send: { _ in }, calendar: FakeCalendar(), apps: WorkspaceApps())
        screens.owed.apply(event(#"{"type":"commitments.listed","items":[{"id":"a","record":"r","record_started_at_unix_ms":0,"text":"A promise long enough to wrap onto more than one line when the window is narrow","merged":1}]}"#))
        // Models with a download under way, one waiting, and one failed with the core's words.
        screens.catalogue.apply(event(#"{"type":"models.listed","models":[{"id":"qwen3-asr-1.7b-q8","licence":"Apache-2.0","size_bytes":2520744288,"installed":false,"jobs":[]},{"id":"parakeet-tdt-0.6b-v3-coreml","licence":"CC-BY-4.0","size_bytes":483105645,"installed":false,"jobs":[]},{"id":"silero-vad-v6-16k","licence":"MIT","size_bytes":1289603,"installed":false,"jobs":[]}]}"#))
        screens.catalogue.download(["silero-vad-v6-16k", "parakeet-tdt-0.6b-v3-coreml", "qwen3-asr-1.7b-q8"])
        screens.catalogue.apply(event(#"{"type":"model.update_finished","id":"silero-vad-v6-16k","next":"silero-vad-v6-16k","ok":false,"no_model_warm":false,"message":"the new files could not be installed: downloading silero_vad_16k_op15.onnx: the connection was reset by the server before the file was complete"}"#))
        screens.catalogue.apply(event(#"{"type":"model.update_progress","id":"parakeet-tdt-0.6b-v3-coreml","next":"parakeet-tdt-0.6b-v3-coreml","done_bytes":120000000,"total_bytes":483105645}"#))
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
            event(#"{"type":"consent.state","feature":"polish","on":true,"allowed":true,"to":"on_device","name":"SystemLanguageModel.default","allowed_to":"on_device"}"#),
        ])
        for _ in 0..<PolishModel.timeoutWarning {
            screens.apply([
                event(#"{"type":"dictation.started","take":0,"edit":false}"#), timedOut,
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
        // Dictation's switch is read and answered at launch (dictation.ready or .off); a quit
        // before that answer would log the enable it then sends as not sent.
        try await until { core.screens.dictation.state != .starting }
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
        core.received([event(#"{"type":"command.failed","command":"setting.set","id":"setting:onboarding.done","message":"the library could not be written: zebra"}"#)])
        XCTAssertEqual(logged.messages.count, 1)
        XCTAssertTrue(logged.messages[0].contains("setting.set"), logged.messages[0])
        XCTAssertFalse(logged.messages[0].contains("zebra"), "never the core's message or a field")
        XCTAssertFalse(logged.messages[0].contains("onboarding.done"))
        // Handled ones are the screens' to show.
        for handled in ["permissions.check", "models.list", "modes.list", "commitment.set_done", "note.add", "note.update", "note.delete", "model.update"] {
            core.received([event(#"{"type":"command.failed","command":"\#(handled)","message":"x"}"#)])
        }
        // (Owed lists again after a failed set_done; with no core running that is logged as not sent.)
        func failures() -> [String] { logged.messages.filter { $0.hasPrefix("command.failed") } }
        XCTAssertEqual(failures().count, 1)
        // Polish and the dictation keys show their own setting failures (S2.7).
        core.received([event(#"{"type":"command.failed","command":"setting.get","id":"setting:dictation.polish","message":"x"}"#)])
        core.received([event(#"{"type":"command.failed","command":"setting.set","id":"setting:dictation.key","message":"x"}"#)])
        XCTAssertEqual(failures().count, 1)
        core.received([event(#"{"type":"command.failed","command":"setting.get","id":"setting:somebody.else","message":"x"}"#)])
        XCTAssertEqual(failures().count, 2, "a setting no screen reads is logged")
    }

    /// With no core to take them, the commands screens wait on fail one hop later, as if the core
    /// had failed them: the list loading, Ask thinking and a consent read each say they couldn't
    /// instead of waiting forever. Each is logged once as not sent, and no failure goes unshown.
    func testACommandWithNoCoreFailsSoItsScreenStopsWaiting() async throws {
        let logged = Logged()
        let core = CoreController(registersAppleEngines: false, commandLog: logged.log)
        core.library.refreshList()
        core.screens.live.askText = "What did we decide?"
        core.screens.live.submitAsk(context: [])
        core.screens.polish.load()
        XCTAssertEqual(core.library.listLoad, .loading, "after this turn, not inside the send")
        XCTAssertNil(core.screens.live.asked.first?.answer)

        try await until {
            core.library.listLoad == .failed && core.screens.live.asked.first?.answer != nil
                && core.screens.polish.failure != nil
        }
        XCTAssertEqual(core.screens.live.asked.first?.answer, .unavailable("Couldn't answer that. Try asking again."))
        XCTAssertEqual(core.screens.polish.failure, .read)
        XCTAssertEqual(logged.messages.count, 3, "\(logged.messages)")
        XCTAssertTrue(logged.messages.allSatisfy { $0.hasPrefix("no core is running") }, "\(logged.messages)")
        XCTAssertFalse(logged.messages.joined().contains("decide"), "never the question")
    }

    /// A command the core refuses to queue fails the same way, with its id, so its screen hears.
    func testACommandTheCoreRefusesFailsWithItsId() async throws {
        let data = FileManager.default.temporaryDirectory.appendingPathComponent("inkwell-refused-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: data) }
        let core = CoreController(registersAppleEngines: false, commandLog: Logged().log)
        var failures: [CommandFailed] = []
        core.observer = { batch in
            for case .commandFailed(let failed) in batch { failures.append(failed) }
        }
        core.start(environment: ["INK_DATA_DIR": data.path])
        try await until { if case .ready = core.store.status { true } else { false } }

        // A page of no records: the core cannot read it, so it never queues it.
        core.send(.recordsList(kind: nil, before: nil, limit: 0, ref: "list-99"))
        try await until { failures.contains { $0.id == "list-99" } }
        let refused = failures.first { $0.id == "list-99" }
        XCTAssertEqual(refused?.command, "records.list")
        XCTAssertEqual(refused?.message, "couldn't send it: the core could not read the argument")

        await withCheckedContinuation { (done: CheckedContinuation<Void, Never>) in
            core.stop { done.resume() }
        }
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
