// What the shell knows about the core, built only from the core's events. The screens read it and
// never ask the core for state: the core pushes, the shell renders (architecture rule 1).
//
// An EventRelay feeds it batches on the main actor; `apply` folds a batch in one pass, so SwiftUI
// sees one change per batch however many events it held. Event payloads can carry the user's
// words (partials, finals, inserted text): the store holds them for the screens and never logs
// them.
import Foundation
import InkCore
import Observation

/// The shell's view of the core. Main actor only; fed by `apply`.
@MainActor
@Observable
public final class CoreStore {
    /// Whether the core is running.
    public enum Status: Equatable, Sendable {
        /// `ink_init` was called and `core.ready` has not arrived yet.
        case starting
        /// `core.ready`, with the core's version.
        case ready(version: String)
        /// `core.stopped`: `ink_shutdown` ran.
        case stopped
        /// The core did not start, or it is not the core this shell was built with.
        case failed(String)
    }

    /// Where a dictation is.
    public enum DictationPhase: Equatable, Sendable {
        case idle
        /// The key is held and the mic is recorded.
        case listening
        /// The key was released; the take is being transcribed and inserted.
        case transcribing
    }

    /// The take being held or processed: what the Drop shows beside the ink.
    public struct LiveDictation: Equatable, Sendable {
        /// The take's number (`dictation.started`), which its partials carry.
        public let take: Int64
        /// A voice edit rather than a dictation.
        public let edit: Bool
        /// The mode it is expected to write in, by name.
        public let mode: String?
        /// The app in front when it started, by name.
        public let app: String?
        /// What the live engine hears so far, while the key is held. The user's words: shown,
        /// never logged, gone when the take stops.
        public var partial: String?

        public init(take: Int64, edit: Bool, mode: String?, app: String?, partial: String? = nil) {
            self.take = take
            self.edit = edit
            self.mode = mode
            self.app = app
            self.partial = partial
        }
    }

    /// How the last dictation ended.
    public enum DictationOutcome: Equatable, Sendable {
        case inserted(InsertOutcome)
        case discarded(Discard)
        case failed(FailedStage)
    }

    /// The meeting being recorded, or finishing (its final pass runs after `meeting.stopped`).
    public struct LiveMeeting: Equatable, Sendable {
        /// The record it is written to.
        public let record: String
        /// Its title, when the core knew one at the start (a calendar event, a replay's name).
        public var title: String?
        /// The app it records, by id, and that app's name, when it was started for one.
        public var app: String?
        public var appName: String?
        /// The microphone it records, and why that one.
        public var micName: String?
        public var micReason: MicReason?
        /// Its mic went mid-meeting and another records now (`meeting.mic_switched`): said until
        /// the new mic's first line.
        public var micSwitch: MicSwitch?
        /// What it records as the other side: the app alone, or everything this Mac plays.
        public var farEnd: FarEnd?
        /// It was started for an app whose sound could not be recorded alone, so it records
        /// everything this Mac plays instead (`meeting.far_end_fallback`).
        public var farEndFallback = false
        /// `meeting.stopped` arrived: capture ended and the final pass is running.
        public var stopping = false
        /// The final pass's progress: the sides it has transcribed (`meeting.transcribed`), and
        /// whether it has told the far end's speakers apart (`meeting.diarized`) and written the
        /// summary (`meeting.summarized`). Only what the core said: a step it skips never shows.
        public var transcribed: Set<Channel> = []
        public var diarized = false
        public var summarized = false
        /// The latest state of each side's capture.
        public var sides: [Channel: SideState] = [:]
        /// Each channel's current partial: replaced by the next one, cleared by its final.
        public var partials: [Channel: String] = [:]
        /// The newest live finals, oldest first: at most `finalsKept` of them. The record in the
        /// store is the lasting copy of all of them (RAM holds a window, never the session).
        public private(set) var finals: [MeetingFinal] = []
        /// What the ledger holds and has let go of, for the dogfood week's measurement.
        public private(set) var ledger = LedgerStats()

        /// The most finals kept in memory: an hour's meeting has about 600 to 1,000.
        public static let finalsKept = 500

        public init(record: String) {
            self.record = record
        }

        /// Adds a final, letting go of the oldest past `finalsKept`.
        mutating func append(_ final: MeetingFinal) {
            finals.append(final)
            ledger.seen += 1
            ledger.bytes += final.text.utf8.count
            if finals.count > Self.finalsKept {
                let gone = finals.removeFirst()
                ledger.dropped += 1
                ledger.bytes -= gone.text.utf8.count
            }
            ledger.peakBytes = max(ledger.peakBytes, ledger.bytes)
        }
    }

