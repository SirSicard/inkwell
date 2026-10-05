// Generated from schema/events.schema.json by `cargo run -p ink-ffi --bin ink-schema`.
// Do not edit: change the schema and regenerate. A core test fails while this is stale.

import Foundation

/// Every event the Inkwell core sends a shell through the C ABI
/// (core/crates/ink-ffi/include/inkwell.h): one JSON object, its kind in "type". The Swift
/// types in mac/ are generated from this file by `cargo run -p ink-ffi --bin ink-schema`; the
/// generator reads a subset of JSON Schema (string enums, objects of scalars, arrays and
/// references) and refuses anything else. Optional fields are omitted, never null.
public enum InkEvent: Codable, Sendable, Equatable {
    /// `core.ready`
    case coreReady(CoreReady)
    /// `core.stopped`
    case coreStopped(CoreStopped)
    /// `command.failed`
    case commandFailed(CommandFailed)
    /// `engine.registered`
    case engineRegistered(EngineRegistered)
    /// `engine.unregistered`
    case engineUnregistered(EngineUnregistered)
    /// `engine.routed`
    case engineRouted(EngineRouted)
    /// `model.warmed`
    case modelWarmed(ModelWarmed)
    /// `model.warm_failed`
    case modelWarmFailed(ModelWarmFailed)
    /// `model.refused`
    case modelRefused(ModelRefused)
    /// `model.update_started`
    case modelUpdateStarted(ModelUpdateStarted)
    /// `model.update_progress`
    case modelUpdateProgress(ModelUpdateProgress)
    /// `model.update_finished`
    case modelUpdateFinished(ModelUpdateFinished)
    /// `audio.dropped`
    case audioDropped(AudioDropped)
    /// `dictation.voice_detection`
    case dictationVoiceDetection(DictationVoiceDetection)
    /// `dictation.started`
    case dictationStarted(DictationStarted)
    /// `dictation.short_press_ignored`
    case dictationShortPressIgnored(DictationShortPressIgnored)
    /// `dictation.stopped`
    case dictationStopped(DictationStopped)
    /// `dictation.discarded`
    case dictationDiscarded(DictationDiscarded)
    /// `dictation.command`
    case dictationCommand(DictationCommand)
    /// `dictation.inserted`
    case dictationInserted(DictationInserted)
    /// `dictation.failed`
    case dictationFailed(DictationFailed)
    /// `dictation.warning`
    case dictationWarningEvent(DictationWarningEvent)
    /// `dictation.hotkey_lost`
    case dictationHotkeyLost(DictationHotkeyLost)
    /// `dictation.worker_failed`
    case dictationWorkerFailed(DictationWorkerFailed)
    /// `dictation.partial`
    case dictationPartial(DictationPartial)
    /// `dictation.edited`
    case dictationEdited(DictationEdited)
    /// `dictation.edit_failed`
    case dictationEditFailed(DictationEditFailed)
    /// `dictation.edit_hotkey_lost`
    case dictationEditHotkeyLost(DictationEditHotkeyLost)
    /// `dictation.ready`
    case dictationReady(DictationReady)
    /// `dictation.off`
    case dictationOff(DictationOff)
    /// `dictation.mic_failed`
    case dictationMicFailed(DictationMicFailed)
    /// `meeting.started`
    case meetingStarted(MeetingStarted)
    /// `meeting.far_end_fallback`
    case meetingFarEndFallback(MeetingFarEndFallback)
    /// `meeting.mic_switched`
    case meetingMicSwitched(MeetingMicSwitched)
    /// `meeting.detected`
    case meetingDetected(MeetingDetected)
    /// `meeting.detection_ended`
    case meetingDetectionEnded(MeetingDetectionEnded)
    /// `meeting.detection`
    case meetingDetection(MeetingDetection)
    /// `meetings.calls`
    case meetingsCalls(MeetingsCalls)
    /// `meeting.discarded`
    case meetingDiscarded(MeetingDiscarded)
    /// `meeting.answered`
    case meetingAnswered(MeetingAnswered)
    /// `meeting.recovered`
    case meetingRecovered(MeetingRecovered)
    /// `meetings.recovered`
    case meetingsRecovered(MeetingsRecovered)
    /// `meeting.voice_detection`
    case meetingVoiceDetection(MeetingVoiceDetection)
    /// `meeting.side_state`
    case meetingSideState(MeetingSideState)
    /// `meeting.partial`
    case meetingPartial(MeetingPartial)
    /// `meeting.final`
    case meetingFinal(MeetingFinal)
    /// `meeting.warning`
    case meetingWarningEvent(MeetingWarningEvent)
    /// `meeting.stopped`
    case meetingStopped(MeetingStopped)
    /// `meeting.transcribed`
    case meetingTranscribed(MeetingTranscribed)
    /// `meeting.diarized`
    case meetingDiarized(MeetingDiarized)
    /// `meeting.echo`
    case meetingEcho(MeetingEcho)
    /// `meeting.echo_pass`
    case meetingEchoPass(MeetingEchoPass)
    /// `meeting.removed_as_echo`
    case meetingRemovedAsEcho(MeetingRemovedAsEcho)
    /// `meeting.superseded`
    case meetingSuperseded(MeetingSuperseded)
    /// `meeting.kept_live`
    case meetingKeptLive(MeetingKeptLive)
    /// `meeting.summarized`
    case meetingSummarized(MeetingSummarized)
    /// `meeting.commitments`
    case meetingCommitments(MeetingCommitments)
    /// `meeting.looks_done`
    case meetingLooksDone(MeetingLooksDone)
    /// `meeting.finished`
    case meetingFinished(MeetingFinished)
    /// `meeting.failed`
    case meetingFailed(MeetingFailed)
    /// `meeting.capture_failed`
    case meetingCaptureFailed(MeetingCaptureFailed)
    /// `meeting.worker_failed`
    case meetingWorkerFailed(MeetingWorkerFailed)
    /// `permissions.checked`
    case permissionsChecked(PermissionsChecked)
    /// `permission.requested`
    case permissionRequested(PermissionRequested)
    /// `commitments.listed`
    case commitmentsListed(CommitmentsListed)
    /// `commitment.updated`
    case commitmentUpdated(CommitmentUpdated)
    /// `note.added`
    case noteAdded(NoteAdded)
    /// `note.updated`
    case noteUpdated(NoteUpdated)
    /// `note.deleted`
    case noteDeleted(NoteDeleted)
    /// `speaker.named`
    case speakerNamed(SpeakerNamed)
    /// `record.deleted`
    case recordDeleted(RecordDeleted)
    /// `models.listed`
    case modelsListed(ModelsListed)
    /// `setting.value`
    case settingValue(SettingValue)
    /// `audio.devices`
    case audioDevices(AudioDevices)
    /// `audio.devices_changed`
    case audioDevicesChanged(AudioDevicesChanged)
    /// `audio.input_fallback`
    case audioInputFallback(AudioInputFallback)
    /// `audio.test_started`
    case audioTestStarted(AudioTestStarted)
    /// `audio.test_level`
    case audioTestLevel(AudioTestLevel)
    /// `audio.tested`
    case audioTested(AudioTested)
    /// `hotkey.checked`
    case hotkeyChecked(HotkeyChecked)
    /// `consent.state`
    case consentState(ConsentState)
    /// `llm.providers`
    case llmProviders(LlmProviders)
    /// `llm.tested`
    case llmTested(LlmTested)
    /// `modes.listed`
    case modesListed(ModesListed)
    /// `snippets.listed`
    case snippetsListed(SnippetsListed)
    /// `voice_commands.listed`
    case voiceCommandsListed(VoiceCommandsListed)
    /// `import.notes`
    case importNotes(ImportNotes)
    /// `import.checked`
    case importChecked(ImportChecked)
    /// `import.finished`
    case importFinished(ImportFinished)
    /// `library.records`
    case libraryRecords(LibraryRecords)
    /// `library.search`
    case librarySearch(LibrarySearch)
    /// `library.record`
    case libraryRecord(LibraryRecord)
    /// `library.stats`
    case libraryStats(LibraryStats)
    /// `library.swept`
    case librarySwept(LibrarySwept)
    /// `stats.counted`
    case statsCounted(StatsCounted)
    /// `milestones.reached`
    case milestonesReached(MilestonesReached)
    /// An event this build does not know. The core and the shell ship together, so this
    /// means a mismatched build.
    case unknown(type: String)
    /// A known event whose content did not decode (a value this build does not know). Its
    /// type and record are kept so the shell can still tell what it was about.
    case undecodable(type: String, record: String?)

    private enum TypeKey: String, CodingKey {
        case type
        case record
    }

    /// Decodes one event: the JSON an `InkEventCallback` receives. Throws only when it has no
    /// string "type".
    public static func decode(_ json: Data) throws -> InkEvent {
        try JSONDecoder().decode(InkEvent.self, from: json)
    }

    public init(from decoder: Decoder) throws {
        let keys = try decoder.container(keyedBy: TypeKey.self)
        let type = try keys.decode(String.self, forKey: .type)
        do {
            switch type {
            case "core.ready": self = .coreReady(try CoreReady(from: decoder))
            case "core.stopped": self = .coreStopped(try CoreStopped(from: decoder))
            case "command.failed": self = .commandFailed(try CommandFailed(from: decoder))
            case "engine.registered": self = .engineRegistered(try EngineRegistered(from: decoder))
            case "engine.unregistered": self = .engineUnregistered(try EngineUnregistered(from: decoder))
            case "engine.routed": self = .engineRouted(try EngineRouted(from: decoder))
            case "model.warmed": self = .modelWarmed(try ModelWarmed(from: decoder))
            case "model.warm_failed": self = .modelWarmFailed(try ModelWarmFailed(from: decoder))
            case "model.refused": self = .modelRefused(try ModelRefused(from: decoder))
            case "model.update_started": self = .modelUpdateStarted(try ModelUpdateStarted(from: decoder))
            case "model.update_progress": self = .modelUpdateProgress(try ModelUpdateProgress(from: decoder))
            case "model.update_finished": self = .modelUpdateFinished(try ModelUpdateFinished(from: decoder))
            case "audio.dropped": self = .audioDropped(try AudioDropped(from: decoder))
            case "dictation.voice_detection": self = .dictationVoiceDetection(try DictationVoiceDetection(from: decoder))
            case "dictation.started": self = .dictationStarted(try DictationStarted(from: decoder))
            case "dictation.short_press_ignored": self = .dictationShortPressIgnored(try DictationShortPressIgnored(from: decoder))
            case "dictation.stopped": self = .dictationStopped(try DictationStopped(from: decoder))
            case "dictation.discarded": self = .dictationDiscarded(try DictationDiscarded(from: decoder))
            case "dictation.command": self = .dictationCommand(try DictationCommand(from: decoder))
            case "dictation.inserted": self = .dictationInserted(try DictationInserted(from: decoder))
            case "dictation.failed": self = .dictationFailed(try DictationFailed(from: decoder))
            case "dictation.warning": self = .dictationWarningEvent(try DictationWarningEvent(from: decoder))
            case "dictation.hotkey_lost": self = .dictationHotkeyLost(try DictationHotkeyLost(from: decoder))
            case "dictation.worker_failed": self = .dictationWorkerFailed(try DictationWorkerFailed(from: decoder))
            case "dictation.partial": self = .dictationPartial(try DictationPartial(from: decoder))
            case "dictation.edited": self = .dictationEdited(try DictationEdited(from: decoder))
            case "dictation.edit_failed": self = .dictationEditFailed(try DictationEditFailed(from: decoder))
            case "dictation.edit_hotkey_lost": self = .dictationEditHotkeyLost(try DictationEditHotkeyLost(from: decoder))
            case "dictation.ready": self = .dictationReady(try DictationReady(from: decoder))
            case "dictation.off": self = .dictationOff(try DictationOff(from: decoder))
            case "dictation.mic_failed": self = .dictationMicFailed(try DictationMicFailed(from: decoder))
            case "meeting.started": self = .meetingStarted(try MeetingStarted(from: decoder))
            case "meeting.far_end_fallback": self = .meetingFarEndFallback(try MeetingFarEndFallback(from: decoder))
            case "meeting.mic_switched": self = .meetingMicSwitched(try MeetingMicSwitched(from: decoder))
            case "meeting.detected": self = .meetingDetected(try MeetingDetected(from: decoder))
            case "meeting.detection_ended": self = .meetingDetectionEnded(try MeetingDetectionEnded(from: decoder))
            case "meeting.detection": self = .meetingDetection(try MeetingDetection(from: decoder))
            case "meetings.calls": self = .meetingsCalls(try MeetingsCalls(from: decoder))
            case "meeting.discarded": self = .meetingDiscarded(try MeetingDiscarded(from: decoder))
            case "meeting.answered": self = .meetingAnswered(try MeetingAnswered(from: decoder))
            case "meeting.recovered": self = .meetingRecovered(try MeetingRecovered(from: decoder))
            case "meetings.recovered": self = .meetingsRecovered(try MeetingsRecovered(from: decoder))
            case "meeting.voice_detection": self = .meetingVoiceDetection(try MeetingVoiceDetection(from: decoder))
            case "meeting.side_state": self = .meetingSideState(try MeetingSideState(from: decoder))
            case "meeting.partial": self = .meetingPartial(try MeetingPartial(from: decoder))
            case "meeting.final": self = .meetingFinal(try MeetingFinal(from: decoder))
            case "meeting.warning": self = .meetingWarningEvent(try MeetingWarningEvent(from: decoder))
            case "meeting.stopped": self = .meetingStopped(try MeetingStopped(from: decoder))
            case "meeting.transcribed": self = .meetingTranscribed(try MeetingTranscribed(from: decoder))
            case "meeting.diarized": self = .meetingDiarized(try MeetingDiarized(from: decoder))
            case "meeting.echo": self = .meetingEcho(try MeetingEcho(from: decoder))
            case "meeting.echo_pass": self = .meetingEchoPass(try MeetingEchoPass(from: decoder))
            case "meeting.removed_as_echo": self = .meetingRemovedAsEcho(try MeetingRemovedAsEcho(from: decoder))
            case "meeting.superseded": self = .meetingSuperseded(try MeetingSuperseded(from: decoder))
            case "meeting.kept_live": self = .meetingKeptLive(try MeetingKeptLive(from: decoder))
            case "meeting.summarized": self = .meetingSummarized(try MeetingSummarized(from: decoder))
            case "meeting.commitments": self = .meetingCommitments(try MeetingCommitments(from: decoder))
            case "meeting.looks_done": self = .meetingLooksDone(try MeetingLooksDone(from: decoder))
            case "meeting.finished": self = .meetingFinished(try MeetingFinished(from: decoder))
            case "meeting.failed": self = .meetingFailed(try MeetingFailed(from: decoder))
            case "meeting.capture_failed": self = .meetingCaptureFailed(try MeetingCaptureFailed(from: decoder))
            case "meeting.worker_failed": self = .meetingWorkerFailed(try MeetingWorkerFailed(from: decoder))
            case "permissions.checked": self = .permissionsChecked(try PermissionsChecked(from: decoder))
            case "permission.requested": self = .permissionRequested(try PermissionRequested(from: decoder))
            case "commitments.listed": self = .commitmentsListed(try CommitmentsListed(from: decoder))
            case "commitment.updated": self = .commitmentUpdated(try CommitmentUpdated(from: decoder))
            case "note.added": self = .noteAdded(try NoteAdded(from: decoder))
            case "note.updated": self = .noteUpdated(try NoteUpdated(from: decoder))
            case "note.deleted": self = .noteDeleted(try NoteDeleted(from: decoder))
            case "speaker.named": self = .speakerNamed(try SpeakerNamed(from: decoder))
            case "record.deleted": self = .recordDeleted(try RecordDeleted(from: decoder))
            case "models.listed": self = .modelsListed(try ModelsListed(from: decoder))
            case "setting.value": self = .settingValue(try SettingValue(from: decoder))
            case "audio.devices": self = .audioDevices(try AudioDevices(from: decoder))
            case "audio.devices_changed": self = .audioDevicesChanged(try AudioDevicesChanged(from: decoder))
            case "audio.input_fallback": self = .audioInputFallback(try AudioInputFallback(from: decoder))
            case "audio.test_started": self = .audioTestStarted(try AudioTestStarted(from: decoder))
            case "audio.test_level": self = .audioTestLevel(try AudioTestLevel(from: decoder))
            case "audio.tested": self = .audioTested(try AudioTested(from: decoder))
            case "hotkey.checked": self = .hotkeyChecked(try HotkeyChecked(from: decoder))
            case "consent.state": self = .consentState(try ConsentState(from: decoder))
            case "llm.providers": self = .llmProviders(try LlmProviders(from: decoder))
            case "llm.tested": self = .llmTested(try LlmTested(from: decoder))
            case "modes.listed": self = .modesListed(try ModesListed(from: decoder))
            case "snippets.listed": self = .snippetsListed(try SnippetsListed(from: decoder))
            case "voice_commands.listed": self = .voiceCommandsListed(try VoiceCommandsListed(from: decoder))
            case "import.notes": self = .importNotes(try ImportNotes(from: decoder))
            case "import.checked": self = .importChecked(try ImportChecked(from: decoder))
            case "import.finished": self = .importFinished(try ImportFinished(from: decoder))
            case "library.records": self = .libraryRecords(try LibraryRecords(from: decoder))
            case "library.search": self = .librarySearch(try LibrarySearch(from: decoder))
            case "library.record": self = .libraryRecord(try LibraryRecord(from: decoder))
            case "library.stats": self = .libraryStats(try LibraryStats(from: decoder))
            case "library.swept": self = .librarySwept(try LibrarySwept(from: decoder))
            case "stats.counted": self = .statsCounted(try StatsCounted(from: decoder))
            case "milestones.reached": self = .milestonesReached(try MilestonesReached(from: decoder))
            default: self = .unknown(type: type)
            }
        } catch {
            self = .undecodable(type: type, record: try? keys.decode(String.self, forKey: .record))
        }
    }

