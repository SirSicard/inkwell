// "Record a shortcut…" (Settings > Dictation): any key the core can watch is a dictation or edit
// key. The capture names keys as the core does, the core judges them (hotkey.check), only a key it
// accepts is saved, and dictation is off while recording. The real core's answers for every key
// the recorder names are in DictationCoreContractTests; pressing keys in the running app is on
// mac/DICTATION-CHECKLIST.md.
import AppKit
import Carbon.HIToolbox
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
        // Modifiers with no key (recorded, refused by the core): their glyphs.
        XCTAssertEqual(KeyNotation.describe("ctrl+shift").cap, "\u{2303}\u{21E7}")
        XCTAssertEqual(KeyNotation.describe("ctrl+shift").name, "Control-Shift")
        XCTAssertEqual(KeyNotation.describe("ctrl+section").cap, "\u{2303}\u{00A7}")
        XCTAssertEqual(DictationModel.key("ctrl+shift+space")?.cap, "\u{2303}\u{21E7}Space")
    }

    /// A typing key shows as the layout labels it (an AZERTY Mac's A sits where ANSI has Q); the
    /// token stays positional, as the core watches it. Keys that type nothing never change.
    func testTypingKeysShowAsTheLayoutLabelsThem() {
        let azerty: (Int) -> String? = { [kVK_ANSI_Q: "a", kVK_ANSI_A: "q", kVK_ANSI_Semicolon: "m", kVK_Space: " "][$0] }
        let quit = KeyNotation.describe("cmd+q", layout: azerty)
        XCTAssertEqual(quit.token, "cmd+q")
        XCTAssertEqual(quit.cap, "\u{2318}A")
        XCTAssertEqual(quit.name, "Command-A")
        XCTAssertEqual(KeyNotation.describe("ctrl+semicolon", layout: azerty).cap, "\u{2303}M")
        XCTAssertEqual(KeyNotation.describe("ctrl+space", layout: azerty).cap, "\u{2303}Space")
        // A clash follows what is shown: ⌘A on that Mac is Select All, wherever it sits.
        XCTAssertEqual(
            KeyNotation.clash("cmd+q", cap: quit.cap),
            "\u{2318}A is Select All in most apps; while dictation is on, Inkwell takes it from them.")
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
        XCTAssertEqual(run([.keyDown(keyCode: 0x0A, flags: F.control, isRepeat: false)]), .captured("ctrl+section"), "ISO \u{00A7}")
    }
}