    /// The live ledger's size (counts and bytes only, never text): what the shell's RAM holds of
    /// a meeting's words, logged when the meeting ends.
    public struct LedgerStats: Equatable, Sendable {
        /// Finals received.
        public var seen = 0
        /// Finals let go of (older than the window).
        public var dropped = 0
        /// UTF-8 bytes of the finals held now.
        public var bytes = 0
        /// The most bytes held at once.
        public var peakBytes = 0
    }

    /// A meeting's mic that went, and the one recording in its place.
    public struct MicSwitch: Equatable, Sendable {
        /// The mic that went, as the OS named it, when known.
        public let from: String?
        /// The mic recording now.
        public let to: String

        public init(from: String?, to: String) {
            self.from = from
            self.to = to
        }
    }

    /// The mic the user chose isn't connected, and another opened in its place
    /// (`audio.input_fallback`, said once by the core per spell).
    public struct MicFallback: Equatable, Sendable {
        /// The chosen mic's name, when the core remembers one.
        public let wanted: String?
        /// The mic recording instead.
        public let using: String

        public init(wanted: String?, using: String) {
            self.wanted = wanted
            self.using = using
        }
    }

    /// An app the core offers to record (the consent Drop).
    public struct Offer: Equatable, Sendable {
        public let app: String
        public let appName: String
    }

    /// Something the user may need to know or act on (the needs-you banner reads these).
    public struct Notice: Identifiable, Equatable, Sendable {
        public enum Kind: Equatable, Sendable {
            case commandFailed(command: String)
            case modelWarmFailed(Job)
            case modelRefused(Job)
            case modelUpdateFailed(model: String)
            case dictationFailed(FailedStage)
            case dictationWarning(DictationWarning)
            case hotkeyLost
            case editKeyLost
            case dictationWorkerFailed(recovered: Bool)
            case voiceDetectionUnavailable(VadUnavailable?)
            case audioDropped(Chain)
            case meetingWarning(MeetingWarning)
            case meetingFailed
            case meetingCaptureFailed
            case meetingWorkerFailed
            /// A meeting a crash interrupted was finished at launch.
            case meetingRecovered
            /// The meetings a crash may have interrupted could not be looked for.
            case recoveryUnavailable
            /// Detection stopped on its own, or could not start.
            case detectionUnavailable
            /// The retention setting deleted records (a count).
            case librarySwept(deleted: Int64, failed: Int64)
            /// A voice command was heard that this build recognises but does not carry out.
            case voiceCommandNotCarriedOut(CommandAction)
            /// An event this shell cannot read: the core and the shell come from different builds.
            case mismatchedBuild(type: String)
        }

        /// Increases by one per notice.
        public let id: Int
        public let kind: Kind
        /// The core's own message, when it sent one. Core messages never quote the user's words.
        public let detail: String?
    }

    /// The most notices kept; older ones are dropped first.
    public static let noticeLimit = 50

    public private(set) var status: Status = .starting
    /// The engines the shell registered, by id, with the jobs they fill.
    public private(set) var engines: [String: [JobScore]] = [:]
    /// The model kept warm for each job, by id.
    public private(set) var warmModels: [Job: String] = [:]
    /// Models whose files are being replaced.
    public private(set) var updatingModels: Set<String> = []
    public private(set) var meeting: LiveMeeting?
    /// The record of the last meeting that finished.
    public private(set) var lastRecord: String?
    /// The app the core offers to record now, if any (the consent Drop).
    public private(set) var offer: Offer?
    /// Whether the core listens for calls: nil until it says.
    public private(set) var listening: Bool?
    /// The ledger of the meeting that ended last, and its record (the dogfood measurement reads
    /// it).
    public private(set) var lastLedger: (record: String, stats: LedgerStats)?
    public private(set) var dictation: DictationPhase = .idle
    /// The take in progress, while there is one.
    public private(set) var liveDictation: LiveDictation?
    public private(set) var lastDictation: DictationOutcome?
    /// A chosen mic that isn't connected, with the one recording instead: the Drop says so for the
    /// take or meeting it opened for, then it is let go of. Gone too when the devices say the
    /// chosen mic is back (or the choice changed).
    public private(set) var micFallback: MicFallback?
    public private(set) var notices: [Notice] = []

    /// Events applied so far (tests and diagnostics; not observed).
    @ObservationIgnored public private(set) var eventsApplied = 0
    /// Batches applied so far (tests and diagnostics; not observed).
    @ObservationIgnored public private(set) var batchesApplied = 0
    @ObservationIgnored private var nextNoticeID = 1

