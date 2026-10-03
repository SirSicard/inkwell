// "Record a shortcut…" (Settings > Dictation): any key the core can watch is a dictation or edit
// key. The capture names keys as the core does, the core judges them (hotkey.check), only a key it
// accepts is saved, and dictation is off while recording. The real core's answers for every key
// the recorder names are in DictationCoreContractTests; pressing keys in the running app is on
// mac/DICTATION-CHECKLIST.md.
import Foundation
import InkBridge
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

private typealias F = ShortcutCapture.Flag

@MainActor
final class KeyNotationTests: XCTestCase {
    func testATokenReadsInMacNotationWithANameToSayAloud() {
        let cases: [(String, String, String)] = [
            ("ctrl+shift+space", "\u{2303}\u{21E7}Space", "Control-Shift-Space"),
            ("option+cmd+d", "\u{2325}\u{2318}D", "Option-Command-D"),
            ("right_option", "Right \u{2325}", "Right Option"),
            ("fn", "fn", "fn (Globe)"),
            ("f13", "F13", "F13"),
            ("fn+f5", "fn F5", "fn-F5"),
            ("ctrl+left", "\u{2303}\u{2190}", "Control-Left Arrow"),
            ("cmd+slash", "\u{2318}/", "Command-Slash"),
        ]
        for (token, cap, name) in cases {
            XCTAssertEqual(KeyNotation.describe(token).cap, cap, token)
            XCTAssertEqual(KeyNotation.describe(token).name, name, token)
        }
        // Outside the grammar: shown as it is, never dropped.
        XCTAssertEqual(KeyNotation.describe("hyper+x").cap, "hyper+x")
        XCTAssertEqual(DictationModel.key("ctrl+shift+space")?.cap, "\u{2303}\u{21E7}Space")
    }

    func testEveryKeyHasOneTokenAndOneKeyCode() {
        XCTAssertEqual(Set(KeyNotation.allKeys.map(\.token)).count, KeyNotation.allKeys.count)
        XCTAssertEqual(Set(KeyNotation.allKeys.map(\.keyCode)).count, KeyNotation.allKeys.count)
    }

    func testShortcutsMacOSOrMostAppsTakeAreNamed() {
        XCTAssertEqual(KeyNotation.clash("cmd+space"), "\u{2318}Space is the shortcut for Spotlight, so the two may clash.")
        XCTAssertEqual(
            KeyNotation.clash("cmd+c"),
            "\u{2318}C is Copy in most apps; while dictation is on, Inkwell takes it from them.")
        XCTAssertNil(KeyNotation.clash("ctrl+shift+space"))
    }
}

final class ShortcutCaptureTests: XCTestCase {
    private func run(_ inputs: [ShortcutCapture.Input]) -> ShortcutCapture.Outcome {
        var capture = ShortcutCapture()
        var last = ShortcutCapture.Outcome.listening
        for input in inputs {
            last = capture.feed(input)
            if last != .listening { break }
        }
        return last
    }

    func testModifiersAndAKeyAreCapturedAtTheKeyInTheCoresOrder() {
        let flags = F.shift | F.control | 0x1 | 0x2
        XCTAssertEqual(
            run([.flagsChanged(keyCode: 0x38, flags: F.shift | 0x2),
                 .flagsChanged(keyCode: 0x3B, flags: flags),
                 .keyDown(keyCode: 0x31, flags: flags, isRepeat: false)]),
            .captured("ctrl+shift+space"))
        XCTAssertEqual(
            run([.keyDown(keyCode: 0x02, flags: F.command | F.option | 0x8 | 0x20, isRepeat: false)]),
            .captured("option+cmd+d"))
    }