    public func encode(to encoder: Encoder) throws {
        switch self {
        case .coreReady(let event): try event.encode(to: encoder)
        case .coreStopped(let event): try event.encode(to: encoder)
        case .commandFailed(let event): try event.encode(to: encoder)
        case .engineRegistered(let event): try event.encode(to: encoder)
        case .engineUnregistered(let event): try event.encode(to: encoder)
        case .engineRouted(let event): try event.encode(to: encoder)
        case .modelWarmed(let event): try event.encode(to: encoder)
        case .modelWarmFailed(let event): try event.encode(to: encoder)
        case .modelRefused(let event): try event.encode(to: encoder)
        case .modelUpdateStarted(let event): try event.encode(to: encoder)
        case .modelUpdateProgress(let event): try event.encode(to: encoder)
        case .modelUpdateFinished(let event): try event.encode(to: encoder)
        case .audioDropped(let event): try event.encode(to: encoder)
        case .dictationVoiceDetection(let event): try event.encode(to: encoder)
        case .dictationStarted(let event): try event.encode(to: encoder)
        case .dictationShortPressIgnored(let event): try event.encode(to: encoder)
        case .dictationStopped(let event): try event.encode(to: encoder)
        case .dictationDiscarded(let event): try event.encode(to: encoder)
        case .dictationCommand(let event): try event.encode(to: encoder)
        case .dictationInserted(let event): try event.encode(to: encoder)
        case .dictationFailed(let event): try event.encode(to: encoder)
        case .dictationWarningEvent(let event): try event.encode(to: encoder)
        case .dictationHotkeyLost(let event): try event.encode(to: encoder)
        case .dictationWorkerFailed(let event): try event.encode(to: encoder)
        case .dictationPartial(let event): try event.encode(to: encoder)
        case .dictationEdited(let event): try event.encode(to: encoder)
        case .dictationEditFailed(let event): try event.encode(to: encoder)
        case .dictationEditHotkeyLost(let event): try event.encode(to: encoder)
        case .dictationReady(let event): try event.encode(to: encoder)
        case .dictationOff(let event): try event.encode(to: encoder)
        case .dictationMicFailed(let event): try event.encode(to: encoder)
        case .meetingStarted(let event): try event.encode(to: encoder)
        case .meetingFarEndFallback(let event): try event.encode(to: encoder)
        case .meetingMicSwitched(let event): try event.encode(to: encoder)
        case .meetingDetected(let event): try event.encode(to: encoder)
        case .meetingDetectionEnded(let event): try event.encode(to: encoder)
        case .meetingDetection(let event): try event.encode(to: encoder)
        case .meetingsCalls(let event): try event.encode(to: encoder)
        case .meetingDiscarded(let event): try event.encode(to: encoder)
        case .meetingAnswered(let event): try event.encode(to: encoder)
        case .meetingRecovered(let event): try event.encode(to: encoder)
        case .meetingsRecovered(let event): try event.encode(to: encoder)
        case .meetingVoiceDetection(let event): try event.encode(to: encoder)
        case .meetingSideState(let event): try event.encode(to: encoder)
        case .meetingPartial(let event): try event.encode(to: encoder)
        case .meetingFinal(let event): try event.encode(to: encoder)
        case .meetingWarningEvent(let event): try event.encode(to: encoder)
        case .meetingStopped(let event): try event.encode(to: encoder)
        case .meetingTranscribed(let event): try event.encode(to: encoder)
        case .meetingDiarized(let event): try event.encode(to: encoder)
        case .meetingEcho(let event): try event.encode(to: encoder)
        case .meetingEchoPass(let event): try event.encode(to: encoder)
        case .meetingRemovedAsEcho(let event): try event.encode(to: encoder)
        case .meetingSuperseded(let event): try event.encode(to: encoder)
        case .meetingKeptLive(let event): try event.encode(to: encoder)
        case .meetingSummarized(let event): try event.encode(to: encoder)
        case .meetingCommitments(let event): try event.encode(to: encoder)
        case .meetingLooksDone(let event): try event.encode(to: encoder)
        case .meetingFinished(let event): try event.encode(to: encoder)
        case .meetingFailed(let event): try event.encode(to: encoder)
        case .meetingCaptureFailed(let event): try event.encode(to: encoder)
        case .meetingWorkerFailed(let event): try event.encode(to: encoder)
        case .permissionsChecked(let event): try event.encode(to: encoder)
        case .permissionRequested(let event): try event.encode(to: encoder)
        case .commitmentsListed(let event): try event.encode(to: encoder)
        case .commitmentUpdated(let event): try event.encode(to: encoder)
        case .noteAdded(let event): try event.encode(to: encoder)
        case .noteUpdated(let event): try event.encode(to: encoder)
        case .noteDeleted(let event): try event.encode(to: encoder)
        case .speakerNamed(let event): try event.encode(to: encoder)
        case .recordDeleted(let event): try event.encode(to: encoder)
        case .modelsListed(let event): try event.encode(to: encoder)
        case .settingValue(let event): try event.encode(to: encoder)
        case .audioDevices(let event): try event.encode(to: encoder)
        case .audioDevicesChanged(let event): try event.encode(to: encoder)
        case .audioInputFallback(let event): try event.encode(to: encoder)
        case .audioTestStarted(let event): try event.encode(to: encoder)
        case .audioTestLevel(let event): try event.encode(to: encoder)
        case .audioTested(let event): try event.encode(to: encoder)
        case .hotkeyChecked(let event): try event.encode(to: encoder)
        case .consentState(let event): try event.encode(to: encoder)
        case .llmProviders(let event): try event.encode(to: encoder)
        case .llmTested(let event): try event.encode(to: encoder)
        case .modesListed(let event): try event.encode(to: encoder)
        case .snippetsListed(let event): try event.encode(to: encoder)
        case .voiceCommandsListed(let event): try event.encode(to: encoder)
        case .importNotes(let event): try event.encode(to: encoder)
        case .importChecked(let event): try event.encode(to: encoder)
        case .importFinished(let event): try event.encode(to: encoder)
        case .libraryRecords(let event): try event.encode(to: encoder)
        case .librarySearch(let event): try event.encode(to: encoder)
        case .libraryRecord(let event): try event.encode(to: encoder)
        case .libraryStats(let event): try event.encode(to: encoder)
        case .librarySwept(let event): try event.encode(to: encoder)
        case .statsCounted(let event): try event.encode(to: encoder)
        case .milestonesReached(let event): try event.encode(to: encoder)
        case .unknown(let type):
            var keys = encoder.container(keyedBy: TypeKey.self)
            try keys.encode(type, forKey: .type)
        case .undecodable(let type, let record):
            var keys = encoder.container(keyedBy: TypeKey.self)
            try keys.encode(type, forKey: .type)
            try keys.encodeIfPresent(record, forKey: .record)
        }
    }
}

/// One chunk file of a record's audio: raw little-endian float samples, interleaved, after a
/// header of data_offset bytes.
public struct AudioChunk: Codable, Sendable, Equatable {
    /// Which side.
    public let channel: Channel
    /// Interleaved channels per frame.
    public let channels: Int64
    /// Bytes before the first sample.
    public let dataOffset: Int64
    /// Whole frames in it.
    public let frames: Int64
    /// The file, absolute.
    public let path: String
    /// Frames per second.
    public let sampleRate: Int64
    /// Where its first frame falls on the record's timeline, ms (negative: before the start).
    public let startMs: Int64

    private enum CodingKeys: String, CodingKey {
        case channel
        case channels
        case dataOffset = "data_offset"
        case frames
        case path
        case sampleRate = "sample_rate"
        case startMs = "start_ms"
    }
}

/// A connected input or output device.
public struct AudioDevice: Codable, Sendable, Equatable {
    /// The OS's id for it (a Core Audio UID, a WASAPI endpoint id): what audio.input and
    /// audio.output take. Opaque: never shown.
    public let id: String
    /// Whether it is the system default for its direction.
    public let isDefault: Bool
    /// Its name as the OS shows it.
    public let name: String
    /// How it connects.
    public let transport: MicTransport

    private enum CodingKeys: String, CodingKey {
        case id
        case isDefault = "is_default"
        case name
        case transport
    }
}

/// The answer to audio.devices: the devices, the user's choice, and what Inkwell records with
/// now.
public struct AudioDevices: Codable, Sendable, Equatable {
    /// What Automatic records now ("Automatic (<name>)"); absent when there is no microphone.
    public let automatic: AudioInput?
    /// The mic choice (audio.input): auto, or the chosen device's id.
    public let input: String
    /// The connected microphones, the default first.
    public let inputs: [AudioDevice]
    /// The output choice (audio.output), with outputs: default, or the chosen device's id.
    public let output: String?
    /// The output a meeting's far end is to record, and why; absent when there is no output.
    /// Stored and shown now; meetings record it once the Windows far end is pinned to it, and
    /// until then follow the default output.
    public let outputUsing: AudioOutput?
    /// The chosen output as remembered, when output is a device.
    public let outputWanted: AudioWanted?
    /// The connected outputs, the default first, where there is an output picker (Windows);
    /// absent on macOS, whose far end is tapped from its app wherever it plays.
    public let outputs: [AudioDevice]?
    /// The id of the command this answers, when it carried one.
    public let ref: String?
    /// Always `audio.devices`.
    public let type: String
    /// The mic Inkwell opens now for a take, a meeting or a test, and why: the chosen one, or
    /// Automatic (chosen_missing when it stands in for a chosen mic that is not connected).
    /// Absent when there is no microphone. A meeting already recording keeps its own mic
    /// (meeting.started, meeting.mic_switched).
    public let using: AudioInput?
    /// The chosen mic as remembered, when input is a device: Settings shows its name even while
    /// it is not connected.
    public let wanted: AudioWanted?

    private enum CodingKeys: String, CodingKey {
        case automatic
        case input
        case inputs
        case output
        case outputUsing = "output_using"
        case outputWanted = "output_wanted"
        case outputs
        case ref
        case type
        case using
        case wanted
    }
}

/// Devices came or went, a default changed, or the choice did: the same as audio.devices, once
/// a burst of changes has gone quiet (300 ms after the last, at most 1 s after the first). Sent
/// after a setting.set of audio.input or audio.output, and on a device change where the
/// platform tells the core of them.
public struct AudioDevicesChanged: Codable, Sendable, Equatable {
    /// What Automatic records now ("Automatic (<name>)"); absent when there is no microphone.
    public let automatic: AudioInput?
    /// The mic choice (audio.input): auto, or the chosen device's id.
    public let input: String
    /// The connected microphones, the default first.
    public let inputs: [AudioDevice]
    /// The output choice (audio.output), with outputs: default, or the chosen device's id.
    public let output: String?
    /// The output a meeting's far end is to record, and why; absent when there is no output.
    /// Stored and shown now; meetings record it once the Windows far end is pinned to it, and
    /// until then follow the default output.
    public let outputUsing: AudioOutput?
    /// The chosen output as remembered, when output is a device.
    public let outputWanted: AudioWanted?
    /// The connected outputs, the default first, where there is an output picker (Windows);
    /// absent on macOS, whose far end is tapped from its app wherever it plays.
    public let outputs: [AudioDevice]?
    /// Always `audio.devices_changed`.
    public let type: String
    /// The mic Inkwell opens now for a take, a meeting or a test, and why: the chosen one, or
    /// Automatic (chosen_missing when it stands in for a chosen mic that is not connected).
    /// Absent when there is no microphone. A meeting already recording keeps its own mic
    /// (meeting.started, meeting.mic_switched).
    public let using: AudioInput?
    /// The chosen mic as remembered, when input is a device: Settings shows its name even while
    /// it is not connected.
    public let wanted: AudioWanted?

    private enum CodingKeys: String, CodingKey {
        case automatic
        case input
        case inputs
        case output
        case outputUsing = "output_using"
        case outputWanted = "output_wanted"
        case outputs
        case type
        case using
        case wanted
    }
}

/// The pump dropped capture blocks because a chain's queue was full (the chain fell behind).
/// Sent once per stretch, when the queue takes audio again or closes.
public struct AudioDropped: Codable, Sendable, Equatable {
    /// Blocks dropped.
    public let blocks: Int64
    /// Which chain.
    public let chain: Chain
    /// The side, for dictation (the mic). Absent for a meeting, whose two sides share one
    /// queue.
    public let channel: Channel?
    /// 16 kHz samples in them.
    public let samples: Int64
    /// Always `audio.dropped`.
    public let type: String
}

/// A microphone Inkwell picks, and why.
public struct AudioInput: Codable, Sendable, Equatable {
    /// The OS's id for it.
    public let id: String
    /// Its name as the OS shows it.
    public let name: String
    /// Why it is the one.
    public let reason: MicReason
    /// How it connects.
    public let transport: MicTransport
}

/// The mic the user chose is not connected, and a mic just opened on Automatic in its place (a
/// take, a meeting, a meeting's mic that went, a test): said once until the chosen mic is seen
/// again or the choice changes. The shell says "<wanted> isn't connected. Inkwell is using
/// <mic> until it is."
public struct AudioInputFallback: Codable, Sendable, Equatable {
    /// The mic recording instead, as the OS names it.
    public let micName: String
    /// How that mic connects.
    public let micTransport: MicTransport
    /// Always `audio.input_fallback`.
    public let type: String
    /// The chosen mic.
    public let wanted: AudioWanted

    private enum CodingKeys: String, CodingKey {
        case micName = "mic_name"
        case micTransport = "mic_transport"
        case type
        case wanted
    }
}

/// The output a meeting's far end is to record (Windows), and why. Stored and shown now;
/// meetings record it once the Windows far end is pinned to it, and until then follow the
/// default output.
public struct AudioOutput: Codable, Sendable, Equatable {
    /// The OS's id for it.
    public let id: String
    /// Its name as the OS shows it.
    public let name: String
    /// Why it is the one.
    public let reason: OutputReason
    /// How it connects.
    public let transport: MicTransport
}

/// How a mic test ended: its time was up (done), audio.test_stop or the core's shutdown ended
/// it (stopped), a meeting started recording (meeting), or the mic failed or went away (failed,
/// with a message).
public enum AudioTestEnd: String, Codable, Sendable, Equatable, CaseIterable {
    case done
    case stopped
    case meeting
    case failed
}

/// The mic test's level over the last 100 ms: the loudest moment, from the ink's band analyzer
/// with no gain applied, on a meter scale.
public struct AudioTestLevel: Codable, Sendable, Equatable {
    /// 0 at -60 dBFS and below, 1 at full scale, linear in dB between.
    public let level: Double
    /// The id of the command this answers, when it carried one.
    public let ref: String?
    /// Always `audio.test_level`.
    public let type: String
}

