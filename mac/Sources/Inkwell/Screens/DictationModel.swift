// Dictation as the shell shows it: whether it is live and on which keys (Settings > Voice), the
// choice of keys, and what the Drop says about a take that ended without its text going in.
//
// The core holds the keys and the mic (architecture rule 1). Once the core is ready the shell reads
// the user's switch (dictation.enabled, never set: on) and, unless it is off, sends
// dictation.enable; again when the app becomes active while dictation is off for a reason the user
// fixes in System Settings (Accessibility, a lost key). Turning the switch off sends
// dictation.disable, which lets go of the keys and the mic (quitting needs nothing: the core lets go
// of them when it stops). The keys go through setting.set, and the core rebinds at once and answers
// dictation.ready or dictation.off. Nothing here polls.
import Foundation
import InkBridge
import Observation

/// A key a user can pick.
struct DictationKey: Identifiable, Equatable, Sendable {
    /// The core's token.
    let token: String
    /// Its name, spelled out.
    let name: String
    /// How the key cap reads.
    let cap: String

    var id: String { token }
}

@MainActor
@Observable
final class DictationModel {
    /// Where dictation is.
    enum State: Equatable, Sendable {
        /// Not answered yet.
        case starting
        /// Live: the core holds these keys.
        case live(key: String, editKey: String?)
        /// Not live, and why.
        case off(DictationOffReason, message: String?)
    }

    /// Something the Drop says after a take (or during one), once.
    struct Note: Equatable, Sendable {
        /// Increases with every note, so the same words said twice show twice.
        let serial: Int
        let text: DropText
    }

    /// The keys a user can pick (the core's tokens: modifiers held on their own).
    static let keys: [DictationKey] = [
        DictationKey(token: "fn", name: "fn (Globe)", cap: "fn"),
        DictationKey(token: "right_option", name: "Right Option", cap: "right ⌥"),
        DictationKey(token: "right_command", name: "Right Command", cap: "right ⌘"),
        DictationKey(token: "right_control", name: "Right Control", cap: "right ⌃"),
        DictationKey(token: "right_shift", name: "Right Shift", cap: "right ⇧"),
    ]

    /// A token's key, for showing it.
    static func key(_ token: String?) -> DictationKey? {
        keys.first { $0.token == token }
    }

    private(set) var state: State = .starting
    /// The dictation key the user chose (nil until read; the core's default is fn).
    private(set) var keySetting: String?
    /// The edit key the user chose, or "off".
    private(set) var editKeySetting: String?
    /// Why the chosen edit key is not held.
    private(set) var editKeyProblem: String?
    /// Settings dictation could not read (it runs with their defaults).
    private(set) var settingsProblem: String?
    /// A key setting could not be read or saved.
    private(set) var keyFailure: String?
    /// macOS stopped sending the dictation key (Accessibility revoked, say): not working until
    /// dictation is enabled again.
    private(set) var keyLost = false
    /// The user's switch: nil until read, then whether dictation should be live.
    private(set) var wantsOn: Bool?
    /// A dictation.enable or dictation.disable that failed as a command (never ran, or a bug in the
    /// core stopped it), until the core answers the next one.
    private(set) var commandFailure: CommandFailure?

    enum CommandFailure: Equatable, Sendable {
        case enable
        case disable
    }

    static let keyLostText = "The dictation key stopped working: macOS stopped sending it to Inkwell. Check \u{201C}Type for you\u{201D}, then come back."
    static let editKeyLostText = "The edit key stopped working: macOS stopped sending it to Inkwell. Check \u{201C}Type for you\u{201D}, then come back."
    /// The latest note for the Drop.
    private(set) var note: Note?

    @ObservationIgnored private let send: SendCommand
    @ObservationIgnored private let timeZone: @MainActor () -> TimeZone
    @ObservationIgnored private var serial = 0
    @ObservationIgnored private var nextRef = 0
    /// Whether a language model can rewrite a selection (Polish's engine), for the words of a
    /// failed edit.
    @ObservationIgnored var hasLanguageModel: @MainActor () -> Bool = { false }

