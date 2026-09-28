// The commands the screens send (inkwell.h lists them). A view model sends
// these through a closure, so a test can read what it sent and answer with events of its own.
import Foundation
import InkBridge
import os

/// One command to the core.
enum CoreCommand: Equatable, Sendable {
    case permissionsCheck
    case permissionRequest(PermissionName)
    case commitmentsList
    case commitmentSetDone(id: String, done: Bool)
    /// `ref` comes back in `note.added`, so the note can be matched to the line that sent it.
    case noteAdd(record: String, atMs: UInt64, text: String, ref: String)
    /// `ref` comes back in `note.updated` / `note.deleted`, or as the id of a `command.failed`.
    case noteUpdate(note: String, text: String, ref: String)
    case noteDelete(note: String, ref: String)
    case modelsList
    case engineRoute(Job)
    case settingGet(ShellSetting)
    case settingSet(ShellSetting, String)
    case modesList
    /// The library (Today, Library, a record). `ref` comes back as the answer's `ref`, or as the id
    /// of a `command.failed`, so a model matches each answer to its question and can tell "could not
    /// load" from "empty".
    case recordsList(kind: RecordKind?, before: RecordCursor?, limit: Int, ref: String)
    case recordsSearch(query: String, limit: Int, ref: String)
    case recordOpen(record: String, ref: String)
    case libraryStats(sinceUnixMs: Int64, ref: String)
    /// Meetings (S2.8). A start names the app when it answers the Drop's offer, and a title when
    /// the calendar has the call.
    case meetingStart(app: String?, title: String?)
    case meetingStop
    case meetingDismiss(app: String)
    /// `ref` comes back in `meeting.answered`, or as the id of a `command.failed`.
    case meetingAsk(question: String, ref: String)
    case meetingsRecover
    /// "Not yet": a looks-done suggestion is dismissed.
    case commitmentNotYet(id: String)
    /// Dictation live (the core holds the keys): answered by `dictation.ready` or `dictation.off`
    /// with `ref`. `utcOffsetMinutes` is for {date} and {time} in snippets.
    case dictationEnable(utcOffsetMinutes: Int, ref: String)
    /// Lets go of the keys and the mic: `dictation.off` with `ref`.
    case dictationDisable(ref: String)
    /// A feature's switch, where it would send now, and the user's consent: `consent.state` with
    /// `ref`, or `command.failed` with it as the id.
    case consentGet(LlmFeature, ref: String)
    /// The user agreed, in the consent step, that the feature may send to `to` (for a cloud model,
    /// the `endpoint` consent.state named; for voice edit, with its `key`): the core records it and
    /// turns the feature on, or fails if the model has moved since.
    /// `ref` comes back in its `consent.state`, or as the id of a `command.failed`.
    case consentAllow(feature: LlmFeature, to: LlmDestination, endpoint: String?, key: String?, ref: String)
    /// Settings > Snippets and Voice commands: each answered by its `.listed` with `ref`, or a
    /// `command.failed` with that id. A save sends the whole list; the core refuses it over a
    /// stored list it cannot read unless `replaceUnreadable` (the user chose to start over).
    case snippetsList(ref: String)
    case snippetsSave([SnippetDraft], replaceUnreadable: Bool, ref: String)
    case voiceCommandsList(ref: String)
    case voiceCommandsSave(enabled: Bool, wakePrefix: String, commands: [VoiceCommandDraft], replaceUnreadable: Bool, ref: String)
    /// What the Inkwell 0.2 import has to say about the dictation key: `import.notes`.
    case importNotes

    /// Where a page of records continues: the last record of the previous page.
    struct RecordCursor: Equatable, Sendable {
        let startedAtUnixMs: Int64
        let record: String
    }