/// A mic test has opened the mic (audio.test): the one dictation and meetings would use now.
/// audio.test_level follows about ten times a second, then audio.tested.
public struct AudioTestStarted: Codable, Sendable, Equatable {
    /// The mic, as the OS names it.
    public let micName: String
    /// Why that mic.
    public let micReason: MicReason
    /// How it connects.
    public let micTransport: MicTransport
    /// The id of the command this answers, when it carried one.
    public let ref: String?
    /// How long the test runs at most, in seconds.
    public let seconds: Int64
    /// Always `audio.test_started`.
    public let type: String

    private enum CodingKeys: String, CodingKey {
        case micName = "mic_name"
        case micReason = "mic_reason"
        case micTransport = "mic_transport"
        case ref
        case seconds
        case type
    }
}

/// The mic test is over. Nothing it heard was kept.
public struct AudioTested: Codable, Sendable, Equatable {
    /// How it ended.
    public let ended: AudioTestEnd
    /// Whether the mic heard anything louder than a quiet room (-50 dBFS) at any moment: false
    /// is the screen's cue for "Not hearing you?".
    public let heard: Bool
    /// Why it failed, naming the device, when ended is failed.
    public let message: String?
    /// The loudest moment, on audio.test_level's scale.
    public let peak: Double
    /// The id of the command this answers, when it carried one.
    public let ref: String?
    /// Always `audio.tested`.
    public let type: String
}

/// How a record's chunks were placed on its timeline: recorded (from the start the meeting
/// wrote beside them) or estimated (from its earliest chunk, because that start is missing: an
/// older record, or one whose write failed). Estimated: the two sides may be out of step, and
/// the shell says so.
public enum AudioTimeline: String, Codable, Sendable, Equatable, CaseIterable {
    case recorded
    case estimated
}

/// A device the user chose, as the core remembers it from when it was chosen (it may not be
/// connected now).
public struct AudioWanted: Codable, Sendable, Equatable {
    /// The OS's id for it.
    public let id: String
    /// Its name when it was chosen; absent for a choice stored without one.
    public let name: String?
    /// How it connected when it was chosen; absent likewise.
    public let transport: MicTransport?
}

/// A personal best: the dictation held longest, the fastest held at least 30 s (neither a take
/// the stuck-key watchdog stopped), the most words in a day, the best week, the longest
/// meeting, the longest monologue.
public enum BestId: String, Codable, Sendable, Equatable, CaseIterable {
    case longestDictation = "longest_dictation"
    case fastestDictation = "fastest_dictation"
    case mostWordsDay = "most_words_day"
    case bestWeek = "best_week"
    case longestMeeting = "longest_meeting"
    case longestMonologue = "longest_monologue"
}

/// A best just set, for a short note in the Drop (a dictation's) or at the meeting's end (a
/// meeting's).
public struct BestNews: Codable, Sendable, Equatable {
    /// When, as a BestRow's date.
    public let date: String
    /// Which.
    public let id: BestId
    /// The best now.
    public let new: Int64
    /// The best before.
    public let old: Int64
    /// The take holding it, as a BestRow's record.
    public let record: String?
    /// What old and new count.
    public let unit: BestUnit
}

/// A personal best held.
public struct BestRow: Codable, Sendable, Equatable {
    /// When, YYYY-MM-DD: the take's local day, the day, or the week's first day.
    public let date: String
    /// Which.
    public let id: BestId
    /// The take holding it, for a take's best; absent for a day's or a week's.
    public let record: String?
    /// What value counts.
    public let unit: BestUnit
    /// The best, in its unit.
    public let value: Int64
}

/// What a best's value counts: ms, words a minute, or words.
public enum BestUnit: String, Codable, Sendable, Equatable, CaseIterable {
    case ms
    case wpm
    case words
}

/// An app in the call policies' list: one detection has seen hold the microphone for a call, or
/// one the user chose for. Never shown by its identity: a shell names it and shows its icon
/// from the identity, as for a mode's apps.
public struct CallApp: Codable, Sendable, Equatable {
    /// Its identity, as detection reports it: a bundle id on the Mac, the executable (or the
    /// package's app id) on Windows, where it is kept in lowercase.
    public let app: String
    /// Its name as detection last saw it; absent for an app chosen for that has not been seen.
    public let appName: String?
    /// Whether the user chose its policy; false while it follows the default.
    public let chosen: Bool
    /// What happens for it now: the user's choice, else the default (Always is Ask while the
    /// stored list could not be read).
    public let policy: CallPolicy
    /// When detection last saw it hold the microphone for a call, Unix ms.
    public let seenUnixMs: Int64?

    private enum CodingKeys: String, CodingKey {
        case app
        case appName = "app_name"
        case chosen
        case policy
        case seenUnixMs = "seen_unix_ms"
    }
}

/// What happens when an app holds the microphone for a call: always (recorded at once, visibly,
/// as by Record), ask (the consent Drop offers it) or never (neither).
public enum CallPolicy: String, Codable, Sendable, Equatable, CaseIterable {
    case always
    case ask
    case never
}

/// A model in the catalogue that runs on this OS.
public struct CatalogueEntry: Codable, Sendable, Equatable {
    /// Its id.
    public let id: String
    /// Whether its files are installed and complete.
    public let installed: Bool
    /// The jobs it fills, each with its measured error rate. None for a model the core only
    /// downloads because the shell runs it (the Mac's Parakeet, parakeet-tdt-0.6b-v3-coreml).
    public let jobs: [JobScore]
    /// Its weights' licence.
    public let licence: String
    /// Its download size.
    public let sizeBytes: Int64

    private enum CodingKeys: String, CodingKey {
        case id
        case installed
        case jobs
        case licence
        case sizeBytes = "size_bytes"
    }
}

/// Which chain a queue feeds.
public enum Chain: String, Codable, Sendable, Equatable, CaseIterable {
    case dictation
    case meeting
}

/// A side of a meeting: the mic is you, the far end is them (stream identity, architecture rule
/// 5).
public enum Channel: String, Codable, Sendable, Equatable, CaseIterable {
    case mic
    case far
}

/// What a meeting's final pass did on one side.
public struct ChannelPass: Codable, Sendable, Equatable {
    /// Time above the audible floor, ms, before any gain.
    public let audibleMs: Int64
    /// Live finals saved without the speech check.
    public let backloggedFinals: Int64
    /// Audio in the readable chunks, ms. Zero: nothing was captured.
    public let capturedMs: Int64
    /// Which side.
    public let channel: Channel
    /// Chunk files the final pass found.
    public let chunks: Int64
    /// Chunk files the pump wrote for this side; absent when the pump did not report.
    public let chunksWritten: Int64?
    /// Regions that came back without words.
    public let emptyRegions: Int64
    /// Regions that failed.
    public let failedRegions: Int64
    /// Speech regions the engine was given.
    public let regions: Int64
    /// Time the VAD found as speech, ms.
    public let speechMs: Int64
    /// Words in the result.
    public let wordCount: Int64

    private enum CodingKeys: String, CodingKey {
        case audibleMs = "audible_ms"
        case backloggedFinals = "backlogged_finals"
        case capturedMs = "captured_ms"
        case channel
        case chunks
        case chunksWritten = "chunks_written"
        case emptyRegions = "empty_regions"
        case failedRegions = "failed_regions"
        case regions
        case speechMs = "speech_ms"
        case wordCount = "word_count"
    }
}

/// A voice command. The core carries out change_style, toggle_polish and insert_text itself;
/// the others are recognised and not carried out in this build.
public enum CommandAction: String, Codable, Sendable, Equatable, CaseIterable {
    case undo
    case changeStyle = "change_style"
    case switchModel = "switch_model"
    case togglePolish = "toggle_polish"
    case toggleDictation = "toggle_dictation"
    case openUrl = "open_url"
    case openApp = "open_app"
    case insertText = "insert_text"
    case other
}

/// A command was read and could not be carried out.
public struct CommandFailed: Codable, Sendable, Equatable {
    /// Which failure, for the few a shell acts on; absent for the rest. Match on this, never on
    /// the message.
    public let code: FailureCode?
    /// The command's "cmd".
    public let command: String
    /// The command's "id", when it had one.
    public let id: String?
    /// Why. Names what failed, never what was said.
    public let message: String
    /// Always `command.failed`.
    public let type: String
}

/// A commitment: something promised in a record.
public struct CommitmentRow: Codable, Sendable, Equatable {
    /// Its id.
    public let commitment: String
    /// Whether it is done.
    public let done: Bool
    /// When, as said ("by Friday").
    public let due: String?
    /// When, resolved, Unix ms; absent when it could not be resolved.
    public let dueAtUnixMs: Int64?
    /// Where a later meeting suggests it is already done, until the user answers.
    public let looksDone: DoneEvidence?
    /// The commitment it was folded into ("said twice"), when it was.
    public let mergedInto: String?
    /// Who owes it, as said.
    public let owner: String?
    /// Where in the record it was said.
    public let provenance: [Span]
    /// Who it is owed to, as said, when the transcript says.
    public let recipient: String?
    /// The record it came from.
    public let record: String
    /// What was promised. The library's words: never log it.
    public let text: String

    private enum CodingKeys: String, CodingKey {
        case commitment
        case done
        case due
        case dueAtUnixMs = "due_at_unix_ms"
        case looksDone = "looks_done"
        case mergedInto = "merged_into"
        case owner
        case provenance
        case recipient
        case record
        case text
    }
}

/// A commitment was marked done or open again (commitment.set_done), or its looks-done
/// suggestion was dismissed (commitment.not_yet). Either way, a suggestion on it is settled.
public struct CommitmentUpdated: Codable, Sendable, Equatable {
    /// Its id.
    public let commitment: String
    /// Whether it is done now.
    public let done: Bool
    /// Always `commitment.updated`.
    public let type: String
}

/// The open commitments across the library, in answer to commitments.list: the soonest resolved
/// due time first, undated ones last. Carries the promises' words.
public struct CommitmentsListed: Codable, Sendable, Equatable {
    /// The commitments.
    public let items: [OwedItem]
    /// Always `commitments.listed`.
    public let type: String
}

/// A feature that sends the user's words to a language model: its switch, where the model it
/// would use now sends them, and where the user agreed it may. The feature runs only when on
/// and allowed. In answer to consent.get and consent.allow, and after the feature's switch is
/// set to off.
public struct ConsentState: Codable, Sendable, Equatable {
    /// Whether that consent covers the model now: false while the feature is on means it is
    /// paused until the user agrees again.
    public let allowed: Bool
    /// For a cloud consent, the provider's name the user agreed to.
    public let allowedName: String?
    /// Where the user agreed it may send; absent when never agreed (or turned off since).
    public let allowedTo: LlmDestination?
    /// For cloud, the destination consent.allow must name; absent otherwise.
    public let endpoint: String?
    /// Why part of this could not be read (the switch or the consent), as a sentence starting
    /// "couldn't". An unread switch counts as off, an unread consent as none.
    public let error: String?
    /// Which feature.
    public let feature: LlmFeature
    /// That model's name, as the shell registered it (a cloud provider's name); absent with to.
    public let name: String?
    /// Its switch: dictation.polish on, a voice-edit key set, or meetings.llm on.
    public let on: Bool
    /// The command's "id", when it had one.
    public let ref: String?
    /// Where the model it would use now sends; absent when no language model is registered.
    public let to: LlmDestination?
    /// Always `consent.state`.
    public let type: String

    private enum CodingKeys: String, CodingKey {
        case allowed
        case allowedName = "allowed_name"
        case allowedTo = "allowed_to"
        case endpoint
        case error
        case feature
        case name
        case on
        case ref
        case to
        case type
    }
}

/// The core started. The first event after ink_init.
public struct CoreReady: Codable, Sendable, Equatable {
    /// The core's ABI version; must equal INK_ABI_VERSION in inkwell.h.
    public let abi: Int64
    /// Always `core.ready`.
    public let type: String
    /// The core's version.
    public let version: String
}

/// The last event before ink_shutdown returns.
public struct CoreStopped: Codable, Sendable, Equatable {
    /// Always `core.stopped`.
    public let type: String
}

/// The take was a voice command.
public struct DictationCommand: Codable, Sendable, Equatable {
    /// Which.
    public let action: CommandAction
    /// Whether the core carried it out (change_style, toggle_polish and insert_text); the rest
    /// are recognised, so their words are not typed, but this build does not do them yet: say
    /// so. Absent from a core before 1.0.
    public let carriedOut: Bool?
    /// How much confirmation it needs.
    public let risk: Risk
    /// Always `dictation.command`.
    public let type: String
    /// Its argument: a style, a model id, a URL, an app path, or fixed text.
    public let value: String?

    private enum CodingKeys: String, CodingKey {
        case action
        case carriedOut = "carried_out"
        case risk
        case type
        case value
    }
}

/// The take ended without an insertion.
public struct DictationDiscarded: Codable, Sendable, Equatable {
    /// How long the key was held, for too_short.
    public let liveMs: Int64?
    /// Why. speech_too_short: tell the user it was too short and to try again.
    public let reason: Discard
    /// Always `dictation.discarded`.
    public let type: String

    private enum CodingKeys: String, CodingKey {
        case liveMs = "live_ms"
        case reason
        case type
    }
}

/// A voice edit ended without touching the selection.
public struct DictationEditFailed: Codable, Sendable, Equatable {
    /// The error, where there was one. Never the user's words.
    public let message: String?
    /// Why. no_selection: ask the user to select text first. no_model: editing needs a language
    /// model.
    public let reason: EditFailure
    /// Always `dictation.edit_failed`.
    public let type: String
}

/// The OS removed the voice-edit key. Nothing more arrives from it until dictation is enabled
/// again.
public struct DictationEditHotkeyLost: Codable, Sendable, Equatable {
    /// Always `dictation.edit_hotkey_lost`.
    public let type: String
}

/// A voice edit replaced the selection with its rewrite. The rewrite is not saved to the
/// library.
public struct DictationEdited: Codable, Sendable, Equatable {
    /// How it went in.
    public let outcome: InsertOutcome
    /// Always `dictation.edited`.
    public let type: String
}

/// A take failed. Nothing was inserted for a transcription failure.
public struct DictationFailed: Codable, Sendable, Equatable {
    /// Why. Never the user's words.
    public let message: String
    /// Which stage.
    public let stage: FailedStage
    /// Always `dictation.failed`.
    public let type: String
}

/// The OS removed the hotkey. Nothing more arrives until it is started again.
public struct DictationHotkeyLost: Codable, Sendable, Equatable {
    /// Always `dictation.hotkey_lost`.
    public let type: String
}

/// A dictation went out. Carries the user's words: never log it.
public struct DictationInserted: Codable, Sendable, Equatable {
    /// How it went in.
    public let outcome: InsertOutcome
    /// Its library record, unless saving failed.
    public let record: String?
    /// The text as inserted.
    public let text: String
    /// Always `dictation.inserted`.
    public let type: String
}

/// The microphone could not be opened for a take; the take was dropped. The next press tries
/// again.
public struct DictationMicFailed: Codable, Sendable, Equatable {
    /// Why. Never the user's words.
    public let message: String
    /// Always `dictation.mic_failed`.
    public let type: String
}

/// Dictation is not live: key presses reach nothing and no key is held. Sent in answer to
/// dictation.enable and dictation.disable, and when dictation stops on its own (the worker
/// stopped, Accessibility was revoked).
public struct DictationOff: Codable, Sendable, Equatable {
    /// The error, where there was one. Never the user's words.
    public let message: String?
    /// Why.
    public let reason: DictationOffReason
    /// The command's "id", when it had one.
    public let ref: String?
    /// Always `dictation.off`.
    public let type: String
}

/// Why dictation is not live.
public enum DictationOffReason: String, Codable, Sendable, Equatable, CaseIterable {
    case disabled
    case needsAccessibility = "needs_accessibility"
    case keyRefused = "key_refused"
    case unsupported
    case workerStopped = "worker_stopped"
    case failed
    case other
}

/// What the live engine hears while the key is held: settled words, then the current
/// hypothesis. Each replaces the last; none is saved, and none arrives for a take after its
/// dictation.stopped. Carries the user's words: never log it.
public struct DictationPartial: Codable, Sendable, Equatable {
    /// The take, as dictation.started numbered it.
    public let take: Int64
    /// The words so far.
    public let text: String
    /// Always `dictation.partial`.
    public let type: String
}

