// Settings > Snippets and Settings > Voice commands: the lists dictation uses, as the core stores
// them (the user's own, else the ones the Inkwell 0.2 import brought), and what became of 0.2's
// dictation key, said once.
//
// The core is the source of truth: each list arrives whole (snippets.listed,
// voice_commands.listed) and a change sends the whole list back (snippets.save,
// voice_commands.save), which a running dictation takes at once. The row changes at once on screen;
// the core's answer then replaces it. A save that fails says so and reads the list again, so the
// screen never shows what was not saved. Nothing can be changed until a list has been read: a
// stored list the core cannot read is never replaced by an edit, only by "Start over", which the
// user chooses (and the core refuses any other save over it). The user's words travel in these
// lists: never log them.
import Foundation
import InkBridge
import Observation

/// A snippet as Settings edits it.
struct SnippetDraft: Equatable, Identifiable, Sendable {
    var id: String
    var trigger: String
    var expansion: String
    var category: String
    var enabled: Bool

    init(id: String, trigger: String, expansion: String, category: String = "", enabled: Bool = true) {
        self.id = id
        self.trigger = trigger
        self.expansion = expansion
        self.category = category
        self.enabled = enabled
    }

    init(_ info: SnippetInfo) {
        self.init(id: info.id, trigger: info.trigger, expansion: info.expansion, category: info.category, enabled: info.enabled)
    }

    /// Its JSON in `snippets.save`.
    var fields: [String: Any] {
        ["id": id, "trigger": trigger, "expansion": expansion, "category": category, "enabled": enabled]
    }
}

/// A voice command as Settings edits it.
struct VoiceCommandDraft: Equatable, Identifiable, Sendable {
    var id: String
    var triggers: [String]
    var action: CommandAction
    var value: String?
    var enabled: Bool
    /// Whether this build carries it out (the core says; a new one is always of a kind it does).
    var carriedOut: Bool

    init(id: String, triggers: [String], action: CommandAction, value: String?, enabled: Bool = true, carriedOut: Bool = true) {
        self.id = id
        self.triggers = triggers
        self.action = action
        self.value = value
        self.enabled = enabled
        self.carriedOut = carriedOut
    }

    init(_ info: VoiceCommandInfo) {
        self.init(id: info.id, triggers: info.triggers, action: info.action, value: info.value, enabled: info.enabled, carriedOut: info.carriedOut)
    }

    /// Its JSON in `voice_commands.save`.
    var fields: [String: Any] {
        var out: [String: Any] = ["id": id, "triggers": triggers, "action": action.rawValue, "enabled": enabled]
        if let value { out["value"] = value }
        return out
    }
}

/// A new id for a row the user adds.
private func newID(_ prefix: String) -> String {
    "\(prefix)-\(UUID().uuidString.lowercased())"
}

@MainActor
@Observable
final class SnippetsModel {
    private(set) var rows: [SnippetDraft] = []
    /// The list is the one the 0.2 import brought, not yet saved in 1.0.
    private(set) var fromImport = false
    /// The list has been read: until then, and after a read fails, nothing can be changed.
    private(set) var loaded = false
    /// The stored list could not be read: only "Start over" can replace it.
    private(set) var unreadable = false
    /// What went wrong, in words.
    private(set) var failure: String?

    static let loadFailedText = "Couldn\u{2019}t read your snippets."
    static let saveFailedText = "Couldn\u{2019}t save that change. The list shows what is saved."
    static let refPrefix = "snippets:"

    @ObservationIgnored private let send: SendCommand
    @ObservationIgnored private var nextRef = 0
    /// The newest list or save sent: only its answer replaces the rows, so an answer to an earlier
    /// save never shows over a later change for a moment.
    @ObservationIgnored private var latest: String?

    init(send: @escaping SendCommand) {
        self.send = send
    }

    private func ref() -> String {
        nextRef += 1
        let ref = "\(Self.refPrefix)\(nextRef)"
        latest = ref
        return ref
    }

    func load() {
        send(.snippetsList(ref: ref()))
    }

    /// Adds a snippet; a blank trigger adds nothing (the caller keeps Add disabled then).
    func add(trigger: String, expansion: String, category: String) {
        let trigger = trigger.trimmingCharacters(in: .whitespacesAndNewlines)
        guard loaded, !trigger.isEmpty else { return }
        save(rows + [SnippetDraft(id: newID("snip"), trigger: trigger, expansion: expansion, category: category.trimmingCharacters(in: .whitespaces))])
    }

    /// Replaces the snippet with `draft`'s id; a blank trigger changes nothing.
    func update(_ draft: SnippetDraft) {
        var draft = draft
        draft.trigger = draft.trigger.trimmingCharacters(in: .whitespacesAndNewlines)
        guard loaded, !draft.trigger.isEmpty, let i = rows.firstIndex(where: { $0.id == draft.id }) else { return }
        var next = rows
        next[i] = draft
        save(next)
    }

