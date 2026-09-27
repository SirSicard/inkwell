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
    /// `meeting.started`
    case meetingStarted(MeetingStarted)
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
    /// `models.listed`
    case modelsListed(ModelsListed)
    /// `setting.value`
    case settingValue(SettingValue)
    /// `modes.listed`
    case modesListed(ModesListed)
    /// `library.records`
    case libraryRecords(LibraryRecords)
    /// `library.search`
    case librarySearch(LibrarySearch)
    /// `library.record`
    case libraryRecord(LibraryRecord)
    /// `library.stats`
    case libraryStats(LibraryStats)
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
            case "meeting.started": self = .meetingStarted(try MeetingStarted(from: decoder))
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
            case "models.listed": self = .modelsListed(try ModelsListed(from: decoder))
            case "setting.value": self = .settingValue(try SettingValue(from: decoder))
            case "modes.listed": self = .modesListed(try ModesListed(from: decoder))
            case "library.records": self = .libraryRecords(try LibraryRecords(from: decoder))
            case "library.search": self = .librarySearch(try LibrarySearch(from: decoder))
            case "library.record": self = .libraryRecord(try LibraryRecord(from: decoder))
            case "library.stats": self = .libraryStats(try LibraryStats(from: decoder))
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
        case .meetingStarted(let event): try event.encode(to: encoder)
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
        case .modelsListed(let event): try event.encode(to: encoder)
        case .settingValue(let event): try event.encode(to: encoder)
        case .modesListed(let event): try event.encode(to: encoder)
        case .libraryRecords(let event): try event.encode(to: encoder)
        case .librarySearch(let event): try event.encode(to: encoder)
        case .libraryRecord(let event): try event.encode(to: encoder)
        case .libraryStats(let event): try event.encode(to: encoder)
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

/// How a record's chunks were placed on its timeline: recorded (from the start the meeting
/// wrote beside them) or estimated (from its earliest chunk: a record from before that was
/// written).
public enum AudioTimeline: String, Codable, Sendable, Equatable, CaseIterable {
    case recorded
    case estimated
}

/// A model in the catalogue that runs on this OS.
public struct CatalogueEntry: Codable, Sendable, Equatable {
    /// Its id.
    public let id: String
    /// Whether its files are installed and complete.
    public let installed: Bool
    /// The jobs it fills, each with its measured error rate.
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

/// A voice command. change_style and toggle_polish are already applied by the core; the rest
/// are the shell's to carry out.
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
    /// The commitment it was folded into ("said twice"), when it was.
    public let mergedInto: String?
    /// Who owes it, as said.
    public let owner: String?
    /// Where in the record it was said.
    public let provenance: [Span]
    /// The record it came from.
    public let record: String
    /// What was promised. The library's words: never log it.
    public let text: String

    private enum CodingKeys: String, CodingKey {
        case commitment
        case done
        case due
        case dueAtUnixMs = "due_at_unix_ms"
        case mergedInto = "merged_into"
        case owner
        case provenance
        case record
        case text
    }
}

/// A commitment was marked done, or open again.
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
    /// How much confirmation it needs.
    public let risk: Risk
    /// Always `dictation.command`.
    public let type: String
    /// Its argument: a style, a model id, a URL, an app path, or fixed text.
    public let value: String?
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

/// A press shorter than the minimum hold (a modifier used in a shortcut). Nothing was shown or
/// transcribed.
public struct DictationShortPressIgnored: Codable, Sendable, Equatable {
    /// Always `dictation.short_press_ignored`.
    public let type: String
}

/// A hold passed the minimum and is now a take: show that it is listening.
public struct DictationStarted: Codable, Sendable, Equatable {
    /// Always `dictation.started`.
    public let type: String
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
/// deleted_text_not_scrubbed and deleted_text_scrubbed: as for meetings.
public enum DictationWarning: String, Codable, Sendable, Equatable, CaseIterable {
    case vadFailed = "vad_failed"
    case audioLost = "audio_lost"
    case tailCutShort = "tail_cut_short"
    case focusUnreadable = "focus_unreadable"
    case polishUnavailable = "polish_unavailable"
    case polishFailed = "polish_failed"
    case polishTimedOut = "polish_timed_out"
    case noModeForStyle = "no_mode_for_style"
    case saveFailed = "save_failed"
    case deletedTextNotScrubbed = "deleted_text_not_scrubbed"
    case deletedTextScrubbed = "deleted_text_scrubbed"
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

/// A meeting's record exists and its capture is being transcribed live.
public struct MeetingStarted: Codable, Sendable, Equatable {
    /// The meeting's record id.
    public let record: String
    /// Always `meeting.started`.
    public let type: String
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
/// deleted_text_scrubbed: it now is. Each is sent once per change.
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
    case commitmentsFailed = "commitments_failed"
    case echoOnlyFinal = "echo_only_final"
    case echoGateVadFailed = "echo_gate_vad_failed"
    case echoFailed = "echo_failed"
    case echoPathNotFound = "echo_path_not_found"
    case deletedTextNotScrubbed = "deleted_text_not_scrubbed"
    case deletedTextScrubbed = "deleted_text_scrubbed"
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
    /// How many other commitments were merged into it as the same promise said again.
    public let merged: Int64
    /// Who owes it, as said.
    public let owner: String?
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
        case merged
        case owner
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

/// Where a record's audio is: its chunks per side, placed on its timeline.
public struct RecordAudio: Codable, Sendable, Equatable {
    /// Readable chunks, mic first then far end, each in order. Unreadable ones are left out
    /// (and logged).
    public let chunks: [AudioChunk]
    /// How the chunks were placed.
    public let timeline: AudioTimeline
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
    /// The model that wrote it.
    public let model: String
    /// The markdown. The library's words: never log it.
    public let text: String

    private enum CodingKeys: String, CodingKey {
        case createdAtUnixMs = "created_at_unix_ms"
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

/// Why no voice detection model is in use.
public enum VadUnavailable: String, Codable, Sendable, Equatable, CaseIterable {
    case modelMissing = "model_missing"
    case downloading
    case loadFailed = "load_failed"
    case failed
    case other
}