/// Dictation is live: the core holds the keys named here, and a hold of the key is a dictation.
/// Sent in answer to dictation.enable, and again after a key setting changes.
public struct DictationReady: Codable, Sendable, Equatable {
    /// The voice-edit key's token, when one is set and bound.
    public let editKey: String?
    /// Why the voice-edit key set in the settings could not be bound; dictation works without
    /// it. Never the user's words.
    public let editKeyError: String?
    /// The dictation key's token (for example fn or right_option).
    public let key: String
    /// The command's "id", when it had one.
    public let ref: String?
    /// Which of dictation's settings could not be read (it runs with their defaults), as a
    /// sentence: "couldn't read the modes". Never the user's words.
    public let settingsError: String?
    /// Always `dictation.ready`.
    public let type: String

    private enum CodingKeys: String, CodingKey {
        case editKey = "edit_key"
        case editKeyError = "edit_key_error"
        case key
        case ref
        case settingsError = "settings_error"
        case type
    }
}

/// A press shorter than the minimum hold (a modifier used in a shortcut). Nothing was shown or
/// transcribed.
public struct DictationShortPressIgnored: Codable, Sendable, Equatable {
    /// Always `dictation.short_press_ignored`.
    public let type: String
}

/// A hold passed the minimum and is now a take: show that it is listening (or, with edit, that
/// it is taking an instruction for the selection).
public struct DictationStarted: Codable, Sendable, Equatable {
    /// For a dictation: the name of the app in front, when the OS reported one.
    public let app: String?
    /// A voice edit (the edit key) rather than a dictation.
    public let edit: Bool
    /// For a dictation: the mode it is expected to write in, by its name, from the app in front
    /// (the text is written in the mode of the app that receives it).
    public let mode: String?
    /// This take's number within the session: partials carry it, so a late one is never shown
    /// under the next take.
    public let take: Int64
    /// Always `dictation.started`.
    public let type: String
}

/// The user's dictation, counted on this computer: finished dictations by the local day they
/// started. Speed and time saved count only dictations that know how long the key was held.
public struct DictationStats: Codable, Sendable, Equatable {
    /// Local days with a dictation since the month started.
    public let activeDaysMonth: Int64?
    /// Dictations, all time.
    public let dictationsAll: Int64
    /// The heatmap's first local day, YYYY-MM-DD: the first day of the week eleven weeks before
    /// this one.
    public let heatmapFirstDay: String
    /// Words dictated per local day, from heatmap_first_day to today.
    public let heatmapWords: [Int64]
    /// The latest streak, running or ended: what it reached. Once a streak has ended, the shell
    /// shows this and the longest, never a streak as lost.
    public let latestStreakDays: Int64?
    /// The longest streak, all time.
    public let longestStreakDays: Int64
    /// The weekdays the streak rests on (stats.rest_days), ISO: 1 Monday to 7 Sunday; empty for
    /// none. A rest day neither counts nor breaks the streak.
    public let restDays: [Int64]?
    /// What saved_ms_all is about, as saved_about_week.
    public let savedAboutAll: [TimeEquivalent]?
    /// What saved_ms_week is about, largest first ("about two feature films"). Absent when
    /// there is nothing to picture: time lost, under about 12 minutes, or between two counts.
    public let savedAboutWeek: [TimeEquivalent]?
    /// Time saved all time, ms, as saved_ms_week.
    public let savedMsAll: Int64
    /// Time saved this week, ms: the same words typed at typing_wpm less the time spent
    /// speaking. Negative when speaking took longer.
    public let savedMsWeek: Int64
    /// The current streak: local days with a dictation in a row, one missed day forgiven, two
    /// ending it. A rest day neither counts nor breaks it, with a dictation or without; a
    /// paused day without a dictation is not missed, and one with a dictation counts. Running
    /// while at most one day was missed since the last active one (today is never missed).
    public let streakDays: Int64
    /// Whether the user hid the streak (stats.streak): show no streak line and offer none on
    /// the share card. The numbers are still counted.
    public let streakHidden: Bool?
    /// The first day of the pause running today (streak.pause), YYYY-MM-DD. Absent while none
    /// runs; a pause ends by itself after 90 days.
    public let streakPausedSince: String?
    /// Words dictated, all time.
    public let wordsAll: Int64
    /// Words dictated today.
    public let wordsToday: Int64
    /// Words dictated since this week started (the shell's first weekday).
    public let wordsWeek: Int64
    /// Words per minute over the last 30 days, today included: the user's own average. Absent
    /// with less than a minute of speech in them.
    public let wpmAverage: Int64?
    /// Words per minute this week: words over the time the key was held. Absent with less than
    /// a minute of speech this week.
    public let wpmWeek: Int64?

    private enum CodingKeys: String, CodingKey {
        case activeDaysMonth = "active_days_month"
        case dictationsAll = "dictations_all"
        case heatmapFirstDay = "heatmap_first_day"
        case heatmapWords = "heatmap_words"
        case latestStreakDays = "latest_streak_days"
        case longestStreakDays = "longest_streak_days"
        case restDays = "rest_days"
        case savedAboutAll = "saved_about_all"
        case savedAboutWeek = "saved_about_week"
        case savedMsAll = "saved_ms_all"
        case savedMsWeek = "saved_ms_week"
        case streakDays = "streak_days"
        case streakHidden = "streak_hidden"
        case streakPausedSince = "streak_paused_since"
        case wordsAll = "words_all"
        case wordsToday = "words_today"
        case wordsWeek = "words_week"
        case wpmAverage = "wpm_average"
        case wpmWeek = "wpm_week"
    }
}

/// The take is closed and being processed.
public struct DictationStopped: Codable, Sendable, Equatable {
    /// Always `dictation.stopped`.
    public let type: String
}

/// Whether dictations are levelled with voice detection. Sent at the start and whenever it
/// changes; while unavailable the shell shows it.
public struct DictationVoiceDetection: Codable, Sendable, Equatable {
    /// Whether a VAD is in use.
    public let available: Bool
    /// Why not, when not.
    public let reason: VadUnavailable?
    /// Always `dictation.voice_detection`.
    public let type: String
}

/// Something went wrong during a dictation, and it went on without it.
/// deleted_text_not_scrubbed and deleted_text_scrubbed: as for meetings. release_missed: a
/// push-to-talk key was held for 180 s with no release (most likely lost); the take was stopped
/// there and processed. polish_not_allowed: polish is on, but the user has not agreed to send
/// dictations where its model goes now (never agreed, or the model changed destination since):
/// nothing was sent, the text went in as said, and message names the model; consent.get says
/// more.
public enum DictationWarning: String, Codable, Sendable, Equatable, CaseIterable {
    case vadFailed = "vad_failed"
    case audioLost = "audio_lost"
    case tailCutShort = "tail_cut_short"
    case focusUnreadable = "focus_unreadable"
    case polishUnavailable = "polish_unavailable"
    case polishFailed = "polish_failed"
    case polishTimedOut = "polish_timed_out"
    case polishNotAllowed = "polish_not_allowed"
    case noModeForStyle = "no_mode_for_style"
    case saveFailed = "save_failed"
    case deletedTextNotScrubbed = "deleted_text_not_scrubbed"
    case deletedTextScrubbed = "deleted_text_scrubbed"
    case releaseMissed = "release_missed"
    case other
}

/// Something went wrong during a take, and it went on without it.
public struct DictationWarningEvent: Codable, Sendable, Equatable {
    /// Frames lost, for audio_lost.
    public let frames: Int64?
    /// What.
    public let kind: DictationWarning
    /// The error, where there was one.
    public let message: String?
    /// Always `dictation.warning`.
    public let type: String
}

/// A stage of the dictation chain panicked; the take in progress is lost. With recovered, the
/// next take works. Without it the worker has stopped after repeated failures: the shell MUST
/// unbind the dictation hotkey (unbind_hotkey is then true) and show that dictation is off,
/// since key presses now reach nothing.
public struct DictationWorkerFailed: Codable, Sendable, Equatable {
    /// Whether the worker is still serving.
    public let recovered: Bool
    /// Always `dictation.worker_failed`.
    public let type: String
    /// The shell must unbind the hotkey now: always the opposite of recovered.
    public let unbindHotkey: Bool

    private enum CodingKeys: String, CodingKey {
        case recovered
        case type
        case unbindHotkey = "unbind_hotkey"
    }
}

/// Why a dictation ended without an insertion. None of these is an error.
public enum Discard: String, Codable, Sendable, Equatable, CaseIterable {
    case tooShort = "too_short"
    case silence
    case noSpeech = "no_speech"
    case speechTooShort = "speech_too_short"
    case nothingHeard = "nothing_heard"
    case nothingLeft = "nothing_left"
    case cancelled
    case other
}

/// Where a later meeting suggests an open commitment is already done: the user said, there,
/// that they had finished it. A suggestion for the user to confirm (commitment.set_done) or
/// dismiss (commitment.not_yet).
public struct DoneEvidence: Codable, Sendable, Equatable {
    /// The record it was said in.
    public let record: String
    /// When that record started, Unix ms.
    public let recordStartedAtUnixMs: Int64?
    /// That record's title, when it has one.
    public let recordTitle: String?
    /// Where in that record.
    public let span: Span
    /// The line said there, when it is still in the record's transcript. The library's words:
    /// never log it.
    public let text: String?

    private enum CodingKeys: String, CodingKey {
        case record
        case recordStartedAtUnixMs = "record_started_at_unix_ms"
        case recordTitle = "record_title"
        case span
        case text
    }
}

/// Why echo cancellation stopped: backlog (one side ran more than 10 s ahead of the other;
/// channel says which), bad_alignment (a path no canceller follows), stalled (the canceller
/// gave no answer in time; it is left behind), internal (a bug inside the core, reported rather
/// than hidden).
public enum EchoFailure: String, Codable, Sendable, Equatable, CaseIterable {
    case backlog
    case badAlignment = "bad_alignment"
    case stalled
    case `internal`
}

/// The echo path the final pass fitted over the whole recording.
public struct EchoPath: Codable, Sendable, Equatable {
    /// How late the mic hears the far end at the start, ms.
    public let delayMs: Double
    /// How fast the mic's clock runs against the far end's, ppm.
    public let driftPpm: Double
    /// Windows on the path.
    public let inliers: Int64
    /// Where the windows first supported it, ms into the meeting.
    public let stableFromMs: Int64?

    private enum CodingKeys: String, CodingKey {
        case delayMs = "delay_ms"
        case driftPpm = "drift_ppm"
        case inliers
        case stableFromMs = "stable_from_ms"
    }
}

/// Why the live echo search (re)started: the meeting started, the capture's device changed (a
/// new device is a new echo path), or cancellation failed.
public enum EchoSearch: String, Codable, Sendable, Equatable, CaseIterable {
    case start
    case deviceSwitch = "device_switch"
    case afterFailure = "after_failure"
}

/// A stretch of the meeting.
public struct EchoSpan: Codable, Sendable, Equatable {
    /// Its end.
    public let endMs: Int64
    /// Its start, ms into the meeting.
    public let startMs: Int64

    private enum CodingKeys: String, CodingKey {
        case endMs = "end_ms"
        case startMs = "start_ms"
    }
}

/// Whether the live mic is protected from the far end's echo. searching: no echo path yet, the
/// mic reaches the live transcript as captured (unprotected; with earbuds there is none to
/// find); cancelling: a path was found, the live you transcript hears the cancelled mic behind
/// the echo gate; degraded: cancelling, but the linear stage takes almost no echo off; failed:
/// cancellation stopped, and the search starts again; found_at_end: at the end, the search that
/// was still running found a path, so the mic went unprotected throughout it.
public enum EchoState: String, Codable, Sendable, Equatable, CaseIterable {
    case searching
    case cancelling
    case degraded
    case failed
    case foundAtEnd = "found_at_end"
}

/// Why a voice edit left the selection alone. None of these changed the user's text.
/// secure_input: Secure Input was on, so the selection was never read. not_allowed: the user
/// has not agreed to send the selection where the model goes now (never agreed, or it changed
/// destination since); nothing was sent, and message names the model.
public enum EditFailure: String, Codable, Sendable, Equatable, CaseIterable {
    case noSelection = "no_selection"
    case secureInput = "secure_input"
    case selectionUnreadable = "selection_unreadable"
    case transcription
    case noModel = "no_model"
    case timedOut = "timed_out"
    case model
    case insert
    case notAllowed = "not_allowed"
    case other
}

/// What kind of engine the shell registered: offline (finals), streaming (live partials) or llm
/// (a language model, for dictation polish).
public enum EngineKind: String, Codable, Sendable, Equatable, CaseIterable {
    case offline
    case streaming
    case llm
}

/// An engine the shell registered is in use: an offline or streaming engine in the router, or a
/// language model for polish.
public struct EngineRegistered: Codable, Sendable, Equatable {
    /// Its id.
    public let id: String
    /// The jobs it fills; empty for a language model.
    public let jobs: [JobScore]
    /// What kind of engine it is.
    public let kind: EngineKind
    /// Always `engine.registered`.
    public let type: String
}

/// What serves a job now, in answer to engine.route: the router's choice among installed models
/// and registered engines. Without id, nothing installed or registered fills the job.
public struct EngineRouted: Codable, Sendable, Equatable {
    /// The engine that serves it: a registry model's id, or the id a shell engine registered
    /// under.
    public let id: String?
    /// The job asked about.
    public let job: Job
    /// Where that engine comes from.
    public let source: EngineSource?
    /// Always `engine.routed`.
    public let type: String
}

/// Where a routed engine comes from: registry (a model downloaded from the built-in registry)
/// or shell (an engine the shell registered, such as FluidAudio's Parakeet on the Mac).
public enum EngineSource: String, Codable, Sendable, Equatable, CaseIterable {
    case registry
    case shell
}

/// An engine the shell registered was let go; its release function runs once no call is in
/// flight.
public struct EngineUnregistered: Codable, Sendable, Equatable {
    /// Its id.
    public let id: String
    /// Always `engine.unregistered`.
    public let type: String
}

/// Which stage of a dictation failed.
public enum FailedStage: String, Codable, Sendable, Equatable, CaseIterable {
    case transcription
    case insert
    case other
}

/// A command.failed a shell acts on: list_unreadable (a snippets.save, voice_commands.save or
/// meetings.calls.set refused because the stored list cannot be read; send it again with
/// replace_unreadable to start over); meeting_recording (an audio.test refused because a
/// meeting records: the mic test waits until it ends); delete_window_over (a meeting.discard
/// after the meeting's first minute: only Stop is left, and the record can be deleted from the
/// library once it is finished).
public enum FailureCode: String, Codable, Sendable, Equatable, CaseIterable {
    case listUnreadable = "list_unreadable"
    case meetingRecording = "meeting_recording"
    case deleteWindowOver = "delete_window_over"
}

/// What a meeting records as the other side: the sound of its app alone (a call recorded from
/// the consent Drop's offer), or everything this Mac plays except Inkwell itself (Record now,
/// which names no app, or a call whose app could not be heard alone: meeting.far_end_fallback
/// says so).
public enum FarEnd: String, Codable, Sendable, Equatable, CaseIterable {
    case app
    case everything
}

/// Whether this computer can watch a key binding as the dictation key or the edit key, in
/// answer to hotkey.check. A shell checks a shortcut the user recorded before it stores it with
/// setting.set; nothing is stored here.
public struct HotkeyChecked: Codable, Sendable, Equatable {
    /// The binding as the command spelled it.
    public let binding: String
    /// When ok: its one spelling, to store with setting.set and to compare keys by (two
    /// spellings of one chord are one key). For example ctrl+shift+space, right_option or f13.
    public let canonical: String?
    /// Whether this computer can watch it.
    public let ok: Bool
    /// When not ok: why not, in plain words starting in lower case and without a full stop, to
    /// show after "can't use that:". For example "that key on its own would stop working
    /// everywhere else; add Control, Option or Command".
    public let reason: String?
    /// The command's "id", when it had one.
    public let ref: String?
    /// Always `hotkey.checked`.
    public let type: String
}

/// What import.check found: Inkwell 0.2's data at 0.2's own data directory on this computer
/// (the core knows where), and whether this library holds it already. The data is read, never
/// written, and the keychain is not asked, so linked_keys is 0 here.
public struct ImportChecked: Codable, Sendable, Equatable {
    /// What an import would bring (the dry run), when found.
    public let counts: ImportCounts?
    /// Why the data cannot be read now, when unreadable: words to show (for one, that Inkwell
    /// 0.2 is still open and should be quit first).
    public let message: String?
    /// The command's id.
    public let ref: String?
    /// Whether there is something to import.
    public let state: ImportState
    /// Always `import.checked`.
    public let type: String
}

