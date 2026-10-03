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

    @ObservationIgnored private var capture = ShortcutCapture()
    @ObservationIgnored private var nextRef = 0
    @ObservationIgnored private var ref: String?
    @ObservationIgnored private let send: SendCommand
    @ObservationIgnored private let dictation: DictationModel
    /// Saves a recorded edit key (ScreenModels.chooseEditKey: it asks for consent first when voice
    /// edit is not on yet).
    @ObservationIgnored var saveEditKey: @MainActor (String) -> Void

    static let refPrefix = "hotkey:"

    init(send: @escaping SendCommand, dictation: DictationModel, saveEditKey: @escaping @MainActor (String) -> Void) {
        self.send = send
        self.dictation = dictation
        self.saveEditKey = saveEditKey
    }

    func message(for target: Target) -> Message? { messages[target] }

    /// Starts recording `target`'s key, or (pressed again) stops.
    func toggle(_ target: Target) {
        if recording == target {
            cancel()
        } else {
            start(target)
        }
    }

    func start(_ target: Target) {
        if recording == nil && checking == nil {
            dictation.suspendForRecording()
        }
        recording = target
        checking = nil
        ref = nil
        capture = ShortcutCapture()
        messages[target] = nil
    }

    /// Escape, the button pressed again, the window gone or the app in the background: nothing
    /// changes, and dictation comes back.
    func cancel() {
        guard recording != nil || checking != nil else { return }
        recording = nil
        checking = nil
        ref = nil
        dictation.resumeAfterRecording()
    }

    /// One key event while the window is key. Returns whether the recorder took it (the event
    /// then goes no further: Escape does not close the window, a letter types nothing).
    func feed(_ input: ShortcutCapture.Input) -> Bool {
        guard let target = recording else { return false }
        switch capture.feed(input) {
        case .listening:
            break
        case .cancelled:
            cancel()
        case .unknownKey:
            recording = nil
            messages[target] = Message(text: "Inkwell doesn\u{2019}t know that key. Try another.", isProblem: true)
            dictation.resumeAfterRecording()
        case .captured(let token):
            recording = nil
            checking = (target, token)
            nextRef += 1
            let ref = "\(Self.refPrefix)\(nextRef)"
            self.ref = ref
            send(.hotkeyCheck(binding: token, ref: ref))
        }
        return true
    }

    func apply(_ event: InkEvent) {
        switch event {
        case .hotkeyChecked(let checked) where checked.ref != nil && checked.ref == ref:
            guard let (target, token) = checking else { return }
            checking = nil
            ref = nil
            let cap = KeyNotation.describe(token).cap
            if checked.ok, let canonical = checked.canonical {
                save(canonical, for: target, cap: cap)
            } else {
                messages[target] = Message(text: "Can\u{2019}t use \(cap): \(checked.reason ?? "this Mac can\u{2019}t watch it").", isProblem: true)
            }
            dictation.resumeAfterRecording()
        case .commandFailed(let failed) where failed.id != nil && failed.id == ref:
            if let (target, _) = checking {
                messages[target] = Message(text: "Couldn\u{2019}t check that shortcut. The key before still works.", isProblem: true)
            }
            checking = nil
            ref = nil
            dictation.resumeAfterRecording()
        case .coreStopped:
            // Nothing will answer the check now; dictation is turned on again with the core.
            recording = nil
            checking = nil
            ref = nil
        default:
            break
        }
    }

    /// Whether this model shows a failed command itself.
    func handles(_ failed: CommandFailed) -> Bool {
        failed.id?.hasPrefix(Self.refPrefix) ?? false
    }

    /// The two keys are never one: the core would refuse the edit key, and a key that dictates and
    /// edits at once does neither well.
    private func save(_ canonical: String, for target: Target, cap: String) {
        switch target {
        case .dictation:
            if canonical == dictation.editKey {
                messages[target] = Message(text: "\(cap) is the edit key. Pick another, or change the edit key first.", isProblem: true)
                return
            }
            dictation.setKey(canonical)
        case .edit:
            if canonical == dictation.key {
                messages[target] = Message(text: "\(cap) is the dictation key. Pick another.", isProblem: true)
                return
            }
            saveEditKey(canonical)
        }
        messages[target] = KeyNotation.clash(canonical).map { Message(text: $0, isProblem: true) }
    }
}