@MainActor
final class ShortcutRecorderTests: XCTestCase {
    private func live(_ sent: Sent) -> ScreenModels {
        let screens = ScreenModels(send: sent.send)
        screens.apply([event(#"{"type":"setting.value","key":"dictation.enabled","value":"on"}"#),
                       event(#"{"type":"dictation.ready","key":"fn"}"#)])
        sent.commands.removeAll()
        screens.shortcuts.announce = { _ in }
        return screens
    }

    private func acknowledge(_ screens: ScreenModels, _ sent: Sent) {
        for command in sent.commands {
            switch command {
            case .dictationDisable(let ref):
                screens.apply([event("{\"type\":\"dictation.off\",\"reason\":\"disabled\",\"ref\":\"\(ref)\"}")])
            case .meetingsShortcutSuspend(true, let ref):
                screens.apply([event("{\"type\":\"meetings.shortcut.state\",\"key\":\"\(screens.meetingShortcut.key)\",\"active\":false,\"suspended\":true,\"ref\":\"\(ref ?? "")\"}")])
            default: break
            }
        }
    }

    private func answer(_ screens: ScreenModels, _ sent: Sent, canonical: String = "f13", ok: Bool = true) {
        guard case .hotkeyCheck(_, let ref) = sent.commands.last else { return XCTFail("no check") }
        screens.apply([event("{\"type\":\"hotkey.checked\",\"binding\":\"f13\",\"ok\":\(ok),\"canonical\":\"\(canonical)\",\"reason\":\"refused\",\"ref\":\"\(ref)\"}")])
    }

    func testCaptureWaitsForBothCorrelatedSuspensionAcknowledgements() {
        let sent = Sent(); let screens = live(sent); let recorder = screens.shortcuts
        recorder.start(.meeting)
        XCTAssertNil(recorder.recording)
        XCTAssertEqual(recorder.waiting, .meeting)
        XCTAssertFalse(recorder.feed(.keyDown(keyCode: 0x69, flags: 0, isRepeat: false)))
        guard case .meetingsShortcutSuspend(true, let ref) = sent.commands.last else { return XCTFail("no meeting pause") }
        screens.apply([event("{\"type\":\"meetings.shortcut.state\",\"key\":\"\(screens.meetingShortcut.key)\",\"active\":false,\"suspended\":true,\"ref\":\"\(ref ?? "")\"}")])
        XCTAssertNil(recorder.recording, "dictation has not acknowledged")
        acknowledge(screens, sent)
        XCTAssertEqual(recorder.recording, .meeting)
    }

    func testAcceptedKeyWaitsForReleaseBeforeSavingAndResuming() {
        let sent = Sent(); let screens = live(sent); let recorder = screens.shortcuts
        recorder.start(.meeting); acknowledge(screens, sent)
        XCTAssertTrue(recorder.feed(.keyDown(keyCode: 0x69, flags: 0, isRepeat: false)))
        answer(screens, sent)
        XCTAssertFalse(sent.commands.contains(.settingSet(.meetingsKey, "f13")))
        XCTAssertTrue(recorder.feed(.keyUp(keyCode: 0x69, flags: 0)))
        XCTAssertTrue(sent.commands.contains(.settingSet(.meetingsKey, "f13")))
        XCTAssertEqual(sent.commands.last, .meetingsShortcutSuspend(suspended: false, ref: nil))
        XCTAssertFalse(screens.dictation.suspendedForRecording)
    }

    func testThreeWayConflictPreservesStoredKeys() {
        for target in [ShortcutRecorderModel.Target.dictation, .edit, .meeting] {
            let sent = Sent(); let screens = live(sent); let recorder = screens.shortcuts
            screens.apply([event(#"{"type":"meetings.shortcut.state","key":"f13","active":true,"suspended":false}"#)])
            recorder.start(target); acknowledge(screens, sent)
            _ = recorder.feed(.flagsChanged(keyCode: 0x3F, flags: F.function))
            _ = recorder.feed(.flagsChanged(keyCode: 0x3F, flags: 0))
            answer(screens, sent, canonical: target == .meeting ? "fn" : "f13")
            XCTAssertTrue(recorder.message(for: target)?.isProblem ?? false)
            XCTAssertFalse(sent.commands.contains { if case .settingSet = $0 { true } else { false } })
        }
    }

    func testCancellationAndFailedSuspensionResumeBothAndIgnoreLateAnswers() {
        let sent = Sent(); let screens = live(sent); let recorder = screens.shortcuts
        recorder.start(.edit)
        guard case .meetingsShortcutSuspend(true, let ref) = sent.commands.last else { return XCTFail("no pause") }
        screens.apply([event("{\"type\":\"command.failed\",\"command\":\"meetings.shortcut.suspend\",\"id\":\"\(ref ?? "")\",\"message\":\"failed\"}")])
        XCTAssertNil(recorder.waiting)
        XCTAssertFalse(screens.dictation.suspendedForRecording)
        XCTAssertEqual(sent.commands.last, .meetingsShortcutSuspend(suspended: false, ref: nil))
        acknowledge(screens, sent)
        XCTAssertNil(recorder.recording)
    }

    func testStoppedCoreClearsCaptureAndLateApproval() {
        let sent = Sent(); let screens = live(sent); let recorder = screens.shortcuts
        recorder.start(.meeting); acknowledge(screens, sent)
        _ = recorder.feed(.keyDown(keyCode: 0x69, flags: 0, isRepeat: false))
        guard case .hotkeyCheck(_, let ref) = sent.commands.last else { return XCTFail("no check") }
        let count = sent.commands.count
        screens.apply([event(#"{"type":"core.stopped"}"#)])
        XCTAssertFalse(recorder.busy)
        XCTAssertFalse(screens.dictation.suspendedForRecording)
        screens.apply([event("{\"type\":\"hotkey.checked\",\"binding\":\"f13\",\"canonical\":\"f13\",\"ok\":true,\"ref\":\"\(ref)\"}")])
        XCTAssertEqual(sent.commands.count, count)
    }

    func testSuspensionTimeoutResumesBoth() async throws {
        let sent = Sent(); let screens = live(sent); let recorder = screens.shortcuts
        recorder.checkTimeout = .milliseconds(10)
        recorder.start(.meeting)
        try await Task.sleep(for: .milliseconds(60))
        XCTAssertFalse(recorder.busy)
        XCTAssertTrue(recorder.message(for: .meeting)?.isProblem ?? false)
        XCTAssertEqual(sent.commands.last, .meetingsShortcutSuspend(suspended: false, ref: nil))
    }

    func testMeetingStateOnlyAdvertisesAcknowledgedKeyAndDefaultsOff() {
        let sent = Sent(); let screens = live(sent)
        XCTAssertEqual(screens.meetingShortcut.key, "off")
        screens.meetingShortcut.setKey("f13")
        XCTAssertEqual(screens.meetingShortcut.key, "off")
        screens.apply([event(#"{"type":"meetings.shortcut.state","key":"f13","active":true,"suspended":false}"#)])
        XCTAssertEqual(screens.meetingShortcut.key, "f13")
    }
    func testRefusedCheckUnknownKeyAndFailedCheckEachResumeWithoutSaving() {
        for outcome in ["refused", "unknown", "failed"] {
            let sent = Sent(); let screens = live(sent); let recorder = screens.shortcuts
            recorder.start(.dictation); acknowledge(screens, sent)
            _ = recorder.feed(.keyDown(keyCode: outcome == "unknown" ? 0x52 : 0x69, flags: 0, isRepeat: false))
            if outcome == "refused" {
                answer(screens, sent, ok: false)
                _ = recorder.feed(.keyUp(keyCode: 0x69, flags: 0))
            } else if outcome == "failed", case .hotkeyCheck(_, let ref) = sent.commands.last {
                screens.apply([event("{\"type\":\"command.failed\",\"command\":\"hotkey.check\",\"id\":\"\(ref)\",\"message\":\"failed\"}")])
            }
            XCTAssertFalse(recorder.busy)
            XCTAssertTrue(recorder.message(for: .dictation)?.isProblem ?? false)
            XCTAssertFalse(screens.dictation.suspendedForRecording)
            XCTAssertEqual(sent.commands.last, .meetingsShortcutSuspend(suspended: false, ref: nil))
            XCTAssertFalse(sent.commands.contains { if case .settingSet = $0 { true } else { false } })
        }
    }

    func testEscapeFocusLossAndCheckCancellationAreIdempotent() {
        for exit in ["escape", "focus", "check"] {
            let sent = Sent(); let screens = live(sent); let recorder = screens.shortcuts
            recorder.start(.edit); acknowledge(screens, sent)
            if exit == "escape" {
                _ = recorder.feed(.keyDown(keyCode: 0x35, flags: 0, isRepeat: false))
            } else {
                if exit == "check" { _ = recorder.feed(.keyDown(keyCode: 0x69, flags: 0, isRepeat: false)) }
                recorder.cancel()
            }
            let count = sent.commands.count
            recorder.cancel()
            XCTAssertEqual(sent.commands.count, count)
            XCTAssertFalse(recorder.busy)
            XCTAssertFalse(screens.dictation.suspendedForRecording)
        }
    }

    func testAcceptedEditKeyStillRequestsConsentAndKnownSystemClashWarns() {
        let sent = Sent(); let screens = live(sent); let recorder = screens.shortcuts
        screens.apply([event(#"{"type":"consent.state","feature":"edit","on":false,"allowed":false,"to":"on_device","name":"SystemLanguageModel.default"}"#)])
        recorder.start(.edit); acknowledge(screens, sent)
        _ = recorder.feed(.keyDown(keyCode: 0x69, flags: 0, isRepeat: false))
        answer(screens, sent, canonical: "cmd+space")
        _ = recorder.feed(.keyUp(keyCode: 0x69, flags: 0))
        XCTAssertEqual(screens.editConsent.pendingKey, "cmd+space")
        XCTAssertTrue(recorder.message(for: .edit)?.text.contains("Spotlight") ?? false)
    }

    func testOffAndUnreadSwitchNeverResumeDictationWithoutUserIntent() {
        for value in ["off", "unread"] {
            let sent = Sent(); let screens = ScreenModels(send: sent.send)
            screens.shortcuts.announce = { _ in }
            if value == "off" { screens.apply([event(#"{"type":"setting.value","key":"dictation.enabled","value":"off"}"#)]) }
            sent.commands.removeAll()
            screens.shortcuts.start(.meeting)
            XCTAssertTrue(sent.commands.contains { if case .dictationDisable = $0 { true } else { false } })
            screens.dictation.enable()
            screens.dictation.retry()
            screens.shortcuts.cancel()
            XCTAssertFalse(sent.commands.contains { if case .dictationEnable = $0 { true } else { false } })
        }
    }

    func testModifierReleaseAfterChordKeyUpIsRequiredBeforeRearming() {
        let sent = Sent(); let screens = live(sent); let recorder = screens.shortcuts
        recorder.start(.meeting); acknowledge(screens, sent)
        _ = recorder.feed(.keyDown(keyCode: 0x31, flags: F.control, isRepeat: false))
        answer(screens, sent, canonical: "ctrl+space")
        _ = recorder.feed(.keyUp(keyCode: 0x31, flags: F.control))
        XCTAssertTrue(recorder.capturing)
        XCTAssertFalse(sent.commands.contains(.settingSet(.meetingsKey, "ctrl+space")))
        _ = recorder.feed(.flagsChanged(keyCode: 0x3B, flags: 0))
        XCTAssertTrue(sent.commands.contains(.settingSet(.meetingsKey, "ctrl+space")))
    }

    func testRecorderFiltersOtherWindowsAndNoHost() {
        let sent = Sent(); let screens = live(sent); let recorder = screens.shortcuts
        recorder.start(.meeting); acknowledge(screens, sent)
        let host = NSWindow(contentRect: .zero, styleMask: [.titled], backing: .buffered, defer: true)
        let other = NSWindow(contentRect: .zero, styleMask: [.titled], backing: .buffered, defer: true)
        let key = ShortcutCapture.Input.keyDown(keyCode: 0x69, flags: 0, isRepeat: false)
        XCTAssertFalse(ShortcutRecordingFilter.feed(key, from: ObjectIdentifier(other), host: host, to: recorder))
        XCTAssertFalse(ShortcutRecordingFilter.feed(key, from: ObjectIdentifier(host), host: nil, to: recorder))
        XCTAssertTrue(ShortcutRecordingFilter.feed(key, from: ObjectIdentifier(host), host: host, to: recorder))
    }

    func testCheckTimeoutResumesAndLateAnswerCannotSave() async throws {
        let sent = Sent(); let screens = live(sent); let recorder = screens.shortcuts
        recorder.checkTimeout = .milliseconds(10)
        recorder.start(.meeting); acknowledge(screens, sent)
        _ = recorder.feed(.keyDown(keyCode: 0x69, flags: 0, isRepeat: false))
        guard case .hotkeyCheck(_, let ref) = sent.commands.last else { return XCTFail("no check") }
        try await Task.sleep(for: .milliseconds(60))
        XCTAssertFalse(recorder.busy)
        screens.apply([event("{\"type\":\"hotkey.checked\",\"binding\":\"f13\",\"canonical\":\"f13\",\"ok\":true,\"ref\":\"\(ref)\"}")])
        XCTAssertFalse(sent.commands.contains(.settingSet(.meetingsKey, "f13")))
    }

}