/// How many of each kind Inkwell 0.2's data holds, or an import wrote.
public struct ImportCounts: Codable, Sendable, Equatable {
    /// Per-app style rules.
    public let appStyleRules: Int64
    /// Dictations, each a record in the library.
    public let dictations: Int64
    /// Dictionary entries (a word and what replaces it).
    public let dictionaryEntries: Int64
    /// Providers whose API key in the keychain is linked, by reference (never copied).
    public let linkedKeys: Int64
    /// Modes.
    public let modes: Int64
    /// 0.2's own settings.
    public let settings: Int64
    /// Snippets.
    public let snippets: Int64
    /// Voice commands.
    public let voiceCommands: Int64

    private enum CodingKeys: String, CodingKey {
        case appStyleRules = "app_style_rules"
        case dictations
        case dictionaryEntries = "dictionary_entries"
        case linkedKeys = "linked_keys"
        case modes
        case settings
        case snippets
        case voiceCommands = "voice_commands"
    }
}

/// import.run brought Inkwell 0.2's data into the library, in one transaction: the library's
/// records changed (list them again), import.notes may have something to say about the
/// dictation key, and a running dictation already uses what came over. A failure is
/// command.failed, its message in words to show.
public struct ImportFinished: Codable, Sendable, Equatable {
    /// What it wrote.
    public let counts: ImportCounts
    /// The command's id.
    public let ref: String?
    /// Always `import.finished`.
    public let type: String
}

/// 0.2's dictation hotkey, and what the import made of it.
public struct ImportKeyNote: Codable, Sendable, Equatable {
    /// The import set it as the dictation key (a key already chosen in 1.0 is kept).
    public let applied: Bool
    /// 0.2's hotkey as it stored it: a modifier token (fn, right_cmd, ...) or a combination
    /// such as super+shift+space.
    public let hotkey: String
    /// The 1.0 dictation key it became, when it did.
    public let key: String?
    /// Whether it carried over.
    public let outcome: ImportKeyOutcome
    /// 0.2 started and stopped on separate presses; 1.0 is hold to talk.
    public let toggle: Bool
}

/// What became of 0.2's dictation hotkey: mapped to a 1.0 key (the same key), replaced by
/// another key because this OS never sees it (Fn on Windows becomes right Ctrl; key names
/// both), or not carried over, because it is a combination or another key 1.0 cannot hold on
/// its own.
public enum ImportKeyOutcome: String, Codable, Sendable, Equatable, CaseIterable {
    case mapped
    case replaced
    case combination
    case otherKey = "other_key"
}

/// What the Inkwell 0.2 import has to tell the user once, in answer to import.notes.
public struct ImportNotes: Codable, Sendable, Equatable {
    /// What became of 0.2's dictation hotkey; absent when there is nothing to say or the user
    /// dismissed it.
    public let key: ImportKeyNote?
    /// The command's id.
    public let ref: String?
    /// Always `import.notes`.
    public let type: String
}

/// Inkwell 0.2's data: found (and not imported yet), absent from this computer, imported into
/// this library already (0.2's data is not opened then), or found but unreadable now
/// (import.run may still work once the reason is gone).
public enum ImportState: String, Codable, Sendable, Equatable, CaseIterable {
    case found
    case absent
    case imported
    case unreadable
}

/// How a dictation went in. blocked: Secure Input or an elevated target refused synthetic
/// input, and nothing was inserted. inserted_clipboard_not_restored: inserted, and the previous
/// clipboard could not be put back.
public enum InsertOutcome: String, Codable, Sendable, Equatable, CaseIterable {
    case pasted
    case typed
    case blocked
    case insertedClipboardNotRestored = "inserted_clipboard_not_restored"
    case other
}

/// A job the router fills with the best installed engine.
public enum Job: String, Codable, Sendable, Equatable, CaseIterable {
    case dictationFinal = "dictation_final"
    case meetingFinal = "meeting_final"
    case livePartials = "live_partials"
    case diarization
    case voiceActivity = "voice_activity"
}

/// An engine's measured word error rate on one job.
public struct JobScore: Codable, Sendable, Equatable {
    /// The job.
    public let job: Job
    /// Word error rate, percent.
    public let wer: Double
}

/// Why the final pass did not replace the live transcript: refused (the supersede guard: an
/// empty result, or a side under half its live words) or incomplete (some regions failed).
public enum KeptLive: String, Codable, Sendable, Equatable, CaseIterable {
    case refused
    case incomplete
    case other
}

/// What the library holds of one kind since a moment.
public struct KindStats: Codable, Sendable, Equatable {
    /// Their total length, ms (records still live count nothing).
    public let durationMs: Int64
    /// The kind.
    public let kind: RecordKind
    /// Records that started since.
    public let records: Int64
    /// Words in their transcripts.
    public let words: Int64

    private enum CodingKeys: String, CodingKey {
        case durationMs = "duration_ms"
        case kind
        case records
        case words
    }
}

/// One record whole, in answer to record.open. Carries the library's words: never log it.
public struct LibraryRecord: Codable, Sendable, Equatable {
    /// Its audio, when it kept some and the directory is there.
    public let audio: RecordAudio?
    /// Its commitments, in the order they were added, merged ones included.
    public let commitments: [CommitmentRow]
    /// The user's notes, by stamp.
    public let notes: [RecordNote]
    /// The record.
    public let record: RecordRow
    /// The id of the command this answers, echoed so the shell can match the answer to its
    /// question.
    public let ref: String?
    /// The current revision's transcript, by start time.
    public let segments: [RecordSegment]
    /// The speakers the user named.
    public let speakers: [SpeakerName]
    /// Its summary, when one was written.
    public let summary: RecordSummary?
    /// Always `library.record`.
    public let type: String
}

/// Records, in answer to records.list: newest first (by start time, then id).
public struct LibraryRecords: Codable, Sendable, Equatable {
    /// The kind asked for; absent for every kind.
    public let kind: RecordKind?
    /// Whether more records follow the last one: ask again with it as "before".
    public let more: Bool
    /// The records, newest first.
    public let records: [RecordRow]
    /// The id of the command this answers, echoed so the shell can match the answer to its
    /// question.
    public let ref: String?
    /// Always `library.records`.
    public let type: String
}

/// Full-text matches, in answer to records.search, best first.
public struct LibrarySearch: Codable, Sendable, Equatable {
    /// The matches.
    public let hits: [SearchHit]
    /// The query, as asked. The user's words: never log it.
    public let query: String
    /// The id of the command this answers, echoed so the shell can match the answer to its
    /// question.
    public let ref: String?
    /// Always `library.search`.
    public let type: String
}

/// What the library holds since a moment, in answer to library.stats, and whether recent
/// meetings kept no far end.
public struct LibraryStats: Codable, Sendable, Equatable {
    /// The newest finished meetings in a row (of the last 20) that kept the user's words and
    /// none of the far end's: the sign that the other side was not heard.
    public let farSilentMeetings: Int64
    /// When the earliest of those started, Unix ms.
    public let farSilentSinceUnixMs: Int64?
    /// Per kind: dictation, meeting, file_import.
    public let kinds: [KindStats]
    /// The id of the command this answers, echoed so the shell can match the answer to its
    /// question.
    public let ref: String?
    /// The moment asked about, Unix ms.
    public let sinceUnixMs: Int64
    /// Always `library.stats`.
    public let type: String

    private enum CodingKeys: String, CodingKey {
        case farSilentMeetings = "far_silent_meetings"
        case farSilentSinceUnixMs = "far_silent_since_unix_ms"
        case kinds
        case ref
        case sinceUnixMs = "since_unix_ms"
        case type
    }
}

/// The retention setting deleted meetings and dictations older than it keeps (never imports),
/// at launch, after a meeting, or when it changed. Their text is overwritten in the library's
/// files, their audio removed.
public struct LibrarySwept: Codable, Sendable, Equatable {
    /// Records that started before this moment were due, Unix ms.
    public let beforeUnixMs: Int64
    /// Records deleted.
    public let deleted: Int64
    /// Records, or their audio, that could not be removed (each logged by what failed).
    public let failed: Int64
    /// Always `library.swept`.
    public let type: String

    private enum CodingKeys: String, CodingKey {
        case beforeUnixMs = "before_unix_ms"
        case deleted
        case failed
        case type
    }
}

/// Where a feature sends the user's words: on_device (a model on this machine; the words stay
/// on it) or cloud (a model elsewhere; the words leave this machine for its provider).
public enum LlmDestination: String, Codable, Sendable, Equatable, CaseIterable {
    case onDevice = "on_device"
    case cloud
}

/// A feature that sends the user's words to a language model, each with its own consent: polish
/// (the dictation, before it is typed), edit (voice edit: the selection and the spoken
/// instruction) or meetings (a meeting's summary, its commitments and Ask: the meeting's
/// transcript).
public enum LlmFeature: String, Codable, Sendable, Equatable, CaseIterable {
    case polish
    case edit
    case meetings
}

/// An own-key (BYOK) language model provider the user can choose: its id, what it uses unless
/// told otherwise, and whether its API key is stored. The key itself never leaves the OS key
/// store.
public struct LlmProviderEntry: Codable, Sendable, Equatable {
    /// Whether llm.choose may name another address (custom only).
    public let customUrl: Bool
    /// The model used when llm.choose names none.
    public let defaultModel: String
    /// Its address: fixed for a built-in provider; for custom, the address used when llm.choose
    /// names none.
    public let endpoint: String
    /// Whether a key is stored for it in the OS key store, asked without reading the key. False
    /// when that could not be asked (llm.providers' error says so).
    public let hasKey: Bool
    /// The provider: openai, groq, anthropic, openrouter or custom (any OpenAI-compatible
    /// server).
    public let id: String
    /// Whether a call needs its API key (a custom server usually runs without one).
    public let needsKey: Bool

    private enum CodingKeys: String, CodingKey {
        case customUrl = "custom_url"
        case defaultModel = "default_model"
        case endpoint
        case hasKey = "has_key"
        case id
        case needsKey = "needs_key"
    }
}

/// The own-key language model providers and the one chosen, in answer to llm.providers,
/// llm.key.save, llm.key.delete and llm.choose. A feature (polish, voice edit, summaries and
/// Ask) sends to the chosen provider only when no model is registered by the shell, and only
/// with the user's consent for its endpoint (consent.state).
public struct LlmProviders: Codable, Sendable, Equatable {
    /// For custom, the server's address as chosen; absent otherwise.
    public let baseUrl: String?
    /// The chosen provider's id; absent when none is chosen.
    public let chosen: String?
    /// Where the chosen provider sends, as consent.state names it; absent with chosen.
    public let endpoint: String?
    /// What could not be read (the stored keys, or the choice), as a sentence starting
    /// "couldn't"; absent when all was read.
    public let error: String?
    /// Local-only mode (llm.local_only): while on, a provider that is not on this machine is
    /// never called.
    public let localOnly: Bool
    /// The model the chosen provider is asked for; absent with chosen.
    public let model: String?
    /// Every provider, in preference order.
    public let providers: [LlmProviderEntry]
    /// Whether the chosen provider can be called: its key is stored (when it needs one), and
    /// local-only mode lets it through. Each feature still needs its own consent.
    public let ready: Bool
    /// The command's "id", when it had one.
    public let ref: String?
    /// Whether the chosen provider is on this machine (on_device) or not (cloud); absent with
    /// chosen.
    public let to: LlmDestination?
    /// Always `llm.providers`.
    public let type: String

    private enum CodingKeys: String, CodingKey {
        case baseUrl = "base_url"
        case chosen
        case endpoint
        case error
        case localOnly = "local_only"
        case model
        case providers
        case ready
        case ref
        case to
        case type
    }
}

/// The answer to llm.test: one short fixed request (never the user's words) sent to the chosen
/// provider with its stored key, and whether it answered.
public struct LlmTested: Codable, Sendable, Equatable {
    /// Why it did not answer, as a sentence starting "couldn't"; absent when ok. Names what
    /// failed, never the key.
    public let error: String?
    /// The model asked.
    public let model: String
    /// Whether the provider answered.
    public let ok: Bool
    /// The provider tested.
    public let provider: String
    /// The command's "id", when it had one.
    public let ref: String?
    /// The HTTP status of a refusal (401 or 403: the key; 404: the model or address; 429: the
    /// account's limits); absent otherwise.
    public let status: Int64?
    /// Always `llm.tested`.
    public let type: String
}

/// An answer to meeting.ask about the live meeting: the model's words. Render them as text only
/// (no links): model text can say anything. A failure is command.failed with the command's id.
/// Never log it.
public struct MeetingAnswered: Codable, Sendable, Equatable {
    /// The meeting's record id.
    public let record: String
    /// The id of the meeting.ask command this answers.
    public let ref: String?
    /// The answer.
    public let text: String
    /// Always `meeting.answered`.
    public let type: String
}

/// Capture stopped because the core's pump failed (a bug in the core, contained). What reached
/// disk is kept: the meeting ends as if capture had stopped, and its final pass runs over it.
public struct MeetingCaptureFailed: Codable, Sendable, Equatable {
    /// The meeting's record, when it had one.
    public let record: String?
    /// Always `meeting.capture_failed`.
    public let type: String
}

/// Commitments are saved.
public struct MeetingCommitments: Codable, Sendable, Equatable {
    /// Filed, duplicates included.
    public let filed: Int64
    /// Of those, folded into another.
    public let merged: Int64
    /// The meeting's record id.
    public let record: String
    /// Always `meeting.commitments`.
    public let type: String
}

/// An app has held the microphone long enough to be a call, and no meeting is being recorded:
/// the shell offers to record it (the consent Drop), and records only if the user says so
/// (meeting.start with this app). Its policy is Ask (meetings.calls), or Always when its
/// recording could not start by itself (message says why: a start that failed, or a far end
/// that would not be the app's sound alone), when the user stopped a recording by hand during
/// this call, or when the app was made Always during this call. The Drop can also set the app's
/// policy (meetings.calls.set): Always (then meeting.start) or Never.
public struct MeetingDetected: Codable, Sendable, Equatable {
    /// The app, by id (a bundle id on the Mac).
    public let app: String
    /// Its name, as the shell shows it.
    public let appName: String
    /// Why it is offered rather than recorded, when its policy is Always: its recording could
    /// not start by itself (the platform's error), or its own sound cannot be recorded alone,
    /// so the recording would hold everything this computer plays (the Mac's fallback, Windows'
    /// device loopback): an Always app is recorded by itself only when its sound alone is.
    /// Never content.
    public let message: String?
    /// Always `meeting.detected`.
    public let type: String

    private enum CodingKeys: String, CodingKey {
        case app
        case appName = "app_name"
        case message
        case type
    }
}

/// Whether the core is listening for calls now: sent once at start whatever the state (off
/// included, with a message when the default call policy could not be read), then when
/// detection starts, stops (the call policies: it listens while any app could be offered or
/// recorded, so a default of Never with no app chosen for is off), fails to start, or stops on
/// its own (the platform stopped answering). The shell shows this state, not the setting.
public struct MeetingDetection: Codable, Sendable, Equatable {
    /// Whether apps taking the microphone are being watched.
    public let listening: Bool
    /// Why not, when detection could not start or stopped on its own. Never content.
    public let message: String?
    /// Always `meeting.detection`.
    public let type: String
}

/// The offer to record an app is over before it was taken: the app released the microphone, or
/// the user said not this one (meeting.dismiss), or its policy became Never
/// (meetings.calls.set).
public struct MeetingDetectionEnded: Codable, Sendable, Equatable {
    /// The app, by id.
    public let app: String
    /// Whether the user dismissed it, by Not this one or Never (rather than the app releasing
    /// the microphone).
    public let dismissed: Bool
    /// Always `meeting.detection_ended`.
    public let type: String
}

/// Diarization of the far end finished.
public struct MeetingDiarized: Codable, Sendable, Equatable {
    /// Far-end segments given a speaker.
    public let attributed: Int64
    /// Clusters the diarizer returned.
    public let clusters: Int64
    /// Whether the labels were kept: at least two substantial clusters.
    public let labelled: Bool
    /// The meeting's record id.
    public let record: String
    /// Clusters holding at least 2 % of the far end's speech.
    public let substantial: Int64
    /// Always `meeting.diarized`.
    public let type: String
}