    init(send: @escaping SendCommand, timeZone: @escaping @MainActor () -> TimeZone = { .current }) {
        self.send = send
        self.timeZone = timeZone
    }

    /// The ids of this model's commands.
    static let refPrefix = "dictation:"
    static let keySettingID = "setting:\(ShellSetting.dictationKey.rawValue)"
    static let editKeySettingID = "setting:\(ShellSetting.dictationEditKey.rawValue)"
    static let enabledSettingID = "setting:\(ShellSetting.dictationEnabled.rawValue)"

    /// Turns dictation on (or, when on, rebinds its keys).
    func enable() {
        nextRef += 1
        let minutes = timeZone().secondsFromGMT() / 60
        send(.dictationEnable(utcOffsetMinutes: minutes, ref: "\(Self.refPrefix)\(nextRef)"))
    }

    /// Lets go of the keys and the mic.
    func disable() {
        nextRef += 1
        send(.dictationDisable(ref: "\(Self.refPrefix)\(nextRef)"))
    }

    /// Reads the switch (and, unless it is off, then turns dictation on) and the key settings.
    func load() {
        send(.settingGet(.dictationEnabled))
        send(.settingGet(.dictationKey))
        send(.settingGet(.dictationEditKey))
    }

    /// Whether the switch in Settings > Voice reads on.
    var isOn: Bool { wantsOn != false }

    /// The user turned dictation on or off: kept for the next launch, and done now.
    func setOn(_ on: Bool) {
        wantsOn = on
        keyFailure = nil
        send(.settingSet(.dictationEnabled, on ? "on" : "off"))
        if on {
            enable()
        } else {
            disable()
        }
    }

    /// Tries again what failed: turning dictation off if that failed, else turning it on.
    func retry() {
        if commandFailure == .disable {
            disable()
        } else {
            enable()
        }
    }

    /// Back from System Settings, perhaps with Accessibility granted: try again.
    func appBecameActive() {
        guard wantsOn != false else { return }
        if case .off(let reason, _) = state, reason == .needsAccessibility || reason == .keyRefused {
            enable()
        } else if keyLost || editKeyProblem == Self.editKeyLostText {
            enable()
        }
    }

    func setKey(_ token: String) {
        keyFailure = nil
        send(.settingSet(.dictationKey, token))
    }

    /// `nil` turns the edit key off.
    func setEditKey(_ token: String?) {
        keyFailure = nil
        send(.settingSet(.dictationEditKey, token ?? "off"))
    }

    /// The key picked now: the one held, else the one chosen, else the core's default.
    var key: String {
        if case .live(let key, _) = state { return key }
        return keySetting ?? "fn"
    }

    /// The edit key chosen (nil: off). Never the dictation key, as the core refuses it: so the
    /// edit picker's selection is always one of its own options (it offers every key but that one).
    var editKey: String? {
        if case .live(_, let edit) = state, let edit { return edit == key ? nil : edit }
        guard let edit = editKeySetting, edit != "off", edit != key else { return nil }
        return edit
    }

    /// Whether dictation is off for a reason turning it on again may fix (the Voice section offers
    /// that): not for Accessibility, which has its own Allow, nor for an unsupported build.
    var canRetry: Bool {
        if commandFailure != nil { return true }
        if case .off(let reason, _) = state {
            // Off by the user's switch is the switch's to change.
            return [.workerStopped, .failed, .other, .keyRefused].contains(reason)
        }
        return false
    }

    /// The retry button's words.
    var retryTitle: String {
        commandFailure == .disable ? "Try again" : "Turn dictation on"
    }

