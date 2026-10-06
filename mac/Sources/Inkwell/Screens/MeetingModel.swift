// Meetings as the user drives them (S2.8): record now, record the call the Drop offers or say
// not this one, stop, stop and delete, and the settings that shape them: the headset mic, and how
// long the library keeps records (the call policies are CallPolicyModel's).
//
// Recording starts when the user asks (the Drop's "Record this call", Today's "Record now", the
// menu), or for an app the user chose Always for: the core starts that one itself, and the Drop
// shows it with Stop, and Stop and delete for its first minute. A meeting's title comes from the
// calendar when a call is on it now, read without prompting; otherwise the summary's headline
// names it later.
import EventKit
import Foundation
import InkBridge
import Observation

/// Where a call's title comes from when a meeting starts.
@MainActor
protocol CallTitles {
    /// The title of the calendar event going on at `now`, if there is one and the calendar may be
    /// read. Never prompts.
    func titleNow(_ now: Date) -> String?
}

/// The calendar's event going on now: started (or starting within five minutes) and not over.
struct EventKitCallTitles: CallTitles {
    func titleNow(_ now: Date) -> String? {
        guard EKEventStore.authorizationStatus(for: .event) == .fullAccess else { return nil }
        let store = EKEventStore()
        let predicate = store.predicateForEvents(
            withStart: now.addingTimeInterval(-6 * 3600), end: now.addingTimeInterval(300), calendars: nil)
        return Self.pick(
            store.events(matching: predicate)
                .filter { !$0.isAllDay && $0.status != .canceled }
                .map { (title: $0.title ?? "", start: $0.startDate, end: $0.endDate) },
            now: now)
    }

    /// Of `events`, the one on now whose start is nearest; nil when none is on.
    nonisolated static func pick(_ events: [(title: String, start: Date, end: Date)], now: Date) -> String? {
        events
            .filter { $0.start <= now.addingTimeInterval(300) && $0.end > now }
            .filter { !$0.title.trimmingCharacters(in: .whitespaces).isEmpty }
            .min { abs($0.start.timeIntervalSince(now)) < abs($1.start.timeIntervalSince(now)) }?
            .title
    }
}

/// How long the library keeps records (`retention.days`).
enum Retention: String, CaseIterable, Identifiable, Sendable {
    case forever
    case week = "7"
    case month = "30"
    case quarter = "90"
    case year = "365"

    var id: String { rawValue }

    var title: String {
        switch self {
        case .forever: "Forever"
        case .week: "A week"
        case .month: "30 days"
        case .quarter: "90 days"
        case .year: "A year"
        }
    }
}

@MainActor
@Observable
final class MeetingModel {
    /// Where a meeting command was asked for, so its failure shows there and nowhere else.
    enum Origin: Equatable, Sendable {
        /// Record now (Today, or Live with no meeting).
        case recordNow
        /// "Record this call", on the Drop.
        case offer
        /// "Not this one", on the Drop.
        case dismiss
        /// Stop, in Live.
        case stop
        /// Stop, on the Drop of a call recorded by its app's Always.
        case dropStop
        /// "Stop and delete", on that Drop.
        case discard
    }

    /// The places that show a meeting command's failure.
    enum Place: Equatable, Sendable {
        /// Record now's button (Today's foot, Live with no meeting).
        case recordNow
        /// The Drop, whose buttons answer an offer.
        case drop
        /// Live's header, beside Stop.
        case liveStop
    }

    /// The last meeting command's failure: where it was asked, and the core's words (they never
    /// quote the user).
    struct Failure: Equatable, Sendable {
        let origin: Origin
        let message: String
    }

    private(set) var failure: Failure?
    /// The meeting Stop and delete can still delete: one its app's Always started, until its
    /// delete_until_unix_ms. Cleared at that moment by one scheduled wake, never by polling.
    private(set) var deletable: String?
    /// The meeting being stopped and deleted, until the core says it is gone (or refuses). Live
    /// reads it: notes typed in it are never saved into a record being deleted.
    private(set) var discarding: String?
    /// With Bluetooth output, record the headset's own mic.
    private(set) var headsetMic = false
    /// How long the library keeps records; nil until the store answers.
    private(set) var retention: Retention?
    /// A setting could not be read or saved: its control says so.
    private(set) var settingsFailed = false