/// A meeting stopped with meeting.discard (Stop and delete), or one a crash interrupted on its
/// way to being deleted, is gone as if it had never been made: no final pass ran and nothing
/// was sent to a language model; its transcript, notes and search entries were deleted with
/// their words overwritten in the library's files, then its audio. Follows meeting.stopped; no
/// meeting.finished comes. The screens drop it.
public struct MeetingDiscarded: Codable, Sendable, Equatable {
    /// Its recorded audio is still on disk: it could not be removed, or its folder is outside
    /// the library and was left alone. The record itself is gone.
    public let audioLeft: Bool
    /// The record that is gone.
    public let record: String
    /// No copy of its words is left in the library's files (as for record.deleted).
    public let scrubbed: Bool
    /// Always `meeting.discarded`.
    public let type: String

    private enum CodingKeys: String, CodingKey {
        case audioLeft = "audio_left"
        case record
        case scrubbed
        case type
    }
}

/// Whether the live mic is protected from echo, when it changes. Which fields are present
/// depends on state.
public struct MeetingEcho: Codable, Sendable, Equatable {
    /// The side that ran ahead, for a backlog.
    public let channel: Channel?
    /// How late the mic hears the far end, ms, for cancelling.
    public let delayMs: Double?
    /// How fast the mic's clock runs against the far end's, ppm, for cancelling.
    public let driftPpm: Double?
    /// The linear stage's echo return loss enhancement over the last 20 s of far-end audio, dB,
    /// for degraded.
    public let erleDb: Double?
    /// What failed, for failed.
    public let failure: EchoFailure?
    /// Where cancellation began, ms into the meeting, for cancelling.
    public let fromMs: Int64?
    /// The meeting's record id.
    public let record: String
    /// Where the search began, ms into the meeting, for searching.
    public let sinceMs: Int64?
    /// Where the search's windows first supported the path, ms into the meeting, for cancelling
    /// and found_at_end.
    public let stableFromMs: Int64?
    /// The state.
    public let state: EchoState
    /// Always `meeting.echo`.
    public let type: String
    /// How long the mic went uncancelled since the search began, ms, for cancelling (0 when
    /// cancellation carries on along a better fit) and found_at_end.
    public let unprotectedMs: Int64?
    /// Why the search began, for searching.
    public let why: EchoSearch?

    private enum CodingKeys: String, CodingKey {
        case channel
        case delayMs = "delay_ms"
        case driftPpm = "drift_ppm"
        case erleDb = "erle_db"
        case failure
        case fromMs = "from_ms"
        case record
        case sinceMs = "since_ms"
        case stableFromMs = "stable_from_ms"
        case state
        case type
        case unprotectedMs = "unprotected_ms"
        case why
    }
}

/// What echo cancellation did in the final pass. Sent before the supersede.
public struct MeetingEchoPass: Codable, Sendable, Equatable {
    /// Whether the mic was cancelled (false: no path, or cancellation failed).
    public let cancelled: Bool
    /// Windows loud and clear enough to vote.
    public let candidates: Int64
    /// Echo return loss enhancement over far-end audio after that, dB.
    public let erleDb: Double?
    /// Echo return loss enhancement over the first 10 s of far-end audio, dB.
    public let erleFirstDb: Double?
    /// Lines that repeated the far end but were kept: the near end was heard over them.
    public let keptNearSpeech: Int64
    /// Lines that repeated the far end but were kept for want of acoustic evidence.
    public let keptNoEvidence: Int64
    /// The same for the linear output alone (what the you transcript hears), dB.
    public let linearErleDb: Double?
    /// Live you finals the pass judged to be echo; the supersede guard does not count them.
    public let liveEchoFinals: Int64
    /// The path, when the recording has one.
    public let path: EchoPath?
    /// The meeting's record id.
    public let record: String
    /// You lines removed as echo of the far end.
    public let removed: Int64
    /// Always `meeting.echo_pass`.
    public let type: String
    /// Windows analysed.
    public let windows: Int64

    private enum CodingKeys: String, CodingKey {
        case cancelled
        case candidates
        case erleDb = "erle_db"
        case erleFirstDb = "erle_first_db"
        case keptNearSpeech = "kept_near_speech"
        case keptNoEvidence = "kept_no_evidence"
        case linearErleDb = "linear_erle_db"
        case liveEchoFinals = "live_echo_finals"
        case path
        case record
        case removed
        case type
        case windows
    }
}

/// A meeting could not start, or its final pass stopped before replacing the live transcript
/// (which then stands, and the pass can run again).
public struct MeetingFailed: Codable, Sendable, Equatable {
    /// Why. Never the meeting's words.
    public let message: String
    /// The meeting's record, when it had one.
    public let record: String?
    /// Always `meeting.failed`.
    public let type: String
}

/// The meeting was started for an app whose own sound could not be recorded alone, so it
/// records everything this Mac plays instead (except Inkwell itself): other apps' sound is in
/// the recording too. Sent once, right after meeting.started; the shell says so where the
/// meeting shows.
public struct MeetingFarEndFallback: Codable, Sendable, Equatable {
    /// The app it was started for, by id.
    public let app: String
    /// That app's name, as the shell shows it.
    public let appName: String
    /// Why its sound could not be recorded alone (the platform's error). Never audio or words.
    public let message: String
    /// The meeting's record id.
    public let record: String
    /// Always `meeting.far_end_fallback`.
    public let type: String

    private enum CodingKeys: String, CodingKey {
        case app
        case appName = "app_name"
        case message
        case record
        case type
    }
}

/// Settled live text, saved as revision 1. Carries the meeting's words: never log it.
public struct MeetingFinal: Codable, Sendable, Equatable {
    /// Which side.
    public let channel: Channel
    /// End, ms into the meeting.
    public let endMs: Int64
    /// The meeting's record id.
    public let record: String
    /// Start, ms into the meeting.
    public let startMs: Int64
    /// The words.
    public let text: String
    /// Always `meeting.final`.
    public let type: String

    private enum CodingKeys: String, CodingKey {
        case channel
        case endMs = "end_ms"
        case record
        case startMs = "start_ms"
        case text
        case type
    }
}

/// Everything is done: the record is complete.
public struct MeetingFinished: Codable, Sendable, Equatable {
    /// The meeting's record id.
    public let record: String
    /// The transcript's revision now; absent only when the live one was kept and the record
    /// could not be read.
    public let revision: Int64?
    /// Always `meeting.finished`.
    public let type: String
}

/// The final pass did not replace the live transcript, which stays as it was.
public struct MeetingKeptLive: Codable, Sendable, Equatable {
    /// Regions that failed, for incomplete.
    public let failedRegions: Int64?
    /// The guard's reason, for refused.
    public let message: String?
    /// Why.
    public let reason: KeptLive
    /// The meeting's record id.
    public let record: String
    /// Always `meeting.kept_live`.
    public let type: String

    private enum CodingKeys: String, CodingKey {
        case failedRegions = "failed_regions"
        case message
        case reason
        case record
        case type
    }
}

/// In this meeting the user said work was already done that earlier meetings' open commitments
/// promise: each such commitment now carries a "looks done" suggestion (commitments.list shows
/// it) for the user to confirm or dismiss. Sent only when there is at least one.
public struct MeetingLooksDone: Codable, Sendable, Equatable {
    /// The meeting's record id.
    public let record: String
    /// Commitments given a suggestion.
    public let suggested: Int64
    /// Always `meeting.looks_done`.
    public let type: String
}

/// A recording meeting's mic went (unplugged, switched off) and the meeting records with
/// another now: the choice as it is now, else Automatic. A meeting never moves to a mic that
/// was plugged in or made the default mid-call; only its own mic going moves it. The mic_*
/// fields are meeting.started's, for the mic now.
public struct MeetingMicSwitched: Codable, Sendable, Equatable {
    /// The mic that went, as the OS named it, when known.
    public let fromName: String?
    /// How that mic connected, when known.
    public let fromTransport: MicTransport?
    /// The mic it records now.
    public let micName: String
    /// Why that mic.
    public let micReason: MicReason
    /// How that mic connects.
    public let micTransport: MicTransport
    /// The meeting's record id.
    public let record: String
    /// Always `meeting.mic_switched`.
    public let type: String

    private enum CodingKeys: String, CodingKey {
        case fromName = "from_name"
        case fromTransport = "from_transport"
        case micName = "mic_name"
        case micReason = "mic_reason"
        case micTransport = "mic_transport"
        case record
        case type
    }
}

/// Provisional live text; each replaces the last and none is saved. Carries the meeting's
/// words: never log it.
public struct MeetingPartial: Codable, Sendable, Equatable {
    /// Which side.
    public let channel: Channel
    /// The meeting's record id.
    public let record: String
    /// The hypothesis.
    public let text: String
    /// Always `meeting.partial`.
    public let type: String
}

/// A meeting a crash interrupted is being finished: its recorded audio was repaired and its
/// final pass follows (its events, ending in meeting.finished or meeting.failed).
public struct MeetingRecovered: Codable, Sendable, Equatable {
    /// Chunk headers rebuilt.
    public let rebuilt: Int64
    /// The meeting's record id.
    public let record: String
    /// How much audio the meeting recorded before the crash, ms.
    public let recordedMs: Int64
    /// Torn partial frames cut from a chunk's end.
    public let trimmed: Int64
    /// Always `meeting.recovered`.
    public let type: String
    /// Chunk files left as they were: nothing could place them.
    public let unrecoverable: Int64

    private enum CodingKeys: String, CodingKey {
        case rebuilt
        case record
        case recordedMs = "recorded_ms"
        case trimmed
        case type
        case unrecoverable
    }
}

/// You lines the final pass removed as echo, by place and span only. Sent only when there are
/// some, before the supersede.
public struct MeetingRemovedAsEcho: Codable, Sendable, Equatable {
    /// The lines, by start time.
    public let lines: [RemovedEchoLine]
    /// The meeting's record id.
    public let record: String
    /// Always `meeting.removed_as_echo`.
    public let type: String
}

/// What a side is delivering, when that changes.
public struct MeetingSideState: Codable, Sendable, Equatable {
    /// Which side.
    public let channel: Channel
    /// The meeting's record id.
    public let record: String
    /// Its state.
    public let state: SideState
    /// Always `meeting.side_state`.
    public let type: String
}

/// A meeting's record exists and its capture is being transcribed live. Names what the shell
/// shows of it: its title, its app and its mic, when known. Sent for every meeting, however it
/// started (Record, the consent Drop's offer, or an app's Always policy): the shell shows the
/// recording indicator from this event, so a call recorded by its policy shows exactly as one
/// the user started.
public struct MeetingStarted: Codable, Sendable, Equatable {
    /// The app it records, by id (a bundle id on the Mac), when it was started for one.
    public let app: String?
    /// That app's name, as the shell shows it.
    public let appName: String?
    /// True when the app's call policy (Always) started it, without a tap: the shell says so
    /// where the recording shows, keeps the reminder to tell the others, and offers Stop and
    /// Stop and delete. Absent for a start the user made. A policy start always records the
    /// app's own sound alone (far_end app).
    public let auto: Bool?
    /// Until this moment, Unix ms (a minute after the start), meeting.discard (Stop and delete)
    /// may delete this meeting as if it had never been made; after it only Stop is offered
    /// (meeting.discard is refused with delete_window_over). Absent for a meeting that cannot
    /// be deleted so (a replay).
    public let deleteUntilUnixMs: Int64?
    /// What it records as the other side.
    public let farEnd: FarEnd?
    /// The microphone it records, as the OS names it.
    public let micName: String?
    /// Why that microphone was chosen.
    public let micReason: MicReason?
    /// How that microphone connects.
    public let micTransport: MicTransport?
    /// The meeting's record id.
    public let record: String
    /// Its title, when known at the start (a calendar event, a replay's name); otherwise the
    /// summary's headline names it later.
    public let title: String?
    /// Always `meeting.started`.
    public let type: String

    private enum CodingKeys: String, CodingKey {
        case app
        case appName = "app_name"
        case auto
        case deleteUntilUnixMs = "delete_until_unix_ms"
        case farEnd = "far_end"
        case micName = "mic_name"
        case micReason = "mic_reason"
        case micTransport = "mic_transport"
        case record
        case title
        case type
    }
}

/// Meetings recorded here and finished, counted (imported meetings are left out: their channels
/// came from elsewhere). Me versus them is stream identity: the mic is the user, the far end
/// everyone else.
public struct MeetingStats: Codable, Sendable, Equatable {
    /// The user's longest stretch of speech with no one else speaking and no pause over 3
    /// seconds, ms.
    public let longestMonologueMs: Int64
    /// Meetings.
    public let meetings: Int64
    /// The user's lines ending in a question mark: a plain count.
    public let questions: Int64
    /// Their total length, ms.
    public let recordedMs: Int64
    /// Everyone else's talk time, ms.
    public let themMs: Int64
    /// The user's talk time, ms: how long their lines cover.
    public let youMs: Int64

    private enum CodingKeys: String, CodingKey {
        case longestMonologueMs = "longest_monologue_ms"
        case meetings
        case questions
        case recordedMs = "recorded_ms"
        case themMs = "them_ms"
        case youMs = "you_ms"
    }
}

/// Capture ended and the record is marked ended; the final pass runs next.
public struct MeetingStopped: Codable, Sendable, Equatable {
    /// The meeting's record id.
    public let record: String
    /// Always `meeting.stopped`.
    public let type: String
}

/// The summary is saved.
public struct MeetingSummarized: Codable, Sendable, Equatable {
    /// The meeting's record id.
    public let record: String
    /// Always `meeting.summarized`.
    public let type: String
    /// Items dropped because their citation did not check out.
    public let unverified: Int64
}

/// The final pass replaced the live transcript.
public struct MeetingSuperseded: Codable, Sendable, Equatable {
    /// The meeting's record id.
    public let record: String
    /// The new revision.
    public let revision: Int64
    /// Always `meeting.superseded`.
    public let type: String
}

/// The final pass finished one side.
public struct MeetingTranscribed: Codable, Sendable, Equatable {
    /// What it did.
    public let pass: ChannelPass
    /// The meeting's record id.
    public let record: String
    /// Always `meeting.transcribed`.
    public let type: String
}

/// Whether a side is levelled with voice detection. Sent per side at the start and when it
/// changes.
public struct MeetingVoiceDetection: Codable, Sendable, Equatable {
    /// Whether a VAD is in use.
    public let available: Bool
    /// Which side.
    public let channel: Channel
    /// Why not, when not.
    public let reason: VadUnavailable?
    /// The meeting's record id.
    public let record: String
    /// Always `meeting.voice_detection`.
    public let type: String
}

/// Something went wrong in a meeting, and it went on without it. deleted_text_not_scrubbed:
/// text the library deleted or replaced could not yet be cleared from its files (another
/// process is reading the database; the change is saved, and the store keeps trying);
/// deleted_text_scrubbed: it now is. Each is sent once per change. not_crash_protected: the
/// crash-recovery marker could not be written when the meeting started, so if the app quits
/// unexpectedly this meeting is not finished at the next launch (its audio is still saved).
/// summary_not_allowed: the user has not agreed to send meeting transcripts where the language
/// model goes now (the meetings consent: never agreed, or the model changed destination since):
/// nothing was sent, so the meeting has no summary and no commitments, and message names the
/// model; consent.get says more.
public enum MeetingWarning: String, Codable, Sendable, Equatable, CaseIterable {
    case vadFailed = "vad_failed"
    case capture
    case audioLost = "audio_lost"
    case liveEngineFailed = "live_engine_failed"
    case liveFinalsBacklog = "live_finals_backlog"
    case liveEventsAfterStop = "live_events_after_stop"
    case farEndQuietWhileYouSpeak = "far_end_quiet_while_you_speak"
    case liveEngineStalled = "live_engine_stalled"
    case finalWithoutSpeech = "final_without_speech"
    case emptySpeechRegion = "empty_speech_region"
    case finalEngineFailed = "final_engine_failed"
    case audioUnreadable = "audio_unreadable"
    case littleSpeechHeard = "little_speech_heard"
    case capturedOnlyZeros = "captured_only_zeros"
    case bluetoothMicOnlyZeros = "bluetooth_mic_only_zeros"
    case audioUnlisted = "audio_unlisted"
    case nothingCaptured = "nothing_captured"
    case diarizationFailed = "diarization_failed"
    case storeFailed = "store_failed"
    case clockWentBack = "clock_went_back"
    case summaryUnavailable = "summary_unavailable"
    case summaryFailed = "summary_failed"
    case summaryNotAllowed = "summary_not_allowed"
    case commitmentsFailed = "commitments_failed"
    case echoOnlyFinal = "echo_only_final"
    case echoGateVadFailed = "echo_gate_vad_failed"
    case echoFailed = "echo_failed"
    case echoPathNotFound = "echo_path_not_found"
    case deletedTextNotScrubbed = "deleted_text_not_scrubbed"
    case deletedTextScrubbed = "deleted_text_scrubbed"
    case notCrashProtected = "not_crash_protected"
    case other
}

