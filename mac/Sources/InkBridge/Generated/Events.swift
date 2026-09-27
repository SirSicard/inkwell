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
            case "meeting.superseded": self = .meetingSuperseded(try MeetingSuperseded(from: decoder))
            case "meeting.kept_live": self = .meetingKeptLive(try MeetingKeptLive(from: decoder))
            case "meeting.summarized": self = .meetingSummarized(try MeetingSummarized(from: decoder))
            case "meeting.commitments": self = .meetingCommitments(try MeetingCommitments(from: decoder))
            case "meeting.finished": self = .meetingFinished(try MeetingFinished(from: decoder))
            case "meeting.failed": self = .meetingFailed(try MeetingFailed(from: decoder))
            case "meeting.capture_failed": self = .meetingCaptureFailed(try MeetingCaptureFailed(from: decoder))
            case "meeting.worker_failed": self = .meetingWorkerFailed(try MeetingWorkerFailed(from: decoder))
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
        case .meetingSuperseded(let event): try event.encode(to: encoder)
        case .meetingKeptLive(let event): try event.encode(to: encoder)
        case .meetingSummarized(let event): try event.encode(to: encoder)
        case .meetingCommitments(let event): try event.encode(to: encoder)
        case .meetingFinished(let event): try event.encode(to: encoder)
        case .meetingFailed(let event): try event.encode(to: encoder)
        case .meetingCaptureFailed(let event): try event.encode(to: encoder)
        case .meetingWorkerFailed(let event): try event.encode(to: encoder)
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
public enum DictationWarning: String, Codable, Sendable, Equatable, CaseIterable {
    case vadFailed = "vad_failed"
    case audioLost = "audio_lost"
    case tailCutShort = "tail_cut_short"
    case focusUnreadable = "focus_unreadable"
    case polishUnavailable = "polish_unavailable"
    case polishFailed = "polish_failed"
    case noModeForStyle = "no_mode_for_style"
    case saveFailed = "save_failed"
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

/// Something went wrong in a meeting, and it went on without it.
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
    case other
}

/// Something went wrong in a meeting, and it went on without it. Which fields are present
/// depends on kind.
public struct MeetingWarningEvent: Codable, Sendable, Equatable {
    /// Time above the audible floor, ms.
    public let audibleMs: Int64?
    /// Which side.
    public let channel: Channel?
    /// Chunk files skipped or cut short.
    public let chunks: Int64?
    /// Events counted.
    public let count: Int64?
    /// End of the stretch.
    public let endMs: Int64?
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

/// Which part of a meeting a problem came from.
public enum Phase: String, Codable, Sendable, Equatable, CaseIterable {
    case live
    case `final`
}

/// Why a job's model was refused.
public enum Refusal: String, Codable, Sendable, Equatable, CaseIterable {
    case updating
}

/// How much confirmation a voice command needs: safe runs at once, moderate gets a brief notice
/// the user can cancel, dangerous asks first.
public enum Risk: String, Codable, Sendable, Equatable, CaseIterable {
    case safe
    case moderate
    case dangerous
    case other
}

/// What a meeting side is delivering, as the silent-channel watchdog judges it: ok, stopped (no
/// audio where it must keep coming), zeros (only exact zeros: no data at all, most often a
/// denied capture).
public enum SideState: String, Codable, Sendable, Equatable, CaseIterable {
    case ok
    case stopped
    case zeros
}

/// Why no voice detection model is in use.
public enum VadUnavailable: String, Codable, Sendable, Equatable, CaseIterable {
    case modelMissing = "model_missing"
    case downloading
    case loadFailed = "load_failed"
    case failed
    case other
}