    func testAFunctionKeyAloneAndTheFnTheKeyboardAddsToIt() {
        XCTAssertEqual(run([.keyDown(keyCode: 0x69, flags: 0, isRepeat: false)]), .captured("f13"))
        // Laptops set Fn on the function row and the arrows by themselves: not part of the shortcut.
        XCTAssertEqual(run([.keyDown(keyCode: 0x60, flags: F.function, isRepeat: false)]), .captured("f5"))
        XCTAssertEqual(run([.keyDown(keyCode: 0x7B, flags: F.function | F.control, isRepeat: false)]), .captured("ctrl+left"))
        // Fn held with a letter is the user's.
        XCTAssertEqual(run([.keyDown(keyCode: 0x02, flags: F.function, isRepeat: false)]), .captured("fn+d"))
    }

    func testAModifierAloneIsCapturedWhenItComesUpWithNothingElsePressed() {
        XCTAssertEqual(
            run([.flagsChanged(keyCode: 0x3D, flags: F.option | 0x40), .flagsChanged(keyCode: 0x3D, flags: 0)]),
            .captured("right_option"))
        XCTAssertEqual(
            run([.flagsChanged(keyCode: 0x3A, flags: F.option | 0x20), .flagsChanged(keyCode: 0x3A, flags: 0)]),
            .captured("left_option"), "the core refuses it, and says why")
        XCTAssertEqual(
            run([.flagsChanged(keyCode: 0x3F, flags: F.function), .flagsChanged(keyCode: 0x3F, flags: 0)]),
            .captured("fn"))
        // Right Command released while left Command is still down: the device bit says which.
        XCTAssertEqual(
            run([.flagsChanged(keyCode: 0x36, flags: F.command | 0x10), .flagsChanged(keyCode: 0x36, flags: 0)]),
            .captured("right_command"))
        // Two modifiers and no key: named for the core to refuse.
        XCTAssertEqual(
            run([.flagsChanged(keyCode: 0x38, flags: F.shift | 0x2),
                 .flagsChanged(keyCode: 0x3B, flags: F.shift | F.control | 0x3),
                 .flagsChanged(keyCode: 0x38, flags: F.control | 0x1),
                 .flagsChanged(keyCode: 0x3B, flags: 0)]),
            .captured("ctrl+shift"))
        // A keyboard that reports no side: the public flag says down.
        XCTAssertEqual(
            run([.flagsChanged(keyCode: 0x3E, flags: F.control), .flagsChanged(keyCode: 0x3E, flags: 0)]),
            .captured("right_control"))
    }

    func testEscapeCancelsUnlessItIsPartOfAShortcut() {
        XCTAssertEqual(run([.keyDown(keyCode: 0x35, flags: 0, isRepeat: false)]), .cancelled)
        XCTAssertEqual(run([.keyDown(keyCode: 0x35, flags: F.control | 0x1, isRepeat: false)]), .captured("ctrl+escape"))
    }

    func testRepeatsUnknownKeysAndCapsLock() {
        XCTAssertEqual(run([.keyDown(keyCode: 0x31, flags: F.control, isRepeat: true)]), .listening)
        XCTAssertEqual(run([.keyDown(keyCode: 0x52, flags: F.control, isRepeat: false)]), .unknownKey, "keypad 0")
        XCTAssertEqual(run([.flagsChanged(keyCode: 0x39, flags: 0x1_0000)]), .captured("caps_lock"))
    }
}