/// Something went wrong in a meeting, and it went on without it. Which fields are present
/// depends on kind.
public struct MeetingWarningEvent: Codable, Sendable, Equatable {
    /// Time above the audible floor, ms; for echo_path_not_found, while the far end played.
    public let audibleMs: Int64?
    /// Which side.
    public let channel: Channel?
    /// Chunk files skipped or cut short.
    public let chunks: Int64?
    /// Events counted.
    public let count: Int64?
    /// End of the stretch.
    public let endMs: Int64?
    /// What failed, for echo_failed.
    public let failure: EchoFailure?
    /// Frames lost.
    public let frames: Int64?
    /// What.
    public let kind: MeetingWarning
    /// The error, where there was one. Never the meeting's words.
    public let message: String?
    /// Live or the final pass.
    public let phase: Phase?
    /// How long the far end was without signal, ms.
    public let quietMs: Int64?
    /// The meeting's record id.
    public let record: String
    /// Time found as speech, ms.
    public let speechMs: Int64?
    /// Start of the stretch, ms into the meeting.
    public let startMs: Int64?
    /// Always `meeting.warning`.
    public let type: String

    private enum CodingKeys: String, CodingKey {
        case audibleMs = "audible_ms"
        case channel
        case chunks
        case count
        case endMs = "end_ms"
        case failure
        case frames
        case kind
        case message
        case phase
        case quietMs = "quiet_ms"
        case record
        case speechMs = "speech_ms"
        case startMs = "start_ms"
        case type
    }
}

/// The meeting's worker panicked and stopped. Its recorded audio is on disk; the live
/// transcript so far is saved.
public struct MeetingWorkerFailed: Codable, Sendable, Equatable {
    /// The meeting's record id.
    public let record: String
    /// Always `meeting.worker_failed`.
    public let type: String
}

/// The call policies: the default for apps not chosen for, and every app seen or chosen for (at
/// most 64), most recently seen first. In answer to meetings.calls.list and meetings.calls.set
/// (with ref), and unasked when the default changed or detection saw a new app.
public struct MeetingsCalls: Codable, Sendable, Equatable {
    /// The apps.
    public let apps: [CallApp]
    /// The policy for apps not chosen for (meetings.calls.default; ask unless set).
    public let `default`: CallPolicy
    /// Why the stored choices could not be read, while they are set aside: every app follows
    /// the default then, with Always lowered to Ask, and meetings.calls.set is refused
    /// (list_unreadable) unless it says replace_unreadable, which starts the list over. In the
    /// answer to that start over under a default of Always: that the default is Ask now
    /// (written), so the user sets Always again knowingly.
    public let message: String?
    /// The command's "id", when it had one, so the shell can match the answer to what it sent.
    public let ref: String?
    /// Always `meetings.calls`.
    public let type: String
}

/// Recovery (meetings.recover) is done.
public struct MeetingsRecovered: Codable, Sendable, Equatable {
    /// Interrupted meetings it finished or tried to.
    public let meetings: Int64
    /// Present when the meetings could not even be looked for (the data directory could not be
    /// listed): an interrupted meeting may be waiting, and the next launch looks again. Names
    /// what failed, never a meeting's words.
    public let message: String?
    /// Always `meetings.recovered`.
    public let type: String
}

/// Why Inkwell records this microphone: the one the user chose in Settings > Sound (chosen,
/// found by its id or by its name and transport); Automatic standing in for a chosen mic that
/// is not connected (chosen_missing); or Automatic's reason: the system default input; the
/// built-in mic because the output is Bluetooth (a headset mic is call-quality audio; on
/// Windows a USB mic may be the one kept); the headset's own mic because a platform's retired
/// headset-mic switch is on; the default because this Mac has no built-in mic (on Windows:
/// every mic is Bluetooth); the first input because no default is set; it was named; the LE
/// Audio headset's own mic, which keeps full quality (Windows); or a reason this build of the
/// core does not name (unknown).
public enum MicReason: String, Codable, Sendable, Equatable, CaseIterable {
    case chosen
    case chosenMissing = "chosen_missing"
    case defaultInput = "default_input"
    case builtInForBluetoothOutput = "built_in_for_bluetooth_output"
    case headsetMicSetting = "headset_mic_setting"
    case noBuiltInMic = "no_built_in_mic"
    case firstInput = "first_input"
    case requested
    case leAudioHeadset = "le_audio_headset"
    case unknown
}

/// How a device connects: built in, Bluetooth (call-quality audio, and zeros while its user is
/// silent), USB, a virtual or aggregate device, or anything else.
public enum MicTransport: String, Codable, Sendable, Equatable, CaseIterable {
    case builtIn = "built_in"
    case bluetooth
    case usb
    case virtual
    case other
}

/// What a milestone counts: words dictated all time, or the longest streak in days.
public enum MilestoneKind: String, Codable, Sendable, Equatable, CaseIterable {
    case words
    case streak
}

/// A milestone's name, a key the shells word: first_page (1,000 words), notebook (10,000),
/// short_novel (50,000), novels_worth (100,000), seven_days, thirty_days and hundred_days
/// (streaks).
public enum MilestoneName: String, Codable, Sendable, Equatable, CaseIterable {
    case firstPage = "first_page"
    case notebook
    case shortNovel = "short_novel"
    case novelsWorth = "novels_worth"
    case sevenDays = "seven_days"
    case thirtyDays = "thirty_days"
    case hundredDays = "hundred_days"
}

/// A milestone and whether the library has reached it.
public struct MilestoneRow: Codable, Sendable, Equatable {
    /// Its id: words_1000, words_10000, words_50000, words_100000, streak_7, streak_30,
    /// streak_100.
    public let id: String
    /// What it counts.
    public let kind: MilestoneKind
    /// Its name, worded by the shell: the same on the chip, in the celebration and on the share
    /// card's seal.
    public let name: MilestoneName?
    /// Whether it is reached.
    public let reached: Bool
    /// The count that reaches it.
    public let threshold: Int64
}

/// In answer to milestones.check: the milestones reached since the last check, to celebrate,
/// and a best the take just set, for a short note. Each milestone is reported once ever; a
/// library's first check, and any check while stats.celebrate is off, reports none (what is
/// reached is noted all the same). A hidden streak's milestones are noted, not reported.
public struct MilestonesReached: Codable, Sendable, Equatable {
    /// A best the newest take, today or this week just set: at most one a day, never on a
    /// library's first check or with stats.celebrate off, and only once it beat at least five
    /// earlier entries. Absent otherwise.
    public let best: BestNews?
    /// The newly reached milestones, in the fixed order of stats.counted's; usually none.
    public let milestones: [MilestoneRow]
    /// The id of the command this answers, echoed so the shell can match the answer to its
    /// question.
    public let ref: String?
    /// Always `milestones.reached`.
    public let type: String
}

/// A mode: how dictation writes in the apps it names.
public struct ModeInfo: Codable, Sendable, Equatable {
    /// The apps it is picked for, matched against the frontmost app's identity. Never shown to
    /// the user as they are: a shell names each app.
    public let apps: [String]
    /// Its id.
    public let id: String
    /// Its name, as the user sees and says it.
    public let name: String
    /// Whether its dictations are polished (when a language model can).
    public let polish: Bool
    /// Whether fillers and stutters are removed.
    public let removeFillers: Bool
    /// How it writes.
    public let style: ModeStyle

    private enum CodingKeys: String, CodingKey {
        case apps
        case id
        case name
        case polish
        case removeFillers = "remove_fillers"
        case style
    }
}

/// How a mode writes: a style this build knows, or other.
public enum ModeStyle: String, Codable, Sendable, Equatable, CaseIterable {
    case formal
    case casual
    case relaxed
    case other
}

/// A job asked for a model that is held exclusively (being updated), and was refused. The job
/// fails; nothing was loaded from files being replaced.
public struct ModelRefused: Codable, Sendable, Equatable {
    /// The model's registry id.
    public let id: String
    /// The job that asked.
    public let job: Job
    /// Why.
    public let reason: Refusal
    /// Always `model.refused`.
    public let type: String
}

/// A model update ended, and its hold is released.
public struct ModelUpdateFinished: Codable, Sendable, Equatable {
    /// The model that was to be replaced.
    public let id: String
    /// Why it failed, when it did.
    public let message: String?
    /// The model replacing it.
    public let next: String
    /// The update left no model warm: dictation has no model until one is warmed.
    public let noModelWarm: Bool
    /// Whether the new model is installed (and warm, if the old one was).
    public let ok: Bool
    /// Always `model.update_finished`.
    public let type: String

    private enum CodingKeys: String, CodingKey {
        case id
        case message
        case next
        case noModelWarm = "no_model_warm"
        case ok
        case type
    }
}

/// How far a model update's download has got, between model.update_started and
/// model.update_finished: about four a second at most, and one when every byte is on disk
/// (done_bytes equal to total_bytes; model.update_finished then says whether the files checked
/// out). A model already installed sends only that one.
public struct ModelUpdateProgress: Codable, Sendable, Equatable {
    /// Bytes on disk so far across its files. It can go down: a file whose server ignores a
    /// resume starts over.
    public let doneBytes: Int64
    /// The model being replaced (the same as next for a first download).
    public let id: String
    /// The model being downloaded.
    public let next: String
    /// Its download size (models.listed's size_bytes).
    public let totalBytes: Int64
    /// Always `model.update_progress`.
    public let type: String

    private enum CodingKeys: String, CodingKey {
        case doneBytes = "done_bytes"
        case id
        case next
        case totalBytes = "total_bytes"
        case type
    }
}

/// A model update holds its model: jobs that need it are refused until model.update_finished.
public struct ModelUpdateStarted: Codable, Sendable, Equatable {
    /// The model being replaced.
    public let id: String
    /// The model replacing it.
    public let next: String
    /// Always `model.update_started`.
    public let type: String
}

/// A job's model could not be warmed.
public struct ModelWarmFailed: Codable, Sendable, Equatable {
    /// The model's id, when one was chosen.
    public let id: String?
    /// The job.
    public let job: Job
    /// Why.
    public let message: String
    /// Always `model.warm_failed`.
    public let type: String
}

/// A job's model is loaded and kept loaded.
public struct ModelWarmed: Codable, Sendable, Equatable {
    /// The model's registry id.
    public let id: String
    /// The job.
    public let job: Job
    /// Always `model.warmed`.
    public let type: String
}

/// The catalogue's models for this OS, in answer to models.list. What serves each job now is
/// engine.route's answer.
public struct ModelsListed: Codable, Sendable, Equatable {
    /// The models, in the catalogue's order.
    public let models: [CatalogueEntry]
    /// Always `models.listed`.
    public let type: String
}

/// The user's modes, in answer to modes.list, in the order they are matched: the first mode
/// naming the frontmost app wins, else the default.
public struct ModesListed: Codable, Sendable, Equatable {
    /// The mode used when no other matches.
    public let defaultId: String
    /// The modes.
    public let modes: [ModeInfo]
    /// Always `modes.listed`.
    public let type: String

    private enum CodingKeys: String, CodingKey {
        case defaultId = "default_id"
        case modes
        case type
    }
}

/// A note was saved to a record, in answer to note.add. Its words stay with the shell that sent
/// them.
public struct NoteAdded: Codable, Sendable, Equatable {
    /// Where in the record it was written, ms.
    public let atMs: Int64
    /// The new note's id.
    public let note: String
    /// The record.
    public let record: String
    /// The command's "id", when it had one, so the shell can match the note to what it sent.
    public let ref: String?
    /// Always `note.added`.
    public let type: String

    private enum CodingKeys: String, CodingKey {
        case atMs = "at_ms"
        case note
        case record
        case ref
        case type
    }
}

/// A note was deleted.
public struct NoteDeleted: Codable, Sendable, Equatable {
    /// The note's id.
    public let note: String
    /// The command's "id", when it had one, so the shell can match the answer to the line that
    /// sent it.
    public let ref: String?
    /// Always `note.deleted`.
    public let type: String
}

/// A note's text was replaced.
public struct NoteUpdated: Codable, Sendable, Equatable {
    /// The note's id.
    public let note: String
    /// The command's "id", when it had one, so the shell can match the answer to the line that
    /// sent it.
    public let ref: String?
    /// Always `note.updated`.
    public let type: String
}

/// Why a meeting's far end records this output (Windows): the one the user chose (chosen), the
/// default because the chosen one is not connected (chosen_missing), or the default output,
/// chosen (default_output).
public enum OutputReason: String, Codable, Sendable, Equatable, CaseIterable {
    case chosen
    case chosenMissing = "chosen_missing"
    case defaultOutput = "default_output"
}

/// An open commitment: not done, and not merged into another.
public struct OwedItem: Codable, Sendable, Equatable {
    /// Which side said it there.
    public let channel: Channel?
    /// When, as said.
    public let due: String?
    /// When, resolved to a time, Unix ms: what overdue is measured against.
    public let dueAtUnixMs: Int64?
    /// Its id.
    public let id: String
    /// Where a later meeting suggests it is already done, until the user answers.
    public let looksDone: DoneEvidence?
    /// How many other commitments were merged into it as the same promise said again.
    public let merged: Int64
    /// Who owes it, as said.
    public let owner: String?
    /// Who it is owed to, as said, when the transcript says.
    public let recipient: String?
    /// The record it was said in.
    public let record: String
    /// When that record started, Unix ms.
    public let recordStartedAtUnixMs: Int64
    /// That record's title, when it has one.
    public let recordTitle: String?
    /// Where in the record it was first said, ms.
    public let saidAtMs: Int64?
    /// What was promised.
    public let text: String

    private enum CodingKeys: String, CodingKey {
        case channel
        case due
        case dueAtUnixMs = "due_at_unix_ms"
        case id
        case looksDone = "looks_done"
        case merged
        case owner
        case recipient
        case record
        case recordStartedAtUnixMs = "record_started_at_unix_ms"
        case recordTitle = "record_title"
        case saidAtMs = "said_at_ms"
        case text
    }
}

/// A permission the app depends on.
public enum PermissionName: String, Codable, Sendable, Equatable, CaseIterable {
    case microphone
    case systemAudio = "system_audio"
    case accessibility
    case inputMonitoring = "input_monitoring"
}

/// permission.request showed the system prompt or opened the settings pane. The answer comes
/// later: check again when the user comes back.
public struct PermissionRequested: Codable, Sendable, Equatable {
    /// The permission asked for.
    public let permission: PermissionName
    /// Always `permission.requested`.
    public let type: String
}

/// A permission's state, as the platform's probe reads it without prompting.
public enum PermissionState: String, Codable, Sendable, Equatable, CaseIterable {
    case granted
    case denied
    case notDetermined = "not_determined"
    case unknown
}

/// Every permission's state now, in answer to permissions.check. Never prompts. System audio
/// reads not_determined until the app has asked for it, because checking it before would make
/// macOS prompt.
public struct PermissionsChecked: Codable, Sendable, Equatable {
    /// Typing into other apps, reading the focused app and the dictation key. Never asked reads
    /// as denied.
    public let accessibility: PermissionState
    /// A listen-only key tap, which the Mac app does not use. Never asked reads as denied.
    public let inputMonitoring: PermissionState
    /// Recording the microphone.
    public let microphone: PermissionState
    /// Recording other apps' sound (the far end), verified by listening for the app's own tone.
    public let systemAudio: PermissionState
    /// Always `permissions.checked`.
    public let type: String

    private enum CodingKeys: String, CodingKey {
        case accessibility
        case inputMonitoring = "input_monitoring"
        case microphone
        case systemAudio = "system_audio"
        case type
    }
}

