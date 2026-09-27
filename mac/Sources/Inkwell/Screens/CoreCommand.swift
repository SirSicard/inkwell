// The commands the Live, Owed and Settings screens send (inkwell.h lists them). A view model sends
// these through a closure, so a test can read what it sent and answer with events of its own.
import Foundation
import InkBridge

/// One command to the core.
enum CoreCommand: Equatable, Sendable {
    case permissionsCheck
    case permissionRequest(PermissionName)
    case commitmentsList
    case commitmentSetDone(id: String, done: Bool)
    /// `ref` comes back in `note.added`, so the note can be matched to the line that sent it.
    case noteAdd(record: String, atMs: UInt64, text: String, ref: String)
    case noteUpdate(note: String, text: String)
    case noteDelete(note: String)
    case modelsList
    case engineRoute(Job)
    case settingGet(ShellSetting)
    case settingSet(ShellSetting, String)
    case modesList

    /// Its JSON, as `ink_command` reads it.
    var json: String {
        let fields: [String: Any] = switch self {
        case .permissionsCheck: ["cmd": "permissions.check"]
        case .permissionRequest(let p): ["cmd": "permission.request", "permission": p.rawValue]
        case .commitmentsList: ["cmd": "commitments.list"]
        case .commitmentSetDone(let id, let done): ["cmd": "commitment.set_done", "commitment": id, "done": done]
        case .noteAdd(let record, let at, let text, let ref):
            ["cmd": "note.add", "record": record, "at_ms": at, "text": text, "id": ref]
        case .noteUpdate(let note, let text): ["cmd": "note.update", "note": note, "text": text]
        case .noteDelete(let note): ["cmd": "note.delete", "note": note]
        case .modelsList: ["cmd": "models.list"]
        case .engineRoute(let job): ["cmd": "engine.route", "job": job.rawValue]
        case .settingGet(let key): ["cmd": "setting.get", "key": key.rawValue]
        case .settingSet(let key, let value): ["cmd": "setting.set", "key": key.rawValue, "value": value]
        case .modesList: ["cmd": "modes.list"]
        }
        // Strings, numbers and booleans only: serialisation cannot fail.
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
        }
    }
}

/// The settings the shell owns in the core's store (`setting.get` / `setting.set`).
enum ShellSetting: String, Sendable {
    /// "true" once the first-run state was completed or skipped.
    case onboardingDone = "onboarding.done"
    /// "on" or "off": the user's wish for dictation polish.
    case dictationPolish = "dictation.polish"
}

/// Where the screens' commands go.
typealias SendCommand = @MainActor (CoreCommand) -> Void
