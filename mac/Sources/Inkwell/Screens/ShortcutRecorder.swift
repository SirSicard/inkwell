// "Record a shortcut…" for the dictation key and the edit key (Settings > Dictation).
//
// Everyone may use any key they like, so the quick picks are a start, not the list. While the user
// records, the next key press is captured (a modifier alone when it comes up with nothing else
// pressed, a function key, or modifiers and a key), named as the core names it, and sent to the
// core's hotkey.check: the core is the one judge of what it can watch. Only a key it accepts is
// saved, in the spelling it answers; a refusal is shown with its reason and the old key stays.
// Escape cancels. While recording, dictation is off (dictation.disable), or the current key would
// start a take, and the core's tap would swallow it before the recorder saw it; it comes back on
// (dictation.enable, after the save) when recording ends, however it ends.
import AppKit
import Foundation
import InkBridge
import Observation

/// The capture itself: key events in, a token out. No AppKit, so it is tested with plain numbers.
/// `flags` are `NSEvent.ModifierFlags`' raw bits, whose layout is `CGEventFlags`' (the side of a
/// modifier is in the device bits below the public ones).
struct ShortcutCapture {
    enum Input: Equatable {
        case keyDown(keyCode: Int, flags: UInt, isRepeat: Bool)
        case flagsChanged(keyCode: Int, flags: UInt)
    }

    enum Outcome: Equatable {
        /// Still listening.
        case listening
        /// Escape, with no modifier.
        case cancelled
        /// A token for the core to judge.
        case captured(String)
        /// A key the core has no name for.
        case unknownKey
    }

    enum Flag {
        static let shift: UInt = 0x2_0000
        static let control: UInt = 0x4_0000
        static let option: UInt = 0x8_0000
        static let command: UInt = 0x10_0000
        static let function: UInt = 0x80_0000
    }

    /// A modifier key: its token alone, its chord name, its public flag and its device bit (0 for
    /// Fn, which has none).
    struct Modifier {
        let alone: String
        let chord: String
        let flag: UInt
        let device: UInt
    }

    /// By keyCode (`HIToolbox/Events.h`); the device bits are `IOLLEvent.h`'s NX_DEVICE*KEYMASK.
    static let modifiers: [Int: Modifier] = [
        0x3F: Modifier(alone: "fn", chord: "fn", flag: Flag.function, device: 0),
        0x3B: Modifier(alone: "left_control", chord: "ctrl", flag: Flag.control, device: 0x1),
        0x3E: Modifier(alone: "right_control", chord: "ctrl", flag: Flag.control, device: 0x2000),
        0x38: Modifier(alone: "left_shift", chord: "shift", flag: Flag.shift, device: 0x2),
        0x3C: Modifier(alone: "right_shift", chord: "shift", flag: Flag.shift, device: 0x4),
        0x3A: Modifier(alone: "left_option", chord: "option", flag: Flag.option, device: 0x20),
        0x3D: Modifier(alone: "right_option", chord: "option", flag: Flag.option, device: 0x40),
        0x37: Modifier(alone: "left_command", chord: "cmd", flag: Flag.command, device: 0x8),
        0x36: Modifier(alone: "right_command", chord: "cmd", flag: Flag.command, device: 0x10),
    ]

    static let capsLock = 0x39
    static let escape = 0x35

    /// The modifier keys down now, and every one pressed since the last all-up.
    private var down: Set<Int> = []
    private var pressed: [Int] = []