/// Which part of a meeting a problem came from.
public enum Phase: String, Codable, Sendable, Equatable, CaseIterable {
    case live
    case `final`
}

/// Promises from meetings (commitments not merged into another), in Owed's states.
public struct PromiseStats: Codable, Sendable, Equatable {
    /// Marked done.
    public let kept: Int64
    /// Promises made: kept, open and overdue together.
    public let made: Int64
    /// Not done, and not past their due day.
    public let `open`: Int64
    /// Not done, past their due day (due today is not late).
    public let overdue: Int64
}

/// Where a record's audio is: its chunks per side, placed on its timeline.
public struct RecordAudio: Codable, Sendable, Equatable {
    /// Readable chunks, mic first then far end, each in order. Unreadable ones are left out
    /// (and logged).
    public let chunks: [AudioChunk]
    /// Chunk files left out of the player: unreadable, or whose format or place on the timeline
    /// recovery could only guess. Nonzero: some of the audio cannot be played or placed, and
    /// the shell says so.
    public let leftOut: Int64
    /// How the chunks were placed.
    public let timeline: AudioTimeline

    private enum CodingKeys: String, CodingKey {
        case chunks
        case leftOut = "left_out"
        case timeline
    }
}

/// A record was deleted whole, in answer to record.delete: its transcript, notes, summary,
/// commitments, speaker names and search entries, its words overwritten in the library's files
/// as the retention setting deletes them, then its audio. The screens drop it.
public struct RecordDeleted: Codable, Sendable, Equatable {
    /// Its recorded audio is still on disk: it could not be removed, or its folder is outside
    /// the library and was left alone. The record itself is gone.
    public let audioLeft: Bool
    /// What it was.
    public let kind: RecordKind
    /// The record that is gone.
    public let record: String
    /// The command's "id", when it had one, so the shell can match the answer to what it sent.
    public let ref: String?
    /// No copy of its words is left in the library's files. False, as for
    /// deleted_text_not_scrubbed, while another process reading the database keeps them in its
    /// log: the deletion is saved, and the library keeps trying; a dictation's or meeting's
    /// deleted_text_scrubbed says when it has. This answer counts as that change's report: each
    /// change reaches the shell once.
    public let scrubbed: Bool
    /// Always `record.deleted`.
    public let type: String

    private enum CodingKeys: String, CodingKey {
        case audioLeft = "audio_left"
        case kind
        case record
        case ref
        case scrubbed
        case type
    }
}

/// What produced a record: one dictation, a meeting (mic and far end), or an imported audio or
/// video file.
public enum RecordKind: String, Codable, Sendable, Equatable, CaseIterable {
    case dictation
    case meeting
    case fileImport = "file_import"
}

/// A note the user typed, stamped with where in the record it was written.
public struct RecordNote: Codable, Sendable, Equatable {
    /// Where in the record, ms from its start: the timestamp chip.
    public let atMs: Int64
    /// Its id.
    public let note: String
    /// The note. The user's words: never log it.
    public let text: String

    private enum CodingKeys: String, CodingKey {
        case atMs = "at_ms"
        case note
        case text
    }
}

/// A record as a list shows it.
public struct RecordRow: Codable, Sendable, Equatable {
    /// When it ended, Unix ms; absent while it is live.
    public let endedAtUnixMs: Int64?
    /// Whether it kept audio (a meeting's chunks).
    public let hasAudio: Bool
    /// What produced it.
    public let kind: RecordKind
    /// The first words of an untitled record's transcript. Carries the user's words: never log
    /// it.
    public let preview: String?
    /// Its id.
    public let record: String
    /// The transcript revision: 1 while live, 2 once the final pass replaced it.
    public let revision: Int64
    /// The application involved, when known.
    public let sourceApp: String?
    /// When it started, Unix ms.
    public let startedAtUnixMs: Int64
    /// Its title: a calendar event's, a file's name, or the summary's headline. Absent until
    /// one is known.
    public let title: String?

    private enum CodingKeys: String, CodingKey {
        case endedAtUnixMs = "ended_at_unix_ms"
        case hasAudio = "has_audio"
        case kind
        case preview
        case record
        case revision
        case sourceApp = "source_app"
        case startedAtUnixMs = "started_at_unix_ms"
        case title
    }
}

/// Settled transcript on a record's timeline. Carries the user's words: never log it.
public struct RecordSegment: Codable, Sendable, Equatable {
    /// Mic (you) or far end (them).
    public let channel: Channel
    /// End, ms from the record's start.
    public let endMs: Int64
    /// The diarized far-end speaker, when labels were kept.
    public let speaker: String?
    /// Start, ms from the record's start.
    public let startMs: Int64
    /// The words.
    public let text: String

    private enum CodingKeys: String, CodingKey {
        case channel
        case endMs = "end_ms"
        case speaker
        case startMs = "start_ms"
        case text
    }
}

/// A record's summary, as stored: markdown (a headline, then sections). Shells render it; it is
/// never shown raw.
public struct RecordSummary: Codable, Sendable, Equatable {
    /// When it was written, Unix ms.
    public let createdAtUnixMs: Int64
    /// Its decisions and actions with the line each cites, in the order the text lists them
    /// (empty for a summary saved before they were kept).
    public let items: [SummaryItemRow]
    /// The model that wrote it.
    public let model: String
    /// The markdown. The library's words: never log it.
    public let text: String

    private enum CodingKeys: String, CodingKey {
        case createdAtUnixMs = "created_at_unix_ms"
        case items
        case model
        case text
    }
}

/// Why a job's model was refused.
public enum Refusal: String, Codable, Sendable, Equatable, CaseIterable {
    case updating
}

/// A you line the final pass removed as echo of the far end. Its words are not here: the store
/// keeps the line with the record, at index in its removed lines (by start time), so it can be
/// put back.
public struct RemovedEchoLine: Codable, Sendable, Equatable {
    /// Its end.
    public let endMs: Int64
    /// The far-end lines whose words it matched, in time order.
    public let far: [EchoSpan]
    /// Its place in the record's removed lines.
    public let index: Int64
    /// How many of them matched the far end, in order.
    public let matched: Int64
    /// Its start, ms into the meeting.
    public let startMs: Int64
    /// Its words, normalised.
    public let words: Int64

    private enum CodingKeys: String, CodingKey {
        case endMs = "end_ms"
        case far
        case index
        case matched
        case startMs = "start_ms"
        case words
    }
}

/// How much confirmation a voice command needs: safe runs at once, moderate gets a brief notice
/// the user can cancel, dangerous asks first.
public enum Risk: String, Codable, Sendable, Equatable, CaseIterable {
    case safe
    case moderate
    case dangerous
    case other
}

/// A full-text match.
public struct SearchHit: Codable, Sendable, Equatable {
    /// The record.
    public let record: String
    /// The matching text. The library's words: never log it.
    public let snippet: String
    /// Where in the record the match is, ms.
    public let startMs: Int64
    /// When the record started, Unix ms.
    public let startedAtUnixMs: Int64
    /// The record's title, when it has one.
    public let title: String?

    private enum CodingKeys: String, CodingKey {
        case record
        case snippet
        case startMs = "start_ms"
        case startedAtUnixMs = "started_at_unix_ms"
        case title
    }
}

/// A shell setting's value, in answer to setting.get or setting.set.
public struct SettingValue: Codable, Sendable, Equatable {
    /// The setting.
    public let key: String
    /// Always `setting.value`.
    public let type: String
    /// Its value; absent when it has never been set.
    public let value: String?
}

/// What a meeting side is delivering, as the silent-channel watchdog judges it: ok, stopped (no
/// audio where it must keep coming), zeros (only exact zeros: no data at all, most often a
/// denied capture).
public enum SideState: String, Codable, Sendable, Equatable, CaseIterable {
    case ok
    case stopped
    case zeros
}

/// A snippet: a spoken trigger that dictation replaces with its expansion.
public struct SnippetInfo: Codable, Sendable, Equatable {
    /// A grouping, possibly empty.
    public let category: String
    /// Whether it expands.
    public let enabled: Bool
    /// What it becomes; {date}, {time} and {clipboard} are filled in.
    public let expansion: String
    /// Its id.
    public let id: String
    /// What the user says, matched as whole words in any case.
    public let trigger: String
}

/// The user's snippets, in answer to snippets.list or snippets.save. Carries the user's words:
/// never log it.
public struct SnippetsListed: Codable, Sendable, Equatable {
    /// They are the ones the Inkwell 0.2 import brought, not yet saved in 1.0.
    public let fromImport: Bool
    /// The command's id.
    public let ref: String?
    /// The snippets, in the user's order.
    public let snippets: [SnippetInfo]
    /// Always `snippets.listed`.
    public let type: String

    private enum CodingKeys: String, CodingKey {
        case fromImport = "from_import"
        case ref
        case snippets
        case type
    }
}

/// A stretch of a record: where something was said.
public struct Span: Codable, Sendable, Equatable {
    /// Which side said it.
    public let channel: Channel
    /// End, ms from the record's start.
    public let endMs: Int64
    /// Start, ms from the record's start.
    public let startMs: Int64

    private enum CodingKeys: String, CodingKey {
        case channel
        case endMs = "end_ms"
        case startMs = "start_ms"
    }
}

/// A diarized speaker the user named in one record.
public struct SpeakerName: Codable, Sendable, Equatable {
    /// The name the user gave.
    public let name: String
    /// The diarizer's label.
    public let speaker: String
}

/// A far-end speaker of a record was named, renamed or cleared, in answer to speaker.name. The
/// name stays with the shell that sent it: record.open carries it.
public struct SpeakerNamed: Codable, Sendable, Equatable {
    /// Whether the speaker has a name now: false when it was cleared, and reads as numbered
    /// again.
    public let named: Bool
    /// The record.
    public let record: String
    /// The command's "id", when it had one, so the shell can match the answer to what it sent.
    public let ref: String?
    /// The diarizer's label.
    public let speaker: String
    /// Always `speaker.named`.
    public let type: String
}

/// The Stats screen's numbers, in answer to stats.get, streak.pause or streak.resume: counted
/// on this computer from the library, on the user's calendar. Nothing here is sent anywhere or
/// drawn from what was said.
public struct StatsCounted: Codable, Sendable, Equatable {
    /// The user's personal bests, from takes made here (never an import's), in a fixed order:
    /// longest_dictation, fastest_dictation, most_words_day, best_week, longest_meeting,
    /// longest_monologue. A best not held yet is absent: nothing to show, never a zero.
    public let bests: [BestRow]?
    /// Dictation.
    public let dictation: DictationStats
    /// Meetings, all time.
    public let meetingsAll: MeetingStats
    /// Meetings that started this month.
    public let meetingsMonth: MeetingStats
    /// Every milestone, in a fixed order; with the streak hidden, the words milestones only.
    public let milestones: [MilestoneRow]
    /// Promises, all time.
    public let promisesAll: PromiseStats
    /// Promises made in meetings that started this month.
    public let promisesMonth: PromiseStats
    /// The id of the command this answers, echoed so the shell can match the answer to its
    /// question.
    public let ref: String?
    /// Today on the user's calendar, YYYY-MM-DD.
    public let today: String
    /// Always `stats.counted`.
    public let type: String
    /// The typing speed time saved is measured against (stats.typing_wpm, 40 unless set).
    public let typingWpm: Int64
    /// Last week, reviewed, to lead the screen with until the user dismisses it
    /// (stats.review_dismissed, its week's first day). Absent when last week had no dictation
    /// and no meeting, or once dismissed.
    public let weekReview: WeekReview?

    private enum CodingKeys: String, CodingKey {
        case bests
        case dictation
        case meetingsAll = "meetings_all"
        case meetingsMonth = "meetings_month"
        case milestones
        case promisesAll = "promises_all"
        case promisesMonth = "promises_month"
        case ref
        case today
        case type
        case typingWpm = "typing_wpm"
        case weekReview = "week_review"
    }
}

/// Whether a summary item is a decision or an action.
public enum SummaryItemKind: String, Codable, Sendable, Equatable, CaseIterable {
    case decision
    case action
}

/// A decision or an action in a summary, with the line it cites.
public struct SummaryItemRow: Codable, Sendable, Equatable {
    /// Which it is.
    public let kind: SummaryItemKind
    /// Where the line it cites was said.
    public let span: Span
    /// The item as the summary states it. The library's words: never log it.
    public let text: String
}

/// Time saved, pictured: about count of key, the nearest whole number, within a fifth of the
/// time. Always said with "about".
public struct TimeEquivalent: Codable, Sendable, Equatable {
    /// How many: 1 to 5, or any number of the largest (working_week).
    public let count: Int64
    /// What it is about.
    public let key: TimeEquivalentKey
}

/// Something time saved is about, as long as: working_week 40 h, working_day 8 h, feature_film
/// 2 h, lunch_hour 1 h, coffee_break 15 min.
public enum TimeEquivalentKey: String, Codable, Sendable, Equatable, CaseIterable {
    case workingWeek = "working_week"
    case workingDay = "working_day"
    case featureFilm = "feature_film"
    case lunchHour = "lunch_hour"
    case coffeeBreak = "coffee_break"
}

/// Why no voice detection model is in use.
public enum VadUnavailable: String, Codable, Sendable, Equatable, CaseIterable {
    case modelMissing = "model_missing"
    case downloading
    case loadFailed = "load_failed"
    case failed
    case other
}

/// A voice command and the phrases that trigger it.
public struct VoiceCommandInfo: Codable, Sendable, Equatable {
    /// What it does.
    public let action: CommandAction
    /// Whether this build carries the action out.
    public let carriedOut: Bool
    /// Whether it can match.
    public let enabled: Bool
    /// Its id.
    public let id: String
    /// The phrases, any of which triggers it after the wake word.
    public let triggers: [String]
    /// Its argument: a style, a model id, a URL, an app path, or fixed text.
    public let value: String?

    private enum CodingKeys: String, CodingKey {
        case action
        case carriedOut = "carried_out"
        case enabled
        case id
        case triggers
        case value
    }
}

/// The voice commands, in answer to voice_commands.list or voice_commands.save. Carries the
/// user's words: never log it.
public struct VoiceCommandsListed: Codable, Sendable, Equatable {
    /// The commands, in order of precedence.
    public let commands: [VoiceCommandInfo]
    /// Whether any dictation can be a command.
    public let enabled: Bool
    /// They are the ones the Inkwell 0.2 import brought, not yet saved in 1.0.
    public let fromImport: Bool
    /// The command's id.
    public let ref: String?
    /// Always `voice_commands.listed`.
    public let type: String
    /// The word a command starts with.
    public let wakePrefix: String

    private enum CodingKeys: String, CodingKey {
        case commands
        case enabled
        case fromImport = "from_import"
        case ref
        case type
        case wakePrefix = "wake_prefix"
    }
}

/// Last week, reviewed: gains and plain facts only, nothing said to be down.
public struct WeekReview: Codable, Sendable, Equatable {
    /// The day with the most words, YYYY-MM-DD (the earliest on a tie). Absent without words.
    public let bestDay: String?
    /// Its words.
    public let bestDayWords: Int64?
    /// Their length, ms.
    public let meetingMs: Int64
    /// Meetings recorded here.
    public let meetings: Int64
    /// Of the promises made in its meetings, those done now (no time is kept for when one was
    /// done). Absent when none.
    public let promisesKept: Int64?
    /// What saved_ms is about, as saved_about_week.
    public let savedAbout: [TimeEquivalent]?
    /// Time saved, ms, as saved_ms_week. Absent unless there was some.
    public let savedMs: Int64?
    /// Its first day, YYYY-MM-DD: what stats.review_dismissed takes to dismiss it (kept
    /// dismissed if the week's first day changes later).
    public let week: String
    /// Words dictated.
    public let words: Int64
    /// Words per minute, with at least a minute of speech.
    public let wpm: Int64?
    /// How many words a minute faster than the four weeks before it. Absent unless it was
    /// faster: a slower week is never compared.
    public let wpmGain: Int64?

    private enum CodingKeys: String, CodingKey {
        case bestDay = "best_day"
        case bestDayWords = "best_day_words"
        case meetingMs = "meeting_ms"
        case meetings
        case promisesKept = "promises_kept"
        case savedAbout = "saved_about"
        case savedMs = "saved_ms"
        case week
        case words
        case wpm
        case wpmGain = "wpm_gain"
    }
}