    /// The line under the keys in Settings.
    var status: String {
        if keyLost { return Self.keyLostText }
        if commandFailure == .disable { return "Dictation couldn't be turned off." }
        switch state {
        case .starting:
            return "Starting…"
        case .live(let key, _):
            let cap = Self.key(key)?.cap ?? key
            return "Hold \(cap), speak, let go."
        case .off(let reason, let message):
            switch reason {
            case .needsAccessibility:
                return "Dictation needs \u{201C}Type for you\u{201D} (Accessibility) to hold its key."
            case .keyRefused:
                return "That key can't be used here\(message.map { ": \($0)" } ?? ".")"
            case .unsupported:
                return "Dictation isn't available in this build."
            case .workerStopped:
                return "Dictation stopped after repeated failures."
            case .disabled:
                return "Dictation is off."
            case .failed, .other:
                return "Dictation couldn't start\(message.map { ": \($0)" } ?? ".")"
            }
        }
    }

    /// Whether the status is a problem to show in the alert colour.
    var isProblem: Bool {
        if case .off(let reason, _) = state { return reason != .disabled }
        return keyFailure != nil || keyLost || editKeyProblem != nil || commandFailure != nil
    }

    private func show(_ text: DropText?) {
        guard let text else { return }
        serial += 1
        note = Note(serial: serial, text: text)
    }

    func apply(_ event: InkEvent) {
        switch event {
        case .coreStopped:
            state = .starting
            wantsOn = nil
        case .dictationReady(let ready):
            state = .live(key: ready.key, editKey: ready.editKey)
            keyLost = false
            commandFailure = nil
            editKeyProblem = ready.editKeyError
            settingsProblem = ready.settingsError
        case .dictationOff(let off):
            state = .off(off.reason, message: off.message)
            keyLost = false
            commandFailure = nil
        case .settingValue(let value) where value.key == ShellSetting.dictationEnabled.rawValue:
            let first = wantsOn == nil
            wantsOn = value.value != "off"
            if first {
                if wantsOn == true {
                    enable()
                } else {
                    state = .off(.disabled, message: nil)
                }
            }
        case .commandFailed(let failed) where failed.id == Self.enabledSettingID:
            if failed.command == "setting.get" {
                keyFailure = "Couldn't read whether dictation is on, so it is on."
                if wantsOn == nil {
                    wantsOn = true
                    enable()
                }
            } else {
                keyFailure = "Couldn't save the switch, so the next launch keeps the one before."
            }
        case .commandFailed(let failed) where failed.id?.hasPrefix(Self.refPrefix) ?? false:
            if failed.command == "dictation.disable" {
                commandFailure = .disable
            } else {
                commandFailure = .enable
                state = .off(.failed, message: nil)
            }
        case .dictationHotkeyLost:
            keyLost = true
            show(DropText(title: "The dictation key stopped working", detail: "Check \u{201C}Type for you\u{201D} in Settings", tone: .alert))
        case .dictationEditHotkeyLost:
            editKeyProblem = Self.editKeyLostText
            show(DropText(title: "The edit key stopped working", detail: "Check \u{201C}Type for you\u{201D} in Settings", tone: .alert))
        case .settingValue(let value) where value.key == ShellSetting.dictationKey.rawValue:
            keySetting = value.value
        case .settingValue(let value) where value.key == ShellSetting.dictationEditKey.rawValue:
            editKeySetting = value.value
        case .commandFailed(let failed) where failed.id == Self.keySettingID || failed.id == Self.editKeySettingID:
            keyFailure = failed.command == "setting.set"
                ? "Couldn't save the key. The one before still works."
                : "Couldn't read your key settings."
        default:
            show(Self.note(for: event, hasLanguageModel: hasLanguageModel()))
        }
    }

    /// Whether this model shows a failed command itself.
    func handles(_ failed: CommandFailed) -> Bool {
        failed.id == Self.keySettingID || failed.id == Self.editKeySettingID || failed.id == Self.enabledSettingID
            || (failed.id?.hasPrefix(Self.refPrefix) ?? false)
    }