    /// Its JSON, as `ink_command` reads it.
    var json: String {
        let fields: [String: Any] = switch self {
        case .permissionsCheck: ["cmd": "permissions.check"]
        case .permissionRequest(let p): ["cmd": "permission.request", "permission": p.rawValue]
        case .commitmentsList: ["cmd": "commitments.list"]
        case .commitmentSetDone(let id, let done): ["cmd": "commitment.set_done", "commitment": id, "done": done]
        case .noteAdd(let record, let at, let text, let ref):
            ["cmd": "note.add", "record": record, "at_ms": at, "text": text, "id": ref]
        case .noteUpdate(let note, let text, let ref): ["cmd": "note.update", "note": note, "text": text, "id": ref]
        case .noteDelete(let note, let ref): ["cmd": "note.delete", "note": note, "id": ref]
        case .modelsList: ["cmd": "models.list"]
        case .engineRoute(let job): ["cmd": "engine.route", "job": job.rawValue]
        // The id names the setting, so a failure can be matched to it (command.failed has no key).
        case .settingGet(let key): ["cmd": "setting.get", "key": key.rawValue, "id": "setting:\(key.rawValue)"]
        case .settingSet(let key, let value):
            ["cmd": "setting.set", "key": key.rawValue, "value": value, "id": "setting:\(key.rawValue)"]
        case .modesList: ["cmd": "modes.list"]
        case .recordsList(let kind, let before, let limit, let ref):
            ["cmd": "records.list", "limit": limit, "id": ref]
                .merging(kind.map { ["kind": $0.rawValue] } ?? [:]) { a, _ in a }
                .merging(before.map { ["before": ["started_at_unix_ms": $0.startedAtUnixMs, "id": $0.record]] } ?? [:]) { a, _ in a }
        case .recordsSearch(let query, let limit, let ref): ["cmd": "records.search", "query": query, "limit": limit, "id": ref]
        case .recordOpen(let record, let ref): ["cmd": "record.open", "record": record, "id": ref]
        case .libraryStats(let since, let ref): ["cmd": "library.stats", "since_unix_ms": since, "id": ref]
        case .meetingStart(let app, let title):
            ["cmd": "meeting.start", "id": "meeting.start"]
                .merging(app.map { ["app": $0] } ?? [:]) { a, _ in a }
                .merging(title.map { ["title": $0] } ?? [:]) { a, _ in a }
        case .meetingStop: ["cmd": "meeting.stop", "id": "meeting.stop"]
        case .meetingDismiss(let app): ["cmd": "meeting.dismiss", "app": app, "id": "meeting.dismiss"]
        case .meetingAsk(let question, let ref): ["cmd": "meeting.ask", "question": question, "id": ref]
        case .meetingsRecover: ["cmd": "meetings.recover"]
        case .commitmentNotYet(let id): ["cmd": "commitment.not_yet", "commitment": id]
        case .dictationEnable(let offset, let ref): ["cmd": "dictation.enable", "utc_offset_minutes": offset, "id": ref]
        case .dictationDisable(let ref): ["cmd": "dictation.disable", "id": ref]
        case .consentGet(let feature, let ref):
            ["cmd": "consent.get", "feature": feature.rawValue, "id": ref]
        case .consentAllow(let feature, let to, let endpoint, let key, let ref):
            ["cmd": "consent.allow", "feature": feature.rawValue, "to": to.rawValue, "id": ref]
                .merging(endpoint.map { ["endpoint": $0] } ?? [:]) { a, _ in a }
                .merging(key.map { ["key": $0] } ?? [:]) { a, _ in a }
        case .snippetsList(let ref): ["cmd": "snippets.list", "id": ref]
        case .snippetsSave(let snippets, let replace, let ref):
            ["cmd": "snippets.save", "snippets": snippets.map(\.fields), "id": ref]
                .merging(replace ? ["replace_unreadable": true] : [:]) { a, _ in a }
        case .voiceCommandsList(let ref): ["cmd": "voice_commands.list", "id": ref]
        case .voiceCommandsSave(let enabled, let wakePrefix, let commands, let replace, let ref):
            ["cmd": "voice_commands.save", "enabled": enabled, "wake_prefix": wakePrefix,
             "commands": commands.map(\.fields), "id": ref]
                .merging(replace ? ["replace_unreadable": true] : [:]) { a, _ in a }
        case .importNotes: ["cmd": "import.notes", "id": "import.notes"]
        }
        // Strings, numbers, booleans and objects of them: serialisation cannot fail.
        let data = (try? JSONSerialization.data(withJSONObject: fields, options: [.sortedKeys])) ?? Data("{}".utf8)
        return String(decoding: data, as: UTF8.self)
    }