    mutating func feed(_ input: Input) -> Outcome {
        switch input {
        case .keyDown(_, _, true):
            return .listening
        case .keyDown(let code, let flags, false):
            let named = [Flag.control, Flag.option, Flag.shift, Flag.command].contains { flags & $0 != 0 }
            if code == Self.escape && !named {
                return .cancelled
            }
            guard let key = KeyNotation.byKeyCode[code] else { return .unknownKey }
            var parts: [String] = []
            if flags & Flag.function != 0 && !key.setsFn { parts.append("fn") }
            if flags & Flag.control != 0 { parts.append("ctrl") }
            if flags & Flag.option != 0 { parts.append("option") }
            if flags & Flag.shift != 0 { parts.append("shift") }
            if flags & Flag.command != 0 { parts.append("cmd") }
            parts.append(key.token)
            return .captured(parts.joined(separator: "+"))
        case .flagsChanged(let code, _) where code == Self.capsLock:
            // A switch, not a key that is held: the core says so.
            return .captured("caps_lock")
        case .flagsChanged(let code, let flags):
            guard let modifier = Self.modifiers[code] else { return .listening }
            if Self.isDown(modifier, flags) {
                down.insert(code)
                if !pressed.contains(code) { pressed.append(code) }
                return .listening
            }
            down.remove(code)
            guard down.isEmpty, !pressed.isEmpty else { return .listening }
            defer { pressed = [] }
            if pressed.count == 1, let only = Self.modifiers[pressed[0]] {
                return .captured(only.alone)
            }
            // Modifiers together with no key: the core refuses them and says why.
            let names = Set(pressed.compactMap { Self.modifiers[$0]?.chord })
            return .captured(KeyNotation.chordOrder.filter(names.contains).joined(separator: "+"))
        }
    }

    /// Whether the change left this modifier down. The device bit tells left from right; a
    /// keyboard that sets neither side's bit is read by the public flag.
    static func isDown(_ modifier: Modifier, _ flags: UInt) -> Bool {
        guard modifier.device != 0 else { return flags & modifier.flag != 0 }
        if flags & modifier.device != 0 { return true }
        let sides = modifiers.values.filter { $0.flag == modifier.flag }.reduce(UInt(0)) { $0 | $1.device }
        return flags & sides == 0 && flags & modifier.flag != 0
    }
}

/// The recorder for both keys, its inline messages, and what it asks of dictation.
@MainActor
@Observable
final class ShortcutRecorderModel {
    enum Target: Equatable, Sendable {
        case dictation
        case edit

        /// For VoiceOver: "the dictation key".
        var spoken: String {
            switch self {
            case .dictation: "the dictation key"
            case .edit: "the edit key"
            }
        }
    }

    /// What shows under a key's row after a recording.
    struct Message: Equatable, Sendable {
        let text: String
        let isProblem: Bool
    }

    /// The key being recorded, if any.
    private(set) var recording: Target?
    /// A captured key the core is judging.
    private(set) var checking: (target: Target, token: String)?
    /// The latest message per key.
    private(set) var messages: [Target: Message] = [:]

    /// How long the core has to answer a check before the recorder gives up on it (the queries
    /// thread answers in milliseconds; a core that stopped or hung would not).
    @ObservationIgnored var checkTimeout: Duration = .seconds(5)
    /// How a token is shown: the user's keyboard layout labels the typing keys.
    @ObservationIgnored var describe: @MainActor (String) -> DictationKey = { KeyNotation.display($0) }
    /// Says something to VoiceOver, once, when it happens (never again when the view reappears).
    @ObservationIgnored var announce: @MainActor (String) -> Void = { text in
        NSAccessibility.post(
            element: NSApp as Any, notification: .announcementRequested,
            userInfo: [.announcement: text, .priority: NSAccessibilityPriorityLevel.high.rawValue])
    }
    /// Saves a recorded edit key (ScreenModels.chooseEditKey: it asks for consent first when voice
    /// edit is not on yet).
    @ObservationIgnored var saveEditKey: @MainActor (String) -> Void

    @ObservationIgnored private var capture = ShortcutCapture()
    @ObservationIgnored private var nextRef = 0
    @ObservationIgnored private var ref: String?
    @ObservationIgnored private var timeout: Task<Void, Never>?
    @ObservationIgnored private let send: SendCommand
    @ObservationIgnored private let dictation: DictationModel

    static let refPrefix = "hotkey:"

    init(send: @escaping SendCommand, dictation: DictationModel, saveEditKey: @escaping @MainActor (String) -> Void) {
        self.send = send
        self.dictation = dictation
        self.saveEditKey = saveEditKey
    }

    func message(for target: Target) -> Message? { messages[target] }

    /// Starts recording `target`'s key, or (pressed again, while recording or checking) stops.
    func toggle(_ target: Target) {
        if recording == target || checking?.target == target {
            cancel()
            announce("Recording cancelled. The key is unchanged.")
        } else {
            start(target)
        }
    }

    func start(_ target: Target) {
        dictation.suspendForRecording()
        recording = target
        endCheck()
        capture = ShortcutCapture()
        messages[target] = nil
        announce("Recording a shortcut for \(target.spoken). Press the keys. Escape on its own cancels.")
    }

