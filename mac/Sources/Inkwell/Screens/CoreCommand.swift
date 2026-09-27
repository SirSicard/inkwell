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
        }
    }
}

/// The settings the shell owns in the core's store (`setting.get` / `setting.set`).
enum ShellSetting: String, Sendable {
    /// "true" once the first-run state was completed or skipped.
    case onboardingDone = "onboarding.done"
    /// "on" or "off": the user's wish for dictation polish.
    case dictationPolish = "dictation.polish"
    /// "on" (the default) or "off": listen for calls and offer to record them.
    case meetingsDetect = "meetings.detect"
    /// "on" or "off" (the default): with Bluetooth output, record the headset's own mic.
    case meetingsHeadsetMic = "meetings.headset_mic"
    /// "forever" (the default), or days: how long the library keeps records.
    case retentionDays = "retention.days"
    /// The dictation key (a token: fn, right_option, ...).
    case dictationKey = "dictation.key"
    /// The voice-edit key, or "off".
    case dictationEditKey = "dictation.edit_key"
    /// "on" or "off": whether dictation is live (Settings > Voice). Never set: on.
    case dictationEnabled = "dictation.enabled"
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