    public init() {}

    /// Folds a batch of events in, oldest first.
    public func apply(_ batch: [InkEvent]) {
        for event in batch {
            apply(event)
        }
        eventsApplied += batch.count
        batchesApplied += 1
    }

    /// `ink_init` refused to start (or the shell could not call it). `reason` is shown to the user.
    public func startFailed(_ reason: String) {
        status = .failed(reason)
    }

    /// Removes a notice the user dismissed.
    public func dismissNotice(_ id: Notice.ID) {
        notices.removeAll { $0.id == id }
    }

    private func notice(_ kind: Notice.Kind, _ detail: String? = nil) {
        notices.append(Notice(id: nextNoticeID, kind: kind, detail: detail))
        nextNoticeID += 1
        if notices.count > Self.noticeLimit {
            notices.removeFirst(notices.count - Self.noticeLimit)
        }
    }

    private func apply(_ event: InkEvent) {
        switch event {
        // The core
        case .coreReady(let ready):
            // The shell and the core ship together; another ABI means a broken build, and no
            // command from this shell can be trusted to mean the same thing to that core.
            status = ready.abi == Int64(INK_ABI_VERSION)
                ? .ready(version: ready.version)
                : .failed("This app was built against core ABI \(INK_ABI_VERSION), and the core reports \(ready.abi).")
        case .coreStopped:
            status = .stopped
            meeting = nil
            offer = nil
            listening = nil
            dictation = .idle
            liveDictation = nil
        case .commandFailed(let failed):
            notice(.commandFailed(command: failed.command), failed.message)

        // Engines and models
        case .engineRegistered(let engine):
            engines[engine.id] = engine.jobs
        case .engineUnregistered(let engine):
            engines[engine.id] = nil
        case .modelWarmed(let warmed):
            warmModels[warmed.job] = warmed.id
        case .modelWarmFailed(let failed):
            warmModels[failed.job] = nil
            notice(.modelWarmFailed(failed.job), failed.message)
        case .modelRefused(let refused):
            notice(.modelRefused(refused.job))
        case .modelUpdateStarted(let update):
            updatingModels.insert(update.id)
            // Unloaded for the update: no job has it warm until the update warms the new one.
            warmModels = warmModels.filter { $0.value != update.id }
        case .modelUpdateFinished(let update):
            updatingModels.remove(update.id)
            if !update.ok {
                notice(.modelUpdateFailed(model: update.id), update.message)
            }
        case .audioDropped(let dropped):
            notice(.audioDropped(dropped.chain))

        // Dictation
        case .dictationStarted(let started):
            dictation = .listening
            liveDictation = LiveDictation(take: started.take, edit: started.edit, mode: started.mode, app: started.app)
        case .dictationPartial(let partial):
            // Only the take being held: a late partial of an earlier take is never shown.
            if dictation == .listening, liveDictation?.take == partial.take {
                liveDictation?.partial = partial.text
            }
        case .dictationStopped:
            dictation = .transcribing
            liveDictation?.partial = nil
        case .dictationShortPressIgnored:
            endDictation()
        case .dictationInserted(let inserted):
            endDictation()
            lastDictation = .inserted(inserted.outcome)
        case .dictationDiscarded(let discarded):
            endDictation()
            lastDictation = .discarded(discarded.reason)
        case .dictationFailed(let failed):
            endDictation()
            lastDictation = .failed(failed.stage)
            notice(.dictationFailed(failed.stage), failed.message)
        case .dictationCommand(let command):
            // A voice command ends its take like any other outcome (the Drop would otherwise
            // stay on "Transcribing").
            endDictation()
            // Heard, and nothing typed: never silently.
            if command.carriedOut == false {
                notice(.voiceCommandNotCarriedOut(command.action))
            }
        case .dictationEdited, .dictationEditFailed, .dictationMicFailed:
            // The Drop says how an edit or a failed mic went (DictationModel).
            endDictation()
        case .dictationEditHotkeyLost:
            notice(.editKeyLost)
        case .dictationVoiceDetection(let vad):
            if !vad.available {
                notice(.voiceDetectionUnavailable(vad.reason))
            }
        case .dictationWarningEvent(let warning):
            notice(.dictationWarning(warning.kind), warning.message)
        case .dictationHotkeyLost:
            endDictation()
            notice(.hotkeyLost)
        case .dictationWorkerFailed(let failed):
            endDictation()
            notice(.dictationWorkerFailed(recovered: failed.recovered))

        // Meetings
        case .meetingStarted(let started):
            var live = LiveMeeting(record: started.record)
            live.title = started.title
            live.app = started.app
            live.appName = started.appName
            live.micName = started.micName
            live.micReason = started.micReason
            live.farEnd = started.farEnd
            meeting = live
            offer = nil
        case .meetingDetected(let detected):
            // Only while nothing is recorded: the core never offers during a meeting.
            if meeting == nil {
                offer = Offer(app: detected.app, appName: detected.appName)
            }
        case .meetingDetectionEnded(let ended):
            if offer?.app == ended.app {
                offer = nil
            }
        case .meetingDetection(let detection):
            listening = detection.listening
            if !detection.listening {
                offer = nil
                if let message = detection.message {
                    notice(.detectionUnavailable, message)
                }
            }
        case .meetingMicSwitched(let switched):
            updateMeeting(switched.record) {
                $0.micName = switched.micName
                $0.micReason = switched.micReason
                $0.micSwitch = MicSwitch(from: switched.fromName, to: switched.micName)
            }
        case .audioInputFallback(let fallback):
            micFallback = MicFallback(wanted: fallback.wanted.name, using: fallback.micName)
        case .audioDevices(let devices) where devices.using?.reason != .chosenMissing:
            micFallback = nil
        case .audioDevicesChanged(let devices) where devices.using?.reason != .chosenMissing:
            micFallback = nil
        case .meetingFarEndFallback(let fallback):
            updateMeeting(fallback.record) { $0.farEndFallback = true }
        case .meetingRecovered:
            notice(.meetingRecovered)
        case .meetingsRecovered(let done):
            if let message = done.message {
                notice(.recoveryUnavailable, message)
            }
        case .librarySwept(let swept):
            notice(.librarySwept(deleted: swept.deleted, failed: swept.failed))
        case .meetingSideState(let side):
            updateMeeting(side.record) { $0.sides[side.channel] = side.state }
        case .meetingPartial(let partial):
            updateMeeting(partial.record) { $0.partials[partial.channel] = partial.text }
        case .meetingFinal(let final):
            updateMeeting(final.record) {
                $0.partials[final.channel] = nil
                $0.append(final)
                // The new mic is heard: the switch has been said long enough.
                if final.channel == .mic { $0.micSwitch = nil }
            }
        case .meetingStopped(let stopped):
            updateMeeting(stopped.record) {
                $0.stopping = true
                $0.partials = [:]
            }
        case .meetingVoiceDetection(let vad):
            if !vad.available {
                notice(.voiceDetectionUnavailable(vad.reason))
            }
        case .meetingTranscribed(let done):
            updateMeeting(done.record) { $0.transcribed.insert(done.pass.channel) }
        case .meetingDiarized(let done):
            updateMeeting(done.record) { $0.diarized = true }
        case .meetingSummarized(let done):
            updateMeeting(done.record) { $0.summarized = true }
        case .meetingWarningEvent(let warning):
            notice(.meetingWarning(warning.kind), warning.message)
        case .meetingFinished(let finished):
            lastRecord = finished.record
            endMeeting(finished.record)
        case .meetingFailed(let failed):
            notice(.meetingFailed, failed.message)
            // A failure without a record is a meeting that never started.
            if let record = failed.record {
                endMeeting(record)
            }
        case .meetingCaptureFailed:
            notice(.meetingCaptureFailed)
        case .meetingWorkerFailed(let failed):
            notice(.meetingWorkerFailed)
            endMeeting(failed.record)
        case .unknown(let type):
            notice(.mismatchedBuild(type: type))
        case .undecodable(let type, _):
            notice(.mismatchedBuild(type: type))
        // Nothing to keep: the rest of the final pass (superseded, kept live, commitments), whose
        // screens read the record from the store, and voice commands. An event added to the schema later lands here too until the store
        // learns it, so a new event never breaks the shell's build.
        default:
            break
        }
    }

    private func endDictation() {
        dictation = .idle
        liveDictation = nil
        // Said for the take whose mic it was; a meeting recording says it on its own.
        if meeting == nil { micFallback = nil }
    }

    /// Changes the live meeting when `record` is the one live. An event for another record (one
    /// that ended, or a mismatched build) changes nothing.
    private func updateMeeting(_ record: String, _ change: (inout LiveMeeting) -> Void) {
        guard var live = meeting, live.record == record else { return }
        change(&live)
        meeting = live
    }

    private func endMeeting(_ record: String) {
        if let live = meeting, live.record == record {
            lastLedger = (record, live.ledger)
            meeting = nil
            micFallback = nil
        }
    }
}