    /// What the Drop says for `event`, or nil when it says nothing (the text went in as it should,
    /// or a screen shows it).
    static func note(for event: InkEvent, hasLanguageModel: Bool) -> DropText? {
        switch event {
        case .dictationDiscarded(let discarded):
            switch discarded.reason {
            case .tooShort, .speechTooShort:
                return DropText(title: "Too short", detail: "Try again")
            case .noSpeech:
                return DropText(title: "No speech heard", detail: "Nothing was typed")
            case .silence:
                return DropText(title: "The microphone is silent", detail: "Check \u{201C}Hear you\u{201D} in Settings", tone: .alert)
            case .nothingHeard, .nothingLeft:
                return DropText(title: "Nothing to type", detail: "")
            case .cancelled, .other:
                return nil
            }
        case .dictationFailed(let failed):
            switch failed.stage {
            case .transcription:
                return DropText(title: "Couldn't transcribe that", detail: "Nothing was typed", tone: .alert)
            case .insert:
                return DropText(title: "Couldn't type it here", detail: "Your words are in the Library", tone: .alert)
            case .other:
                return DropText(title: "Dictation failed", detail: "Nothing was typed", tone: .alert)
            }
        case .dictationInserted(let inserted):
            return outcomeNote(inserted.outcome, edit: false)
        case .dictationEdited(let edited):
            return outcomeNote(edited.outcome, edit: true)
        case .dictationEditFailed(let failed):
            switch failed.reason {
            case .noSelection:
                return DropText(title: "Select some text first", detail: "Then hold the edit key and say what to change")
            case .secureInput:
                return DropText(title: "Secure input is on", detail: "The selection was left alone", tone: .alert)
            case .selectionUnreadable:
                return DropText(title: "Couldn't read the selection", detail: "Inkwell needs \u{201C}Type for you\u{201D}", tone: .alert)
            case .transcription:
                return DropText(title: "Couldn't hear the instruction", detail: "The selection was left alone", tone: .alert)
            case .noModel, .model:
                return hasLanguageModel
                    ? DropText(title: "Couldn't rewrite the selection", detail: "It was left alone", tone: .alert)
                    : DropText(title: "Editing needs Apple Intelligence", detail: "The selection was left alone", tone: .alert)
            case .timedOut:
                return DropText(title: "The rewrite took too long", detail: "The selection was left alone", tone: .alert)
            case .insert:
                return DropText(title: "Couldn't replace the selection", detail: "It was left alone", tone: .alert)
            case .other:
                return DropText(title: "The edit failed", detail: "The selection was left alone", tone: .alert)
            }
        case .dictationMicFailed:
            return DropText(title: "Couldn't open the microphone", detail: "Check \u{201C}Hear you\u{201D} in Settings", tone: .alert)
        case .dictationWarningEvent(let warning):
            switch warning.kind {
            case .polishTimedOut:
                return DropText(title: "Polish took too long", detail: "Typed as you said it")
            case .releaseMissed:
                return DropText(title: "Stopped after 3 minutes", detail: "The key's release never arrived")
            // Shown elsewhere (Today's notices, Settings) or nothing the user acts on at once.
            case .vadFailed, .audioLost, .tailCutShort, .focusUnreadable, .polishUnavailable,
                 .polishFailed, .noModeForStyle, .saveFailed, .deletedTextNotScrubbed,
                 .deletedTextScrubbed, .other:
                return nil
            }
        default:
            return nil
        }
    }

    private static func outcomeNote(_ outcome: InsertOutcome, edit: Bool) -> DropText? {
        switch outcome {
        case .blocked:
            return DropText(
                title: "Secure input is on",
                detail: edit ? "The selection was left alone" : "Nothing was typed; your words are in the Library",
                tone: .alert)
        case .insertedClipboardNotRestored:
            return DropText(title: edit ? "Replaced" : "Typed", detail: "Your clipboard couldn't be put back")
        case .pasted, .typed, .other:
            return nil
        }
    }
}