    /// The command's name, for a log line (never its fields: a note's words travel in them).
    var name: String {
        switch self {
        case .permissionsCheck: "permissions.check"
        case .permissionRequest: "permission.request"
        case .commitmentsList: "commitments.list"
        case .commitmentSetDone: "commitment.set_done"
        case .noteAdd: "note.add"
        case .noteUpdate: "note.update"
        case .noteDelete: "note.delete"
        case .modelsList: "models.list"
        case .engineRoute: "engine.route"
        case .settingGet: "setting.get"
        case .settingSet: "setting.set"
        case .modesList: "modes.list"
        case .recordsList: "records.list"
        case .recordsSearch: "records.search"
        case .recordOpen: "record.open"
        case .libraryStats: "library.stats"
        case .meetingStart: "meeting.start"
        case .meetingStop: "meeting.stop"
        case .meetingDismiss: "meeting.dismiss"
        case .meetingAsk: "meeting.ask"
        case .meetingsRecover: "meetings.recover"
        case .commitmentNotYet: "commitment.not_yet"
        case .dictationEnable: "dictation.enable"
        case .dictationDisable: "dictation.disable"
        case .consentGet: "consent.get"
        case .consentAllow: "consent.allow"
        case .snippetsList: "snippets.list"
        case .snippetsSave: "snippets.save"
        case .voiceCommandsList: "voice_commands.list"
        case .voiceCommandsSave: "voice_commands.save"
        case .importNotes: "import.notes"
        }
    }
}

/// The settings the shell owns in the core's store (`setting.get` / `setting.set`).
enum ShellSetting: String, Sendable {
    /// "true" once the first-run state was completed or skipped.
    case onboardingDone = "onboarding.done"
    /// "on" or "off": the user's switch for dictation polish. Only "off" is set this way: polish
    /// turns on through the consent step (`consentAllow`).
    case dictationPolish = "dictation.polish"
    /// "on" (the default) or "off": listen for calls and offer to record them.
    case meetingsDetect = "meetings.detect"
    /// "on" or "off": the switch for a meeting's summary and Ask. Only "off" is set this way: they
    /// turn on through the consent step (`consentAllow`).
    case meetingsLLM = "meetings.llm"
    /// "on" or "off" (the default): with Bluetooth output, record the headset's own mic.
    case meetingsHeadsetMic = "meetings.headset_mic"
    /// "forever" (the default), or days: how long the library keeps records.
    case retentionDays = "retention.days"
    /// The dictation key (a token: fn, right_option, ...).
    case dictationKey = "dictation.key"
    /// The voice-edit key, or "off" (which withdraws its consent). Turned on (from off) through the
    /// consent step (`consentAllow` with the key).
    case dictationEditKey = "dictation.edit_key"
    /// "on" or "off": whether dictation is live (Settings > Voice). Never set: on.
    case dictationEnabled = "dictation.enabled"
    /// "dismissed": the note about Inkwell 0.2's dictation key has been read.
    case importKeyNote = "import.key_note"
}

/// Where the screens' commands go.
typealias SendCommand = @MainActor (CoreCommand) -> Void

/// Where the shell's diagnostics about commands go: a command's name and what kind of failure,
/// never its fields or anything the user said (a note's words travel in them).
struct ScreenLog: Sendable {
    let write: @Sendable (String) -> Void

    init(_ write: @escaping @Sendable (String) -> Void) {
        self.write = write
    }

    /// The unified log, subsystem com.inkwell.app, category "screens". Every message is built from
    /// command names and fixed words only, so it is logged as public.
    static let system = ScreenLog { message in
        Logger(subsystem: "com.inkwell.app", category: "screens").error("\(message, privacy: .public)")
    }
}