    func setEnabled(_ id: String, _ on: Bool) {
        guard var row = rows.first(where: { $0.id == id }) else { return }
        row.enabled = on
        update(row)
    }

    func delete(_ id: String) {
        guard loaded else { return }
        save(rows.filter { $0.id != id })
    }

    /// Replaces a stored list that cannot be read with an empty one: only when the user chooses it.
    func startOver() {
        guard unreadable else { return }
        send(.snippetsSave([], replaceUnreadable: true, ref: ref()))
    }

    private func save(_ next: [SnippetDraft]) {
        guard loaded else { return }
        rows = next
        failure = nil
        send(.snippetsSave(next, replaceUnreadable: false, ref: ref()))
    }

    func apply(_ event: InkEvent) {
        switch event {
        case .snippetsListed(let listed) where latest == nil || listed.ref == nil || listed.ref == latest:
            rows = listed.snippets.map(SnippetDraft.init)
            fromImport = listed.fromImport
            loaded = true
            unreadable = false
            failure = nil
        case .commandFailed(let failed) where failed.command == "snippets.list" && (failed.id == nil || failed.id == latest):
            // Not known what is stored: nothing is shown, and nothing can be changed.
            rows = []
            loaded = false
            unreadable = true
            failure = Self.loadFailedText
        case .commandFailed(let failed) where failed.command == "snippets.save":
            // The core refused to save over a stored list it cannot read: told apart by the code.
            if failed.code == .listUnreadable {
                // The stored list became unreadable after it was read: say so, and offer Start
                // over, at once (the read below fails the same way).
                rows = []
                loaded = false
                unreadable = true
                failure = Self.loadFailedText
            } else {
                failure = Self.saveFailedText
            }
            send(.snippetsList(ref: ref()))
        default:
            break
        }
    }
}

@MainActor
@Observable
final class VoiceCommandsModel {
    private(set) var enabled = false
    private(set) var wakePrefix = "inkwell"
    private(set) var rows: [VoiceCommandDraft] = []
    private(set) var fromImport = false
    /// As in SnippetsModel.
    private(set) var loaded = false
    private(set) var unreadable = false
    private(set) var failure: String?

    static let loadFailedText = "Couldn\u{2019}t read your voice commands."
    static let saveFailedText = "Couldn\u{2019}t save that change. The list shows what is saved."
    static let refPrefix = "voice_commands:"
    /// The kinds a new command can be: the ones this build carries out, bar polish (which has its
    /// own switch and consent in Settings > AI).
    static let addable: [CommandAction] = [.insertText, .changeStyle]

    @ObservationIgnored private let send: SendCommand
    @ObservationIgnored private var nextRef = 0
    /// The newest list or save sent: only its answer replaces the rows, so an answer to an earlier
    /// save never shows over a later change for a moment.
    @ObservationIgnored private var latest: String?

    init(send: @escaping SendCommand) {
        self.send = send
    }

    private func ref() -> String {
        nextRef += 1
        let ref = "\(Self.refPrefix)\(nextRef)"
        latest = ref
        return ref
    }

    func load() {
        send(.voiceCommandsList(ref: ref()))
    }

    func setEnabled(_ on: Bool) {
        guard loaded else { return }
        save(enabled: on, wakePrefix: wakePrefix, rows: rows)
    }

    /// Replaces stored commands that cannot be read with none, switched off: only when the user
    /// chooses it.
    func startOver() {
        guard unreadable else { return }
        send(.voiceCommandsSave(enabled: false, wakePrefix: "inkwell", commands: [], replaceUnreadable: true, ref: ref()))
    }

    /// A blank wake word changes nothing (it would match nothing).
    func setWakePrefix(_ word: String) {
        let word = word.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        guard loaded, !word.isEmpty, word != wakePrefix else { return }
        save(enabled: enabled, wakePrefix: word, rows: rows)
    }

    func setCommandEnabled(_ id: String, _ on: Bool) {
        guard loaded else { return }
        save(enabled: enabled, wakePrefix: wakePrefix, rows: rows.map { row in
            var row = row
            if row.id == id { row.enabled = on }
            return row
        })
    }

    func delete(_ id: String) {
        guard loaded else { return }
        save(enabled: enabled, wakePrefix: wakePrefix, rows: rows.filter { $0.id != id })
    }

    /// Adds a command: `triggers` comma-separated, `value` the text to type or the style's name.
    /// Nothing is added without a trigger and a value.
    func add(triggers: String, action: CommandAction, value: String) {
        let phrases = Self.phrases(triggers)
        let value = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard loaded, !phrases.isEmpty, !value.isEmpty, Self.addable.contains(action) else { return }
        let row = VoiceCommandDraft(id: newID("custom"), triggers: phrases, action: action, value: value)
        save(enabled: enabled, wakePrefix: wakePrefix, rows: rows + [row])
    }