@MainActor
final class ShortcutRecorderTests: XCTestCase {
    /// Dictation live on fn, its switch on.
    private func live(_ sent: Sent) -> ScreenModels {
        let screens = ScreenModels(send: sent.send)
        screens.apply([
            event(#"{"type":"setting.value","key":"dictation.enabled","value":"on"}"#),
            event(#"{"type":"dictation.ready","key":"fn"}"#),
        ])
        sent.commands.removeAll()
        return screens
    }

    private func press(_ recorder: ShortcutRecorderModel, _ keyCode: Int, _ flags: UInt) -> Bool {
        recorder.feed(.keyDown(keyCode: keyCode, flags: flags, isRepeat: false))
    }

    func testAnAcceptedShortcutIsSavedInTheCoresSpellingAndDictationComesBackOnIt() {
        let sent = Sent()
        let screens = live(sent)
        let recorder = screens.shortcuts
        recorder.toggle(.dictation)
        XCTAssertEqual(recorder.recording, .dictation)
        XCTAssertEqual(sent.commands, [.dictationDisable(ref: "dictation:2")], "the current key starts nothing meanwhile")
        screens.apply([event(#"{"type":"dictation.off","reason":"disabled","ref":"dictation:2"}"#)])
        XCTAssertEqual(screens.dictation.status, "Dictation is paused while you record a shortcut.")
        XCTAssertFalse(screens.dictation.isProblem)
        XCTAssertTrue(press(recorder, 0x31, F.control | F.shift), "taken from the window")
        XCTAssertNil(recorder.recording)
        XCTAssertEqual(sent.commands.last, .hotkeyCheck(binding: "ctrl+shift+space", ref: "hotkey:1"))
        XCTAssertFalse(press(recorder, 0x00, 0), "no longer recording: the window's")
        screens.apply([event(#"{"type":"hotkey.checked","binding":"ctrl+shift+space","ok":true,"canonical":"ctrl+shift+space","ref":"hotkey:1"}"#)])
        XCTAssertEqual(
            Array(sent.commands.suffix(2)),
            [.settingSet(.dictationKey, "ctrl+shift+space"), .dictationEnable(utcOffsetMinutes: TimeZone.current.secondsFromGMT() / 60, ref: "dictation:3")],
            "saved first, so dictation comes back on the new key")
        XCTAssertNil(recorder.message(for: .dictation))
        XCTAssertNil(recorder.checking)
    }

    func testARefusedShortcutSaysWhyAndKeepsTheKeyBefore() {
        let sent = Sent()
        let screens = live(sent)
        let recorder = screens.shortcuts
        recorder.start(.dictation)
        _ = press(recorder, 0x00, 0)
        XCTAssertEqual(sent.commands.last, .hotkeyCheck(binding: "a", ref: "hotkey:1"))
        screens.apply([event(#"{"type":"hotkey.checked","binding":"a","ok":false,"reason":"that key on its own would stop working everywhere else; add Control, Option or Command","ref":"hotkey:1"}"#)])
        XCTAssertEqual(
            recorder.message(for: .dictation),
            .init(text: "Can\u{2019}t use A: that key on its own would stop working everywhere else; add Control, Option or Command.", isProblem: true))
        XCTAssertFalse(sent.commands.contains { if case .settingSet(.dictationKey, _) = $0 { true } else { false } })
        XCTAssertEqual(sent.commands.last, .dictationEnable(utcOffsetMinutes: TimeZone.current.secondsFromGMT() / 60, ref: "dictation:3"))
        // An answer to another check changes nothing.
        screens.apply([event(#"{"type":"hotkey.checked","binding":"f13","ok":true,"canonical":"f13","ref":"hotkey:9"}"#)])
        XCTAssertFalse(sent.commands.contains(.settingSet(.dictationKey, "f13")))
    }

    func testEscapeCancelsAndChangesNothing() {
        let sent = Sent()
        let screens = live(sent)
        let recorder = screens.shortcuts
        recorder.start(.dictation)
        XCTAssertTrue(press(recorder, 0x35, 0))
        XCTAssertNil(recorder.recording)
        XCTAssertEqual(sent.commands, [.dictationDisable(ref: "dictation:2"), .dictationEnable(utcOffsetMinutes: TimeZone.current.secondsFromGMT() / 60, ref: "dictation:3")])
        // Cancelling again, or leaving the window, asks nothing more.
        recorder.cancel()
        XCTAssertEqual(sent.commands.count, 2)
    }

    func testTheTwoKeysAreNeverOne() {
        let sent = Sent()
        let screens = live(sent)
        let recorder = screens.shortcuts
        recorder.start(.edit)
        _ = recorder.feed(.flagsChanged(keyCode: 0x3F, flags: F.function))
        _ = recorder.feed(.flagsChanged(keyCode: 0x3F, flags: 0))
        XCTAssertEqual(sent.commands.last, .hotkeyCheck(binding: "fn", ref: "hotkey:1"))
        screens.apply([event(#"{"type":"hotkey.checked","binding":"fn","ok":true,"canonical":"fn","ref":"hotkey:1"}"#)])
        XCTAssertEqual(recorder.message(for: .edit)?.text, "fn is the dictation key. Pick another.")
        XCTAssertFalse(sent.commands.contains { if case .settingSet(.dictationEditKey, _) = $0 { true } else { false } })
    }

    func testARecordedEditKeyAsksForConsentAsAPickedOneDoes() {
        let sent = Sent()
        let screens = live(sent)
        // Voice edit off, its model on this Mac.
        screens.apply([event(#"{"type":"consent.state","feature":"edit","on":false,"allowed":false,"to":"on_device","name":"SystemLanguageModel.default"}"#)])
        let recorder = screens.shortcuts
        recorder.start(.edit)
        _ = press(recorder, 0x0E, F.command | F.option)
        screens.apply([event(#"{"type":"hotkey.checked","binding":"option+cmd+e","ok":true,"canonical":"option+cmd+e","ref":"hotkey:1"}"#)])
        XCTAssertEqual(screens.editConsent.pendingKey, "option+cmd+e", "the consent step asks first")
    }

    func testAShortcutTheAppKnowsIsTakenIsSavedWithAWarning() {
        let sent = Sent()
        let screens = live(sent)
        let recorder = screens.shortcuts
        recorder.start(.dictation)
        _ = press(recorder, 0x31, F.command)
        screens.apply([event(#"{"type":"hotkey.checked","binding":"cmd+space","ok":true,"canonical":"cmd+space","ref":"hotkey:1"}"#)])
        XCTAssertTrue(sent.commands.contains(.settingSet(.dictationKey, "cmd+space")))
        XCTAssertEqual(recorder.message(for: .dictation)?.text, "\u{2318}Space is the shortcut for Spotlight, so the two may clash.")
    }

    func testACheckThatFailedSaysSoAndDictationComesBack() {
        let sent = Sent()
        let screens = live(sent)
        let recorder = screens.shortcuts
        recorder.start(.dictation)
        _ = press(recorder, 0x69, 0)
        let failure = event(#"{"type":"command.failed","command":"hotkey.check","id":"hotkey:1","message":"a bug in the core stopped this command"}"#)
        if case .commandFailed(let failed) = failure {
            XCTAssertTrue(screens.handles(failed), "said under the key's row")
        } else {
            XCTFail("not a command.failed")
        }
        screens.apply([failure])
        XCTAssertEqual(recorder.message(for: .dictation)?.text, "Couldn\u{2019}t check that shortcut. The key before still works.")
        XCTAssertEqual(sent.commands.last, .dictationEnable(utcOffsetMinutes: TimeZone.current.secondsFromGMT() / 60, ref: "dictation:3"))
    }

    func testWithDictationOffRecordingTurnsNothingOnOrOff() {
        let sent = Sent()
        let screens = ScreenModels(send: sent.send)
        screens.apply([event(#"{"type":"setting.value","key":"dictation.enabled","value":"off"}"#)])
        sent.commands.removeAll()
        screens.shortcuts.start(.dictation)
        XCTAssertTrue(screens.shortcuts.feed(.keyDown(keyCode: 0x35, flags: 0, isRepeat: false)))
        XCTAssertEqual(sent.commands, [])
    }

    func testTheCommandIsReadByItsName() throws {
        let json = CoreCommand.hotkeyCheck(binding: "ctrl+shift+space", ref: "hotkey:1").json
        let fields = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: String])
        XCTAssertEqual(fields, ["cmd": "hotkey.check", "binding": "ctrl+shift+space", "id": "hotkey:1"])
        XCTAssertEqual(CoreCommand.hotkeyCheck(binding: "x", ref: "r").name, "hotkey.check")
    }
}