    @ObservationIgnored private let send: SendCommand
    @ObservationIgnored private let titles: any CallTitles
    @ObservationIgnored private let now: () -> Date
    @ObservationIgnored private let log: ScreenLog
    /// Sleeps until a deadline (tests shorten it).
    @ObservationIgnored private let sleep: @Sendable (Duration) async throws -> Void
    /// Where the start in flight was asked for (a start's command id is the same from both).
    @ObservationIgnored private var starting: Origin = .recordNow
    /// Where the stop in flight was asked for.
    @ObservationIgnored private var stopping: Origin = .stop
    /// The one wake that ends Stop and delete's minute.
    @ObservationIgnored private var deleteDeadline: Task<Void, Never>?

    init(
        send: @escaping SendCommand, titles: any CallTitles = EventKitCallTitles(),
        now: @escaping () -> Date = Date.init, log: ScreenLog = .system,
        sleep: @escaping @Sendable (Duration) async throws -> Void = { try await Task.sleep(for: $0) }
    ) {
        self.send = send
        self.titles = titles
        self.now = now
        self.log = log
        self.sleep = sleep
    }

    isolated deinit {
        deleteDeadline?.cancel()
    }

    /// What `place` says about the last failure, or nil when it was not asked there.
    func failure(on place: Place) -> String? {
        guard let failure else { return nil }
        switch (place, failure.origin) {
        case (.recordNow, .recordNow), (.drop, .offer):
            return "Couldn't start recording: \(failure.message)"
        case (.drop, .dismiss):
            return "Couldn't dismiss the offer: \(failure.message)"
        case (.liveStop, .stop), (.drop, .dropStop):
            return "Couldn't stop: \(failure.message)"
        case (.drop, .discard):
            return failure.message
        default:
            return nil
        }
    }

    /// The ids of this model's setting commands (CoreCommand gives each setting command one).
    static let settingIDs: Set<String> = [
        ShellSetting.meetingsHeadsetMic, .retentionDays,
    ].reduce(into: []) { $0.insert("setting:\($1.rawValue)") }

    func load() {
        send(.settingGet(.meetingsHeadsetMic))
        send(.settingGet(.retentionDays))
    }

    /// Records now, the whole of what this Mac plays as the far end.
    func recordNow() {
        failure = nil
        starting = .recordNow
        send(.meetingStart(app: nil, title: titles.titleNow(now())))
    }

    /// Records the call the core offered.
    func record(app: String) {
        failure = nil
        starting = .offer
        send(.meetingStart(app: app, title: titles.titleNow(now())))
    }

    /// "Not this one".
    func dismiss(app: String) {
        failure = nil
        send(.meetingDismiss(app: app))
    }

    func stop(from origin: Origin = .stop) {
        failure = nil
        stopping = origin
        send(.meetingStop)
    }

    /// "Stop and delete": only while the meeting's first minute lasts.
    func discard() {
        guard let record = deletable else { return }
        failure = nil
        discarding = record
        send(.meetingDiscard)
    }

    /// Whether Stop and delete is offered for `record` now.
    func canDiscard(_ record: String?) -> Bool {
        record != nil && deletable == record && discarding == nil
    }

    /// A button on the Drop.
    func perform(_ action: DropText.Action, permissions: PermissionsModel) {
        switch action {
        case .record(let app): record(app: app)
        case .dismiss(let app): dismiss(app: app)
        case .allowSystemAudio: permissions.request(.hearTheOthers)
        case .stop: stop(from: .dropStop)
        case .stopAndDelete: discard()
        // Not a meeting's: ScreenModels.performDropAction opens Today for the one, and sets the
        // app's call policy for the others.
        case .showSpeechModels, .always, .never: break
        }
    }

    /// Words for a Stop and delete the core refused: past the minute, only Stop is left.
    static func discardFailure(_ failed: CommandFailed) -> String {
        failed.code == .deleteWindowOver
            ? "The first minute is over. Stop it, then delete it in the Library."
            : "Couldn't delete it: \(failed.message)"
    }