    /// Escape, the button pressed again, the window gone or the app in the background: nothing
    /// changes, and dictation comes back.
    func cancel() {
        guard recording != nil || checking != nil else { return }
        recording = nil
        endCheck()
        dictation.resumeAfterRecording()
    }

    /// One key event in the recorder's window. Returns whether the recorder took it (the event
    /// then goes no further: Escape does not close the window, a letter types nothing).
    func feed(_ input: ShortcutCapture.Input) -> Bool {
        guard let target = recording else { return false }
        switch capture.feed(input) {
        case .listening:
            break
        case .cancelled:
            cancel()
            announce("Recording cancelled. The key is unchanged.")
        case .unknownKey:
            recording = nil
            show("Inkwell doesn\u{2019}t know that key (keypad and media keys, for one). Try another.", for: target)
            dictation.resumeAfterRecording()
        case .captured(let token):
            recording = nil
            checking = (target, token)
            nextRef += 1
            let ref = "\(Self.refPrefix)\(nextRef)"
            self.ref = ref
            messages[target] = Message(text: "Checking \(describe(token).cap)\u{2026}", isProblem: false)
            send(.hotkeyCheck(binding: token, ref: ref))
            let wait = checkTimeout
            timeout = Task { [weak self] in
                try? await Task.sleep(for: wait)
                guard !Task.isCancelled else { return }
                self?.timedOut(ref)
            }
        }
        return true
    }

    func apply(_ event: InkEvent) {
        switch event {
        case .hotkeyChecked(let checked) where checked.ref != nil && checked.ref == ref:
            guard let (target, token) = checking else { return }
            endCheck()
            if checked.ok, let canonical = checked.canonical {
                save(canonical, for: target, shown: describe(canonical))
            } else {
                show("Can\u{2019}t use \(describe(token).cap): \(checked.reason ?? "this Mac can\u{2019}t watch it").", for: target)
            }
            dictation.resumeAfterRecording()
        case .commandFailed(let failed) where failed.id != nil && failed.id == ref:
            if let (target, _) = checking {
                endCheck()
                show("Couldn\u{2019}t check that shortcut. The key before still works.", for: target)
            }
            dictation.resumeAfterRecording()
        case .coreStopped:
            // Nothing will answer the check now; the restarted core turns dictation on from its
            // switch (DictationModel forgets the pause too).
            recording = nil
            endCheck()
        default:
            break
        }
    }

    /// Whether this model shows a failed command itself.
    func handles(_ failed: CommandFailed) -> Bool {
        failed.id?.hasPrefix(Self.refPrefix) ?? false
    }

    private func timedOut(_ ref: String) {
        guard self.ref == ref, let (target, _) = checking else { return }
        endCheck()
        show("Couldn\u{2019}t check that shortcut in time. The key before still works.", for: target)
        dictation.resumeAfterRecording()
    }

    /// Forgets the check in flight, and its "Checking…" line.
    private func endCheck() {
        if let (target, _) = checking, messages[target]?.isProblem == false {
            messages[target] = nil
        }
        checking = nil
        ref = nil
        timeout?.cancel()
        timeout = nil
    }

    private func show(_ text: String, for target: Target) {
        messages[target] = Message(text: text, isProblem: true)
        announce(text)
    }

    /// The two keys are never one: the core would refuse the edit key, and a key that dictates and
    /// edits at once does neither well.
    private func save(_ canonical: String, for target: Target, shown key: DictationKey) {
        switch target {
        case .dictation:
            if canonical == dictation.editKey {
                show("\(key.cap) is the edit key. Pick another, or change the edit key first.", for: target)
                return
            }
            dictation.setKey(canonical)
        case .edit:
            if canonical == dictation.key {
                show("\(key.cap) is the dictation key. Pick another.", for: target)
                return
            }
            saveEditKey(canonical)
        }
        // The edit key may still wait on the consent step, so it is "chosen", not set.
        let saved = target == .dictation ? "Dictation key set to \(key.name)." : "Edit key chosen: \(key.name)."
        if let clash = KeyNotation.clash(canonical, cap: key.cap) {
            messages[target] = Message(text: clash, isProblem: true)
            announce("\(saved) \(clash)")
        } else {
            messages[target] = nil
            announce(saved)
        }
    }
}