    /// Comma-separated phrases, trimmed and lowercased, blanks left out.
    static func phrases(_ text: String) -> [String] {
        text.split(separator: ",")
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() }
            .filter { !$0.isEmpty }
    }

    private func save(enabled: Bool, wakePrefix: String, rows: [VoiceCommandDraft]) {
        guard loaded else { return }
        self.enabled = enabled
        self.wakePrefix = wakePrefix
        self.rows = rows
        failure = nil
        send(.voiceCommandsSave(enabled: enabled, wakePrefix: wakePrefix, commands: rows, replaceUnreadable: false, ref: ref()))
    }

    /// What a command does, in words. Its text is shown as text, never as a link.
    static func describe(_ row: VoiceCommandDraft) -> String {
        let value = row.value ?? ""
        return switch row.action {
        case .undo: "Undo the last dictation"
        case .changeStyle: "Write in the \u{201C}\(value)\u{201D} style"
        case .switchModel: "Switch the model to \(value)"
        case .togglePolish: "Turn polish on or off"
        case .toggleDictation: "Pause or resume dictation"
        case .openUrl: "Open \(value)"
        case .openApp: "Open \(value)"
        case .insertText: "Type \u{201C}\(value)\u{201D}"
        case .other: "Something this version doesn\u{2019}t know"
        }
    }

    func apply(_ event: InkEvent) {
        switch event {
        case .voiceCommandsListed(let listed) where latest == nil || listed.ref == nil || listed.ref == latest:
            enabled = listed.enabled
            wakePrefix = listed.wakePrefix
            rows = listed.commands.map(VoiceCommandDraft.init)
            fromImport = listed.fromImport
            loaded = true
            unreadable = false
            failure = nil
        case .commandFailed(let failed) where failed.command == "voice_commands.list" && (failed.id == nil || failed.id == latest):
            rows = []
            loaded = false
            unreadable = true
            failure = Self.loadFailedText
        case .commandFailed(let failed) where failed.command == "voice_commands.save":
            // The core refused to save over a stored list it cannot read: told apart by the code.
            if failed.code == .listUnreadable {
                // The stored list became unreadable after it was read: say so, and offer Start
                // over, at once (the read below fails the same way).
                rows = []
                loaded = false
                unreadable = true
                failure = Self.loadFailedText
            } else {
                failure = Self.saveFailedText
            }
            send(.voiceCommandsList(ref: ref()))
        default:
            break
        }
    }
}

/// What became of Inkwell 0.2's dictation key, said once in Settings > Dictation until dismissed.
@MainActor
@Observable
final class ImportNoteModel {
    private(set) var note: ImportKeyNote?

    @ObservationIgnored private let send: SendCommand

    init(send: @escaping SendCommand) {
        self.send = send
    }

    func load() {
        send(.importNotes)
    }

    /// Read: not said again.
    func dismiss() {
        note = nil
        send(.settingSet(.importKeyNote, "dismissed"))
    }

    func apply(_ event: InkEvent) {
        if case .importNotes(let notes) = event {
            note = notes.key
        }
    }

    /// 0.2's stored hotkey as keys read (`super+shift+space` → ⌘⇧Space); a modifier token by name.
    static func keys(_ hotkey: String) -> String {
        switch hotkey {
        case "fn": return "fn"
        case "right_cmd": return "right \u{2318}"
        case "right_opt": return "right \u{2325}"
        case "right_ctrl": return "right \u{2303}"
        default: break
        }
        let parts = hotkey.split(separator: "+").map { $0.trimmingCharacters(in: .whitespaces).lowercased() }
        guard !parts.isEmpty, !parts.contains(where: \.isEmpty) else { return hotkey }
        return parts.map { part in
            switch part {
            case "super", "cmd", "command", "meta": "\u{2318}"
            case "shift": "\u{21E7}"
            case "alt", "option", "opt": "\u{2325}"
            case "ctrl", "control": "\u{2303}"
            default: part.prefix(1).uppercased() + part.dropFirst()
            }
        }.joined()
    }

    /// The note in words, with the key dictation uses now (`currentKey`, its name).
    static func text(_ note: ImportKeyNote, currentKey: String) -> String {
        let old = keys(note.hotkey)
        var lines: [String] = []
        switch note.outcome {
        case .combination:
            lines.append("Inkwell 0.2 started dictation with \(old), a key combination. Inkwell now listens for one key held on its own, so it uses \(currentKey). Pick another under Dictate if you like.")
        case .otherKey:
            lines.append("Inkwell 0.2\u{2019}s dictation key (\(old)) isn\u{2019}t one Inkwell can listen for now, so it uses \(currentKey). Pick another under Dictate if you like.")
        case .replaced:
            // Only a Windows import replaces a key (Fn never reaches Windows); said as it is there.
            lines.append("Inkwell 0.2 started dictation with \(old), which this computer never sees, so it was replaced by \(currentKey). Pick another under Dictate if you like.")
        case .mapped:
            break
        }
        if note.toggle {
            lines.append("Inkwell 0.2 started and stopped on separate presses. Now you hold \(currentKey) while you speak and let go when you\u{2019}re done.")
        }
        return lines.joined(separator: " ")
    }
}