    /// A meeting started: Stop and delete is offered until its deadline when its app's Always
    /// started it. One scheduled wake ends the offer (architecture rule 9: nothing polls).
    private func started(_ started: MeetingStarted) {
        endDeleteWindow()
        discarding = nil
        // Only a start the policy made: one the user made has Live's Stop, and the library's
        // delete afterwards.
        guard started.auto == true, let until = started.deleteUntilUnixMs else { return }
        let left = Double(until) / 1000 - now().timeIntervalSince1970
        guard left > 0 else { return }
        let record = started.record
        deletable = record
        let sleep = self.sleep
        deleteDeadline = Task { @MainActor [weak self] in
            do { try await sleep(.milliseconds(Int64((left * 1000).rounded(.up)))) } catch { return }
            guard let self, self.deletable == record else { return }
            self.deletable = nil
            self.deleteDeadline = nil
        }
    }

    private func endDeleteWindow() {
        deleteDeadline?.cancel()
        deleteDeadline = nil
        deletable = nil
    }

    /// The meeting `record` ended, one way or another: its Drop buttons' failures go with it.
    private func ended(_ record: String) {
        if deletable == record { endDeleteWindow() }
        if discarding == record { discarding = nil }
        clearMeetingFailure()
    }

    /// A Stop or Stop and delete failure said on the Drop: only while that meeting's Drop shows,
    /// and only until its transcript moves on (it said why the buttons changed).
    private func clearMeetingFailure() {
        if let origin = failure?.origin, origin == .dropStop || origin == .discard {
            failure = nil
        }
    }

    func setHeadsetMic(_ on: Bool) {
        headsetMic = on
        send(.settingSet(.meetingsHeadsetMic, on ? "on" : "off"))
    }

    func setRetention(_ value: Retention) {
        retention = value
        send(.settingSet(.retentionDays, value.rawValue))
    }

    func apply(_ event: InkEvent) {
        switch event {
        case .settingValue(let value):
            switch value.key {
            case ShellSetting.meetingsHeadsetMic.rawValue: headsetMic = value.value == "on"
            case ShellSetting.retentionDays.rawValue:
                retention = value.value.flatMap(Retention.init(rawValue:)) ?? .forever
            default: break
            }
        case .meetingStarted(let meeting):
            failure = nil
            started(meeting)
        case .meetingDetected:
            // A new offer is a new question: a meeting's button failure is not its.
            clearMeetingFailure()
        case .meetingFinal:
            clearMeetingFailure()
        case .meetingStopped(let stopped):
            // Stopped: Stop and delete is no longer offered (a discard in flight goes on).
            if deletable == stopped.record { endDeleteWindow() }
        case .meetingFinished(let finished):
            ended(finished.record)
        case .meetingDiscarded(let discarded):
            ended(discarded.record)
        case .meetingFailed(let failed):
            if let record = failed.record { ended(record) }
        case .meetingWorkerFailed(let failed):
            ended(failed.record)
        case .coreStopped:
            endDeleteWindow()
            discarding = nil
        case .commandFailed(let failed) where failed.command == "meeting.discard":
            // The command names no meeting: a refusal is this shell's only while it waits for one
            // (a meeting that ended meanwhile took its buttons with it).
            guard discarding != nil else {
                log.write("command.failed for a meeting.discard command; its meeting had ended, so nothing shows it")
                break
            }
            discarding = nil
            // Past the minute only Stop is left; another refusal (a store that failed) may pass, so
            // the button stays for its minute to be pressed again.
            if failed.code == .deleteWindowOver { endDeleteWindow() }
            failure = Failure(origin: .discard, message: Self.discardFailure(failed))
            log.write("command.failed for a meeting.discard command; shown where it was asked")
        case .commandFailed(let failed) where ["meeting.start", "meeting.stop", "meeting.dismiss"].contains(failed.command):
            let origin: Origin = switch failed.command {
            case "meeting.stop": stopping
            case "meeting.dismiss": .dismiss
            default: starting
            }
            failure = Failure(origin: origin, message: failed.message)
            // The core logged why; this says the shell showed it (the controller does not log a
            // failure a screen shows). The command's name only, never the core's words.
            log.write("command.failed for a \(failed.command) command; shown where it was asked")
        case .commandFailed(let failed) where Self.settingIDs.contains(failed.id ?? ""):
            // Not known, or not saved: the control shows it, never a guessed value.
            settingsFailed = true
        default:
            break
        }
    }
}
