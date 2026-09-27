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
        /// `meeting.stopped` arrived: capture ended and the final pass is running.
        public var stopping = false
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
    public private(set) var lastDictation: DictationOutcome?
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
        case .dictationStarted:
            dictation = .listening
        case .dictationStopped:
            dictation = .transcribing
        case .dictationShortPressIgnored:
            dictation = .idle
        case .dictationInserted(let inserted):
            dictation = .idle
            lastDictation = .inserted(inserted.outcome)
        case .dictationDiscarded(let discarded):
            dictation = .idle
            lastDictation = .discarded(discarded.reason)
        case .dictationFailed(let failed):
            dictation = .idle
            lastDictation = .failed(failed.stage)
            notice(.dictationFailed(failed.stage), failed.message)
        case .dictationVoiceDetection(let vad):
            if !vad.available {
                notice(.voiceDetectionUnavailable(vad.reason))
            }
        case .dictationWarningEvent(let warning):
            notice(.dictationWarning(warning.kind), warning.message)
        case .dictationHotkeyLost:
            dictation = .idle
            notice(.hotkeyLost)
        case .dictationWorkerFailed(let failed):
            dictation = .idle
            notice(.dictationWorkerFailed(recovered: failed.recovered))

        // Meetings
        case .meetingStarted(let started):
            var live = LiveMeeting(record: started.record)
            live.title = started.title
            live.app = started.app
            live.appName = started.appName
            live.micName = started.micName
            live.micReason = started.micReason
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
        // Nothing to keep: the final pass's progress (transcribed, diarized, superseded, kept
        // live, summarized, commitments), whose screens read the record from the store, and
        // voice commands. An event added to the schema later lands here too until the store
        // learns it, so a new event never breaks the shell's build.
        default:
            break
        }
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
        }
    }
}
