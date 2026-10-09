// Dictation in the shell (S2.7): the keys in Settings drive the core, the Drop shows a take's live
// words and the app's mode, and says what became of a take that did not go in as it should; the
// Polish setting says when it could not be read or saved. TCC-bound behaviour (the event tap,
// insertion into other apps) is on mac/DICTATION-CHECKLIST.md.
import AppKit
import Foundation
import InkBridge
import Synchronization
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

@MainActor
private final class Sent {
    var commands: [CoreCommand] = []
    lazy var send: SendCommand = { [unowned self] in self.commands.append($0) }
}

@MainActor
final class DictationModelTests: XCTestCase {
    func testTheCoreIsAskedToHoldTheKeysOnceReadyAndAgainAfterAccessibilityIsGranted() {
        let sent = Sent()
        let dictation = DictationModel(send: sent.send, timeZone: { TimeZone(secondsFromGMT: 7_200)! })
        dictation.enable()
        XCTAssertEqual(sent.commands, [.dictationEnable(utcOffsetMinutes: 120, ref: "dictation:1")])
        dictation.apply(event(#"{"type":"dictation.off","reason":"needs_accessibility","message":"permission not granted: Accessibility","ref":"dictation:1"}"#))
        XCTAssertEqual(dictation.state, .off(.needsAccessibility, message: "permission not granted: Accessibility"))
        XCTAssertTrue(dictation.isProblem)
        XCTAssertTrue(dictation.status.contains("Type for you"), dictation.status)
        // Back from System Settings: asked again.
        dictation.appBecameActive()
        XCTAssertEqual(sent.commands.last, .dictationEnable(utcOffsetMinutes: 120, ref: "dictation:2"))
        dictation.apply(event(#"{"type":"dictation.ready","key":"fn","ref":"dictation:2"}"#))
        XCTAssertEqual(dictation.state, .live(key: "fn", editKey: nil))
        XCTAssertEqual(dictation.status, "Hold fn, speak, let go.")
        XCTAssertFalse(dictation.isProblem)
        // Live: coming back asks nothing.
        let before = sent.commands.count
        dictation.appBecameActive()
        XCTAssertEqual(sent.commands.count, before)
    }

    func testPickingAKeySavesItAndTheCoresAnswerIsWhatShows() {
        let sent = Sent()
        let dictation = DictationModel(send: sent.send)
        dictation.load()
        XCTAssertEqual(sent.commands, [.settingGet(.dictationEnabled), .settingGet(.dictationKey), .settingGet(.dictationEditKey)])
        dictation.apply(event(#"{"type":"setting.value","key":"dictation.key"}"#))
        XCTAssertEqual(dictation.key, "fn", "never set: the core's default")
        XCTAssertNil(dictation.editKey)
        dictation.setKey("right_option")
        XCTAssertEqual(sent.commands.last, .settingSet(.dictationKey, "right_option"))
        dictation.apply(event(#"{"type":"setting.value","key":"dictation.key","value":"right_option"}"#))
        dictation.apply(event(#"{"type":"dictation.ready","key":"right_option"}"#))
        XCTAssertEqual(dictation.key, "right_option")
        XCTAssertEqual(dictation.status, "Hold Right ⌥, speak, let go.")
        dictation.setEditKey("right_command")
        XCTAssertEqual(sent.commands.last, .settingSet(.dictationEditKey, "right_command"))
        dictation.apply(event(#"{"type":"dictation.ready","key":"right_option","edit_key":"right_command"}"#))
        XCTAssertEqual(dictation.editKey, "right_command")
        dictation.setEditKey(nil)
        XCTAssertEqual(sent.commands.last, .settingSet(.dictationEditKey, "off"))
        // A key the core could not hold for the edit: said, and dictation goes on.
        dictation.apply(event(#"{"type":"dictation.ready","key":"right_option","edit_key_error":"the edit key is the dictation key; pick another"}"#))
        XCTAssertEqual(dictation.editKeyProblem, "the edit key is the dictation key; pick another")
        XCTAssertEqual(dictation.state, .live(key: "right_option", editKey: nil))
    }

    /// Off because the core refused the key: the line says to pick another, and picking one turns
    /// dictation on again with it (the core binds nothing until asked).
    func testAKeyTheCoreRefusedIsReplacedByPickingAnother() {
        let sent = Sent()
        let dictation = DictationModel(send: sent.send, timeZone: { TimeZone(secondsFromGMT: 0)! })
        dictation.apply(event(#"{"type":"setting.value","key":"dictation.enabled","value":"on"}"#))
        dictation.apply(event(#"{"type":"dictation.off","reason":"key_refused","message":"Inkwell doesn't know that key","ref":"dictation:1"}"#))
        XCTAssertEqual(dictation.status, "That key can't be used here: Inkwell doesn't know that key. Pick another, or record a shortcut.")
        dictation.setKey("right_option")
        XCTAssertEqual(Array(sent.commands.suffix(2)), [.settingSet(.dictationKey, "right_option"), .dictationEnable(utcOffsetMinutes: 0, ref: "dictation:2")])
        // Live: a new key only rebinds (the core does it on the setting).
        dictation.apply(event(#"{"type":"dictation.ready","key":"right_option"}"#))
        dictation.setKey("right_command")
        XCTAssertEqual(sent.commands.last, .settingSet(.dictationKey, "right_command"))
    }

    /// The edit picker offers every key but the dictation key, so its selection must never be
    /// that key, even when the stored settings collide before the core has answered.
    func testTheEditKeyIsNeverTheDictationKeyEvenBeforeTheCoreAnswers() {
        let dictation = DictationModel(send: { _ in })
        dictation.apply(event(#"{"type":"setting.value","key":"dictation.key","value":"right_option"}"#))
        dictation.apply(event(#"{"type":"setting.value","key":"dictation.edit_key","value":"right_option"}"#))
        XCTAssertEqual(dictation.state, .starting)
        XCTAssertNil(dictation.editKey)
        dictation.apply(event(#"{"type":"setting.value","key":"dictation.edit_key","value":"right_command"}"#))
        XCTAssertEqual(dictation.editKey, "right_command")
    }

    /// Stopped after repeated failures: Settings offers to turn it on again, which asks the core.
    func testDictationThatStoppedCanBeTurnedOnAgain() {
        let sent = Sent()
        let dictation = DictationModel(send: sent.send)
        dictation.apply(event(#"{"type":"dictation.off","reason":"worker_stopped","message":"dictation stopped after repeated failures; turn it on again"}"#))
        XCTAssertTrue(dictation.canRetry)
        dictation.enable()
        XCTAssertTrue(sent.commands.contains { if case .dictationEnable = $0 { true } else { false } })
        dictation.apply(event(#"{"type":"dictation.off","reason":"needs_accessibility"}"#))
        XCTAssertFalse(dictation.canRetry, "Accessibility has its own Allow")
        dictation.apply(event(#"{"type":"dictation.off","reason":"unsupported"}"#))
        XCTAssertFalse(dictation.canRetry)
    }

    /// A key macOS stopped sending is never shown as working: Settings says so until dictation is
    /// enabled again (coming back to the app tries), the Drop says it once, and Today lists it.
    func testALostKeyIsShownAsNotWorkingUntilDictationIsEnabledAgain() {
        let sent = Sent()
        let dictation = DictationModel(send: sent.send)
        dictation.apply(event(#"{"type":"dictation.ready","key":"fn","edit_key":"right_command"}"#))
        XCTAssertNil(dictation.editKeyProblem)
        dictation.apply(event(#"{"type":"dictation.edit_hotkey_lost"}"#))
        XCTAssertEqual(dictation.editKeyProblem, DictationModel.editKeyLostText)
        XCTAssertTrue(dictation.isProblem)
        XCTAssertEqual(dictation.note?.text.title, "The edit key stopped working")
        // Back from System Settings: asked again, and the answer clears it.
        let before = sent.commands.count
        dictation.appBecameActive()
        XCTAssertEqual(sent.commands.count, before + 1)
        dictation.apply(event(#"{"type":"dictation.ready","key":"fn","edit_key":"right_command"}"#))
        XCTAssertNil(dictation.editKeyProblem)
        XCTAssertFalse(dictation.isProblem)

        dictation.apply(event(#"{"type":"dictation.hotkey_lost"}"#))
        XCTAssertEqual(dictation.status, DictationModel.keyLostText)
        XCTAssertTrue(dictation.isProblem)
        XCTAssertEqual(dictation.note?.text.title, "The dictation key stopped working")
        dictation.apply(event(#"{"type":"dictation.ready","key":"fn"}"#))
        XCTAssertEqual(dictation.status, "Hold fn, speak, let go.")

        let store = CoreStore()
        store.apply([event(#"{"type":"dictation.edit_hotkey_lost"}"#)])
        let kinds = store.notices.map(\.kind)
        XCTAssertEqual(kinds, [.editKeyLost])
        XCTAssertEqual(NeedsYou.describe(.editKeyLost)?.0, "The edit key stopped working")
    }

    /// A dictation command the core could not even queue or run reads as such, with a way to try
    /// again: never "Starting…" forever.
    func testADictationCommandThatFailedIsShownWithARetry() {
        let sent = Sent()
        let dictation = DictationModel(send: sent.send)
        dictation.enable()
        let failed = event(#"{"type":"command.failed","command":"dictation.enable","id":"dictation:1","message":"a bug in the core stopped this command"}"#)
        dictation.apply(failed)
        XCTAssertEqual(dictation.status, "Dictation couldn't start.")
        XCTAssertTrue(dictation.isProblem)
        XCTAssertTrue(dictation.canRetry)
        guard case .commandFailed(let f) = failed else { return XCTFail() }
        XCTAssertTrue(dictation.handles(f))
        dictation.retry()
        XCTAssertEqual(sent.commands.last, .dictationEnable(utcOffsetMinutes: TimeZone.current.secondsFromGMT() / 60, ref: "dictation:2"))
        dictation.apply(event(#"{"type":"dictation.ready","key":"fn","ref":"dictation:2"}"#))
        XCTAssertFalse(dictation.isProblem)

        // Turning it off failed: it still reads as on, says so, and retrying turns it off again.
        dictation.setOn(false)
        XCTAssertEqual(sent.commands.suffix(2), [.settingSet(.dictationEnabled, "off"), .dictationDisable(ref: "dictation:3")])
        dictation.apply(event(#"{"type":"command.failed","command":"dictation.disable","id":"dictation:3","message":"the queries thread has stopped"}"#))
        XCTAssertEqual(dictation.status, "Dictation couldn't be turned off.")
        XCTAssertTrue(dictation.isProblem)
        dictation.retry()
        XCTAssertEqual(sent.commands.last, .dictationDisable(ref: "dictation:4"))
        dictation.apply(event(#"{"type":"dictation.off","reason":"disabled","ref":"dictation:4"}"#))
        XCTAssertEqual(dictation.status, "Dictation is off.")
        XCTAssertFalse(dictation.isProblem)
    }

    /// The user's latest action speaks first: with the key already lost, turning dictation off
    /// and failing reads as that failure, not as the older lost key.
    func testAFailedTurnOffReadsAsSuchEvenWithTheKeyLost() {
        let sent = Sent()
        let dictation = DictationModel(send: sent.send)
        dictation.apply(event(#"{"type":"dictation.ready","key":"fn"}"#))
        dictation.apply(event(#"{"type":"dictation.hotkey_lost"}"#))
        XCTAssertEqual(dictation.status, DictationModel.keyLostText)
        dictation.setOn(false)
        guard case .dictationDisable(let ref) = sent.commands.last else { return XCTFail() }
        dictation.apply(event(#"{"type":"command.failed","command":"dictation.disable","id":"\#(ref)","message":"the queries thread has stopped"}"#))
        XCTAssertEqual(dictation.status, "Dictation couldn't be turned off.")
        XCTAssertEqual(dictation.retryTitle, "Try again")
    }

    /// Settings > Voice turns dictation off and on; the choice is kept, and a launch with it off
    /// holds no key.
    func testDictationCanBeTurnedOffAndStaysOffAtTheNextLaunch() {
        let sent = Sent()
        let dictation = DictationModel(send: sent.send)
        dictation.load()
        XCTAssertTrue(sent.commands.contains(.settingGet(.dictationEnabled)))
        XCTAssertFalse(sent.commands.contains { if case .dictationEnable = $0 { true } else { false } },
                       "not before the switch is read")
        dictation.apply(event(#"{"type":"setting.value","key":"dictation.enabled","value":"off"}"#))
        XCTAssertFalse(sent.commands.contains { if case .dictationEnable = $0 { true } else { false } })
        XCTAssertFalse(dictation.isOn)
        XCTAssertEqual(dictation.status, "Dictation is off.")
        dictation.setOn(true)
        XCTAssertEqual(sent.commands.suffix(2).first, .settingSet(.dictationEnabled, "on"))
        XCTAssertTrue(sent.commands.last.map { if case .dictationEnable = $0 { true } else { false } } ?? false)
        XCTAssertTrue(dictation.isOn)

        // Never set: on, as a fresh install is.
        let fresh = Sent()
        let first = DictationModel(send: fresh.send)
        first.load()
        first.apply(event(#"{"type":"setting.value","key":"dictation.enabled"}"#))
        XCTAssertTrue(fresh.commands.contains { if case .dictationEnable = $0 { true } else { false } })
        // A switch that cannot be read: dictation starts (the default), and Settings says so.
        let unread = Sent()
        let third = DictationModel(send: unread.send)
        third.load()
        third.apply(event(#"{"type":"command.failed","command":"setting.get","id":"setting:dictation.enabled","message":"x"}"#))
        XCTAssertTrue(unread.commands.contains { if case .dictationEnable = $0 { true } else { false } })
        XCTAssertEqual(third.keyFailure, "Couldn't read whether dictation is on, so it is on.")
        // Coming back to the app never turns on a dictation the user turned off.
        let before = sent.commands.count
        dictation.setOn(false)
        dictation.apply(event(#"{"type":"dictation.off","reason":"disabled"}"#))
        dictation.appBecameActive()
        XCTAssertEqual(sent.commands.count, before + 2)
    }

    /// A key setting that could not be saved or read reads "couldn't", never as the key changed.
    func testAKeySettingThatCouldNotBeSavedSaysSo() {
        let dictation = DictationModel(send: { _ in })
        let failed = event(#"{"type":"command.failed","command":"setting.set","id":"setting:dictation.key","message":"the library could not be written"}"#)
        dictation.apply(failed)
        XCTAssertEqual(dictation.keyFailure, "Couldn't save the key. The one before still works.")
        XCTAssertTrue(dictation.isProblem)
        guard case .commandFailed(let f) = failed else { return XCTFail() }
        XCTAssertTrue(dictation.handles(f))
        dictation.apply(event(#"{"type":"command.failed","command":"setting.get","id":"setting:dictation.edit_key","message":"x"}"#))
        XCTAssertEqual(dictation.keyFailure, "Couldn't read your key settings.")
        dictation.setKey("fn")
        XCTAssertNil(dictation.keyFailure, "a new try clears it")
    }

    /// The plan's words for a take too short to hear: "Too short, try again", never "No speech".
    func testTheDropSaysWhatBecameOfATakeThatDidNotGoIn() {
        func note(_ json: String, model: Bool = true) -> DropText? {
            DictationModel.note(for: event(json), hasLanguageModel: model)
        }
        XCTAssertEqual(note(#"{"type":"dictation.discarded","reason":"speech_too_short"}"#), DropText(title: "Too short", detail: "Try again"))
        XCTAssertEqual(note(#"{"type":"dictation.discarded","reason":"too_short","live_ms":250}"#)?.title, "Too short")
        XCTAssertEqual(note(#"{"type":"dictation.discarded","reason":"silence"}"#)?.tone, .alert)
        XCTAssertNil(note(#"{"type":"dictation.discarded","reason":"cancelled"}"#))
        XCTAssertNil(note(#"{"type":"dictation.inserted","text":"hi","outcome":"pasted"}"#), "it went in: nothing to say")
        XCTAssertEqual(note(#"{"type":"dictation.inserted","text":"hi","outcome":"blocked"}"#)?.title, "Secure input is on")
        XCTAssertEqual(note(#"{"type":"dictation.edit_failed","reason":"no_selection"}"#)?.title, "Select some text first")
        XCTAssertEqual(note(#"{"type":"dictation.edit_failed","reason":"secure_input"}"#),
                       DropText(title: "Secure input is on", detail: "The selection was left alone", tone: .alert))
        XCTAssertEqual(note(#"{"type":"dictation.edit_failed","reason":"model","message":"no language model is registered for polish"}"#, model: false)?.title,
                       "Editing needs Apple Intelligence")
        XCTAssertEqual(note(#"{"type":"dictation.edit_failed","reason":"model","message":"x"}"#, model: true)?.title,
                       "Couldn't rewrite the selection")
        XCTAssertEqual(note(#"{"type":"dictation.edit_failed","reason":"timed_out"}"#)?.detail, "The selection was left alone")
        XCTAssertEqual(note(#"{"type":"dictation.mic_failed","message":"permission not granted: Microphone"}"#)?.title, "Couldn't open the microphone")
        // Polish that ran out of its budget: the take went in as said, and the Drop says so.
        XCTAssertEqual(note(#"{"type":"dictation.warning","kind":"polish_timed_out"}"#), DropText(title: "Polish took too long", detail: "Typed as you said it"))
        XCTAssertEqual(note(#"{"type":"dictation.warning","kind":"release_missed"}"#)?.title, "Stopped after 3 minutes")
        // Polish on, but its model now sends somewhere the user has not agreed to.
        XCTAssertEqual(note(#"{"type":"dictation.warning","kind":"polish_not_allowed","message":"Example Cloud"}"#)?.title, "Not polished")
        // The mode names a model of its own the core does not hold, or that sends elsewhere now:
        // nothing was sent, and Settings > Modes says which (it was quiet before).
        XCTAssertEqual(note(#"{"type":"dictation.warning","kind":"polish_model_missing","message":"the mode's model is not held"}"#),
                       DropText(title: "Not polished", detail: "Check this mode's model in Settings > Modes", tone: .alert))
        XCTAssertNil(note(#"{"type":"dictation.warning","kind":"tail_cut_short"}"#))
        // Every note is words the shell wrote: never the user's.
        let dictation = DictationModel(send: { _ in })
        dictation.apply(event(#"{"type":"dictation.discarded","reason":"speech_too_short"}"#))
        dictation.apply(event(#"{"type":"dictation.discarded","reason":"speech_too_short"}"#))
        XCTAssertEqual(dictation.note?.serial, 2, "the same words twice show twice")
    }
}

@MainActor
final class PolishSettingFailureTests: XCTestCase {
    /// §6 carried item: a failed polish read or write gets UI, not only the log.
    func testAPolishSettingThatCouldNotBeReadOrSavedSaysSoAndTheToggleGoesBack() {
        let polish = PolishModel(send: { _ in })
        polish.apply(event(#"{"type":"engine.registered","id":"apple-foundation-models","kind":"llm","jobs":[]}"#))
        polish.apply(event(#"{"type":"command.failed","command":"consent.get","id":"consent.get:polish:1","message":"the library could not be read"}"#))
        XCTAssertEqual(polish.failure, .read)
        XCTAssertEqual(polish.status, "Couldn't read your polish setting. Open Settings again to retry.")
        XCTAssertFalse(polish.canToggle, "unknown, so not switchable")
        XCTAssertTrue(polish.isProblem)
        polish.apply(event(#"{"type":"consent.state","feature":"polish","on":true,"allowed":true,"to":"on_device","name":"SystemLanguageModel.default","allowed_to":"on_device"}"#))
        XCTAssertNil(polish.failure)
        XCTAssertTrue(polish.isOn)
        polish.setOn(false)
        XCTAssertFalse(polish.isOn)
        polish.apply(event(#"{"type":"command.failed","command":"setting.set","id":"setting:dictation.polish","message":"the library could not be written"}"#))
        XCTAssertEqual(polish.failure, .write)
        XCTAssertTrue(polish.isOn, "back where it was: the change was not saved")
        XCTAssertEqual(polish.status, "Couldn't save the change, so polish stays as it was.")
        // The screens show it, so the controller does not log it as unshown.
        let screens = ScreenModels(send: { _ in })
        guard case .commandFailed(let failed) = event(#"{"type":"command.failed","command":"setting.set","id":"setting:dictation.polish","message":"x"}"#) else {
            return XCTFail()
        }
        XCTAssertTrue(screens.handles(failed))
    }
}

@MainActor
final class DictationDropTests: XCTestCase {
    func testTheDropNamesTheAppAndItsModeAndShowsTheLiveWordsOfThisTakeOnly() {
        let store = CoreStore()
        let ink = ShellInk(store: store)
        store.apply([event(#"{"type":"dictation.started","take":3,"edit":false,"mode":"Chat","app":"Example Chat"}"#)])
        XCTAssertEqual(ink.dropText, DropText(title: "Dictating · Example Chat · Chat", detail: "Listening"))
        store.apply([event(#"{"type":"dictation.partial","take":3,"text":"send the deck over before friday and"}"#)])
        XCTAssertEqual(ink.dropText.detail, "send the deck over before friday and")
        XCTAssertTrue(ink.dropText.liveWords)
        // A late partial of an earlier take is never shown.
        store.apply([event(#"{"type":"dictation.partial","take":2,"text":"an older take"}"#)])
        XCTAssertEqual(ink.dropText.detail, "send the deck over before friday and")
        store.apply([event(#"{"type":"dictation.stopped"}"#)])
        XCTAssertEqual(ink.dropText.detail, "Transcribing")
        XCTAssertFalse(ink.dropText.liveWords)
        store.apply([event(#"{"type":"dictation.partial","take":3,"text":"after the stop"}"#)])
        XCTAssertEqual(ink.dropText.detail, "Transcribing", "none shown after the stop")
        store.apply([event(#"{"type":"dictation.inserted","text":"x","outcome":"pasted"}"#)])
        XCTAssertNil(store.liveDictation)
    }

    func testAnEditSaysSoAndAVoiceCommandEndsItsTake() {
        let store = CoreStore()
        let ink = ShellInk(store: store)
        store.apply([event(#"{"type":"dictation.started","take":0,"edit":true}"#)])
        XCTAssertEqual(ink.dropText, DropText(title: "Editing the selection", detail: "Say what to change"))
        store.apply([event(#"{"type":"dictation.stopped"}"#)])
        XCTAssertEqual(ink.dropText.detail, "Rewriting")
        store.apply([event(#"{"type":"dictation.edited","outcome":"pasted"}"#)])
        XCTAssertEqual(ink.state, .idle)
        store.apply([event(#"{"type":"dictation.started","take":1,"edit":false}"#), event(#"{"type":"dictation.stopped"}"#)])
        store.apply([event(#"{"type":"dictation.command","action":"toggle_polish","risk":"safe"}"#)])
        XCTAssertEqual(ink.state, .idle, "a voice command ends the take (the Drop no longer hangs on Transcribing)")
    }

    /// The mic failing mid-take: the Drop leaves "Listening" and says so, whatever comes after.
    func testAMicThatFailsMidTakeEndsTheTakeAndTheDropSaysSo() async throws {
        let store = CoreStore()
        let ink = ShellInk(store: store)
        let dictation = DictationModel(send: { _ in })
        let drop = DropController(ink: ink, notes: dictation)
        store.apply([event(#"{"type":"dictation.started","take":0,"edit":false}"#)])
        drop.update()
        XCTAssertEqual(drop.shownText?.detail, "Listening")
        let failed = event(#"{"type":"dictation.mic_failed","message":"mic format changed from 48000 Hz x1 to 44100 Hz x1"}"#)
        store.apply([failed])
        dictation.apply(failed)
        drop.update()
        XCTAssertNil(store.liveDictation)
        XCTAssertEqual(ink.state, .idle)
        XCTAssertEqual(drop.shownText?.title, "Couldn't open the microphone")
        XCTAssertEqual(drop.inkState, .idle)
    }

    func testTheNewestWordsAreWet() {
        let words = "send the deck over before friday and"
        XCTAssertEqual(String(words[DropText.wetStart(words)...]), "friday and")
        XCTAssertEqual(String("hello"[DropText.wetStart("hello")...]), "hello")
        XCTAssertEqual(String("one two"[DropText.wetStart("one two")...]), "one two")
        let styled = DropContentView.liveWords(words)
        XCTAssertEqual(styled.string, words)
    }

    /// A note shows the ink still, stays a moment, and never interrupts a take: one that came
    /// while a take was live shows when it ends.
    func testANoteShowsAfterTheTakeAndGoesAway() async throws {
        let store = CoreStore()
        let ink = ShellInk(store: store)
        let dictation = DictationModel(send: { _ in })
        let drop = DropController(ink: ink, notes: dictation)
        dictation.apply(event(#"{"type":"dictation.discarded","reason":"speech_too_short"}"#))
        drop.update()
        XCTAssertTrue(drop.isShown)
        XCTAssertEqual(drop.shownText, DropText(title: "Too short", detail: "Try again"))
        XCTAssertEqual(drop.inkState, .idle, "the ink is still under a note")
        XCTAssertFalse(drop.panelIsKey)
        // A take starting takes the Drop over.
        store.apply([event(#"{"type":"dictation.started","take":0,"edit":false}"#)])
        drop.update()
        XCTAssertEqual(drop.inkState, .dictating)
        XCTAssertNil(drop.noteShowing)
        // A note during the take (polish ran out of time) waits for its end.
        store.apply([event(#"{"type":"dictation.stopped"}"#)])
        dictation.apply(event(#"{"type":"dictation.warning","kind":"polish_timed_out"}"#))
        drop.update()
        XCTAssertEqual(drop.shownText?.detail, "Transcribing")
        store.apply([event(#"{"type":"dictation.inserted","text":"x","outcome":"pasted"}"#)])
        drop.update()
        XCTAssertEqual(drop.shownText?.title, "Polish took too long")
        try await Task.sleep(for: DropController.noteDuration + .milliseconds(300))
        XCTAssertFalse(drop.isShown, "gone after its moment")
    }
}

@MainActor
final class DictationScreensTests: XCTestCase {
    func testTheCoreBeingReadyAsksForTheKeysAndTheirSettings() {
        let sent = Sent()
        let screens = ScreenModels(send: sent.send)
        screens.apply([event(#"{"type":"core.ready","abi":2,"version":"0.0.0"}"#)])
        XCTAssertTrue(sent.commands.contains(.settingGet(.dictationKey)))
        XCTAssertTrue(sent.commands.contains(.settingGet(.dictationEnabled)))
        screens.apply([event(#"{"type":"setting.value","key":"dictation.enabled"}"#)])
        XCTAssertTrue(sent.commands.contains { if case .dictationEnable = $0 { true } else { false } })
    }
}

/// Against the real core: the dictation commands are read, and answered with events this shell
/// decodes. Without Accessibility for the test runner the answer is dictation.off (the check never
/// prompts); with it, dictation.ready.
final class DictationCoreContractTests: XCTestCase {
    func testDictationCommandsAreReadByTheCoreAndAnswered() throws {
        let data = FileManager.default.temporaryDirectory.appendingPathComponent("inkwell-dictation-\(UUID().uuidString)")
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
        let enabled = try answer(.dictationEnable(utcOffsetMinutes: 60, ref: "dictation:1")) { event -> String? in
            switch event {
            case .dictationReady(let r): r.ref
            case .dictationOff(let o): o.ref
            default: nil
            }
        }
        XCTAssertEqual(enabled, "dictation:1")
        let key = try answer(.settingSet(.dictationKey, "right_option")) { if case .settingValue(let v) = $0 { v } else { nil } }
        XCTAssertEqual(key?.value, "right_option")
        // Every key the recorder names, with a modifier, is one the core watches, and the core
        // spells it as the recorder does: what is recorded is what the tap matches.
        for (i, key) in KeyNotation.allKeys.enumerated() {
            let token = "ctrl+\(key.token)"
            let checked = try answer(.hotkeyCheck(binding: token, ref: "hotkey:\(i)")) { event -> HotkeyChecked? in
                if case .hotkeyChecked(let c) = event, c.ref == "hotkey:\(i)" { c } else { nil }
            }
            XCTAssertEqual(checked?.canonical, token, "\(token): \(checked?.reason ?? "no answer")")
        }
        // And the modifiers alone it names: the right-hand ones and fn held, the rest refused with
        // the core's reason.
        for (token, watched) in [("fn", true), ("right_option", true), ("right_command", true), ("right_control", true),
                                 ("right_shift", true), ("left_option", false), ("left_command", false),
                                 ("left_control", false), ("left_shift", false), ("caps_lock", false), ("ctrl+shift", false)] {
            let checked = try answer(.hotkeyCheck(binding: token, ref: "hotkey:\(token)")) { event -> HotkeyChecked? in
                if case .hotkeyChecked(let c) = event, c.ref == "hotkey:\(token)" { c } else { nil }
            }
            XCTAssertEqual(checked?.ok, watched, token)
            if !watched {
                XCTAssertFalse(checked?.reason?.isEmpty ?? true, token)
            }
        }
        // A recorded chord is stored in the core's spelling.
        let chord = try answer(.settingSet(.dictationKey, "ctrl+shift+space")) { if case .settingValue(let v) = $0 { v } else { nil } }
        XCTAssertEqual(chord?.value, "ctrl+shift+space")
        let switched = try answer(.settingSet(.dictationEnabled, "off")) { if case .settingValue(let v) = $0 { v } else { nil } }
        XCTAssertEqual(switched?.value, "off")
        let disabled = try answer(.dictationDisable(ref: "dictation:2")) { if case .dictationOff(let o) = $0, o.ref == "dictation:2" { o } else { nil } }
        XCTAssertEqual(disabled?.reason, .disabled)
        let undecodable = events.withLock { $0 }.filter { if case .undecodable = $0 { true } else { false } }
        XCTAssertEqual(undecodable, [])
    }
}
