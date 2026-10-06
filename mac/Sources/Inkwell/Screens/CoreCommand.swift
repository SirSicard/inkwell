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
    /// A registry model's first download (`model.update` with the model as its own next), sent
    /// only when the user pressed Download. `ref` comes back as the id of a `command.failed`; the
    /// download's progress and end are model.update_progress and model.update_finished for it.
    case modelInstall(String, ref: String)
    /// Loads the job's model and keeps it loaded: answered by model.warmed, model.refused or
    /// model.warm_failed.
    case modelWarm(Job)
    case engineRoute(Job)
    case settingGet(ShellSetting)
    case settingSet(ShellSetting, String)
    /// Whether the core can watch a recorded shortcut as a dictation or edit key:
    /// `hotkey.checked` with `ref` (its one spelling, or why not), or `command.failed` with it as
    /// the id. Nothing is stored.
    case hotkeyCheck(binding: String, ref: String)
    case modesList
    /// The library (Today, Library, a record). `ref` comes back as the answer's `ref`, or as the id
    /// of a `command.failed`, so a model matches each answer to its question and can tell "could not
    /// load" from "empty".
    case recordsList(kind: RecordKind?, before: RecordCursor?, limit: Int, ref: String)
    case recordsSearch(query: String, limit: Int, ref: String)
    case recordOpen(record: String, ref: String)
    case libraryStats(sinceUnixMs: Int64, ref: String)
    /// Names a far-end speaker of a record by its diarizer label; an empty name clears it.
    /// Answered by `speaker.named` with `ref`, or a `command.failed` with it as the id.
    case speakerName(record: String, speaker: String, name: String, ref: String)
    /// Deletes a record whole, only after the user confirmed it: answered by `record.deleted` with
    /// `ref`, or a `command.failed` with it as the id (a record still live is refused).
    case recordDelete(record: String, ref: String)
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
    /// Whether Inkwell 0.2's data is on this Mac and not yet imported (the core knows where it
    /// is): `import.checked`, or a `command.failed`, with the command's name as its id.
    case importCheck
    /// Imports it: `import.finished`, or a `command.failed` whose message is words to show.
    case importRun
    /// Settings > AI's own-key language model (CloudModel). Each is answered by `llm.providers`
    /// (Test by `llm.tested`) with `ref`, or a `command.failed` with it as the id. The key travels
    /// once, in `llmKeySave`, to the keychain: never log a command's fields (`name` is safe).
    case llmProviders(ref: String)
    case llmKeySave(provider: String, key: String, ref: String)
    case llmKeyDelete(provider: String, ref: String)
    /// "none" for no provider. `localOnlyOff`: the user's say-so that choosing a provider off this
    /// Mac turns local-only mode off (the core refuses such a choice without it).
    case llmChoose(provider: String, model: String?, baseURL: String?, localOnlyOff: Bool, ref: String)
    case llmTest(ref: String)
    /// The Stats screen's numbers (`stats.counted` with `ref`, or a `command.failed` with it as the
    /// id), counted on the user's calendar: their zone's UTC offsets over time and the ISO weekday
    /// their weeks start on (StatsModel.calendarFields).
    case statsGet(utcOffsets: [UTCOffset], weekStart: Int, ref: String)
    /// The milestones reached since the last check, each reported once ever: `milestones.reached`
    /// with `ref`. Takes stats.get's calendar.
    case milestonesCheck(utcOffsets: [UTCOffset], weekStart: Int, ref: String)
    /// Pauses the streak from today (days without a dictation then don't count against it, for up
    /// to 90 days), or ends the running pause: `stats.counted` with `ref`, or a `command.failed`
    /// with it as the id. Take stats.get's calendar.
    case streakPause(utcOffsets: [UTCOffset], weekStart: Int, ref: String)
    case streakResume(utcOffsets: [UTCOffset], weekStart: Int, ref: String)
    /// Settings > Sound: the microphones, the choice (`audio.input`) and the mic in use, answered
    /// by `audio.devices` with `ref` (then `audio.devices_changed` unasked as devices come and go),
    /// or a `command.failed` with it as the id.
    case audioDevices(ref: String)
    /// The mic test: `audio.test_started`, `audio.test_level` about ten times a second, then
    /// `audio.tested`, all with `ref`; or a `command.failed` with it as the id (`meeting_recording`
    /// while a meeting records). Nothing it hears is kept.
    case audioTest(ref: String)
    /// Ends the running test: its own `audio.tested` (`stopped`), or a `command.failed` with `ref`
    /// when none runs.
    case audioTestStop(ref: String)

    /// A UTC offset from the moment it took effect.
    struct UTCOffset: Equatable, Sendable {
        let fromUnixMs: Int64
        let minutes: Int

        var fields: [String: Any] { ["from_unix_ms": fromUnixMs, "minutes": minutes] }
    }

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
        case .modelInstall(let model, let ref): ["cmd": "model.update", "model": model, "next": model, "id": ref]
        case .modelWarm(let job): ["cmd": "model.warm", "job": job.rawValue, "id": "model.warm:\(job.rawValue)"]
        // The id names the job ("engine.route:dictation_final"), so a failure is matched to its line.
        case .engineRoute(let job): ["cmd": "engine.route", "job": job.rawValue, "id": "engine.route:\(job.rawValue)"]
        // The id names the setting, so a failure can be matched to it (command.failed has no key).
        case .settingGet(let key): ["cmd": "setting.get", "key": key.rawValue, "id": "setting:\(key.rawValue)"]
        case .settingSet(let key, let value):
            ["cmd": "setting.set", "key": key.rawValue, "value": value, "id": "setting:\(key.rawValue)"]
        case .hotkeyCheck(let binding, let ref): ["cmd": "hotkey.check", "binding": binding, "id": ref]
        case .modesList: ["cmd": "modes.list"]
        case .recordsList(let kind, let before, let limit, let ref):
            ["cmd": "records.list", "limit": limit, "id": ref]
                .merging(kind.map { ["kind": $0.rawValue] } ?? [:]) { a, _ in a }
                .merging(before.map { ["before": ["started_at_unix_ms": $0.startedAtUnixMs, "id": $0.record]] } ?? [:]) { a, _ in a }
        case .recordsSearch(let query, let limit, let ref): ["cmd": "records.search", "query": query, "limit": limit, "id": ref]
        case .recordOpen(let record, let ref): ["cmd": "record.open", "record": record, "id": ref]
        case .libraryStats(let since, let ref): ["cmd": "library.stats", "since_unix_ms": since, "id": ref]
        case .speakerName(let record, let speaker, let name, let ref):
            ["cmd": "speaker.name", "record": record, "speaker": speaker, "name": name, "id": ref]
        case .recordDelete(let record, let ref): ["cmd": "record.delete", "record": record, "id": ref]
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
        case .importCheck: ["cmd": "import.check", "id": "import.check"]
        case .importRun: ["cmd": "import.run", "id": "import.run"]
        case .llmProviders(let ref): ["cmd": "llm.providers", "id": ref]
        case .llmKeySave(let provider, let key, let ref): ["cmd": "llm.key.save", "provider": provider, "key": key, "id": ref]
        case .llmKeyDelete(let provider, let ref): ["cmd": "llm.key.delete", "provider": provider, "id": ref]
        case .llmChoose(let provider, let model, let baseURL, let localOnlyOff, let ref):
            ["cmd": "llm.choose", "provider": provider, "id": ref]
                .merging(model.map { ["model": $0] } ?? [:]) { a, _ in a }
                .merging(baseURL.map { ["base_url": $0] } ?? [:]) { a, _ in a }
                .merging(localOnlyOff ? ["local_only": "off"] : [:]) { a, _ in a }
        case .llmTest(let ref): ["cmd": "llm.test", "id": ref]
        case .statsGet(let offsets, let weekStart, let ref):
            ["cmd": "stats.get", "utc_offsets": offsets.map(\.fields), "week_start": weekStart, "id": ref]
        case .milestonesCheck(let offsets, let weekStart, let ref):
            ["cmd": "milestones.check", "utc_offsets": offsets.map(\.fields), "week_start": weekStart, "id": ref]
        case .streakPause(let offsets, let weekStart, let ref):
            ["cmd": "streak.pause", "utc_offsets": offsets.map(\.fields), "week_start": weekStart, "id": ref]
        case .streakResume(let offsets, let weekStart, let ref):
            ["cmd": "streak.resume", "utc_offsets": offsets.map(\.fields), "week_start": weekStart, "id": ref]
        case .audioDevices(let ref): ["cmd": "audio.devices", "id": ref]
        case .audioTest(let ref): ["cmd": "audio.test", "id": ref]
        case .audioTestStop(let ref): ["cmd": "audio.test_stop", "id": ref]
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
        case .modelInstall: "model.update"
        case .modelWarm: "model.warm"
        case .engineRoute: "engine.route"
        case .settingGet: "setting.get"
        case .settingSet: "setting.set"
        case .hotkeyCheck: "hotkey.check"
        case .modesList: "modes.list"
        case .recordsList: "records.list"
        case .recordsSearch: "records.search"
        case .recordOpen: "record.open"
        case .libraryStats: "library.stats"
        case .speakerName: "speaker.name"
        case .recordDelete: "record.delete"
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
        case .importCheck: "import.check"
        case .importRun: "import.run"
        case .llmProviders: "llm.providers"
        case .llmKeySave: "llm.key.save"
        case .llmKeyDelete: "llm.key.delete"
        case .llmChoose: "llm.choose"
        case .llmTest: "llm.test"
        case .statsGet: "stats.get"
        case .milestonesCheck: "milestones.check"
        case .streakPause: "streak.pause"
        case .streakResume: "streak.resume"
        case .audioDevices: "audio.devices"
        case .audioTest: "audio.test"
        case .audioTestStop: "audio.test_stop"
        }
    }

    /// Its "id", when it carries one (answers echo it as `ref`; a `command.failed` as its id).
    var commandID: String? {
        let fields = (try? JSONSerialization.jsonObject(with: Data(json.utf8))) as? [String: Any]
        return fields?["id"] as? String
    }

    /// The `command.failed` the core would have sent had it run this command and failed: the
    /// shell raises it when the command never reached the core (no core, or the core refused to
    /// queue it), so the screen waiting for an answer says it couldn't instead of waiting forever.
    /// `message` names what went wrong, never the command's fields.
    func notSent(_ message: String) -> InkEvent {
        var fields = ["type": "command.failed", "command": name, "message": message]
        fields["id"] = commandID
        // Decoded as the core's own events are, so it reaches the screens the same way. Strings
        // only, with the fields command.failed requires: neither step can fail.
        let data = (try? JSONSerialization.data(withJSONObject: fields, options: [.sortedKeys])) ?? Data("{}".utf8)
        return (try? InkEvent.decode(data)) ?? .undecodable(type: "command.failed", record: nil)
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
    /// The microphone for dictation, meetings and the test: "auto" (the default) or a device's
    /// id from `audio.devices`, which must be connected when it is set (Settings > Sound).
    case audioInput = "audio.input"
    /// "forever" (the default), or days: how long the library keeps records.
    case retentionDays = "retention.days"
    /// The dictation key (a token: fn, right_option, ...).
    case dictationKey = "dictation.key"
    /// The voice-edit key, or "off" (which withdraws its consent). Turned on (from off) through the
    /// consent step (`consentAllow` with the key).
    case dictationEditKey = "dictation.edit_key"
    /// "on" or "off": whether dictation is live (Settings > Dictation). Never set: on.
    case dictationEnabled = "dictation.enabled"
    /// "dismissed": the note about Inkwell 0.2's dictation key has been read.
    case importKeyNote = "import.key_note"
    /// "on" (the default) or "off": local-only mode. While on, no language model off this Mac is
    /// called (Settings > AI).
    case llmLocalOnly = "llm.local_only"
    /// The theme (GlowTheme): "light", "dark" or "system" (the default).
    case appearanceMode = "appearance.mode"
    /// Each mode's dot preset, by id ("indigo" by default).
    case appearanceDotsLight = "appearance.dots.light"
    case appearanceDotsDark = "appearance.dots.dark"
    /// Each mode's own colours: "#rrggbb", or "preset" (the default) for the preset's.
    case appearanceYouLight = "appearance.you.light"
    case appearanceThemLight = "appearance.them.light"
    case appearanceYouDark = "appearance.you.dark"
    case appearanceThemDark = "appearance.them.dark"
    /// "on" (the default) or "off": the window's edge glows while something is live.
    case appearanceEdgeGlow = "appearance.edge_glow"
    /// "system" (the default: Reduce Motion decides) or "still".
    case appearanceMotion = "appearance.motion"
    /// The typing speed the Stats screen measures time saved against: a whole number of words a
    /// minute, 10 to 200 (40 unless set).
    case statsTypingWpm = "stats.typing_wpm"
    /// "on" (the default) or "off": a milestone reached, or a best set, is celebrated.
    case statsCelebrate = "stats.celebrate"
    /// The weekdays the streak rests on: "none" (the default), or ISO weekdays ascending and
    /// comma-separated ("6,7"), never all seven. A rest day neither counts nor breaks a streak.
    case statsRestDays = "stats.rest_days"
    /// "shown" (the default) or "hidden": a hidden streak shows nowhere and celebrates nothing.
    case statsStreak = "stats.streak"
    /// "on" or "off" (the default): the share card may carry the heatmap.
    case statsShareHeatmap = "stats.share_heatmap"
    /// The first day (YYYY-MM-DD) of the week whose review the user dismissed.
    case statsReviewDismissed = "stats.review_dismissed"
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
