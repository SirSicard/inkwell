// Meetings as the user drives them (S2.8): record now, record the call the Drop offers or say
// not this one, stop, and the settings that shape them: listening for calls, the headset mic, and
// how long the library keeps records.
//
// Recording starts only when the user asks (the Drop's "Record this call", Today's "Record now",
// the menu). Detection only offers. A meeting's title comes from the calendar when a call is on it
// now, read without prompting; otherwise the summary's headline names it later.
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
    /// Whether the core listens for calls (the user's setting; on unless turned off).
    private(set) var detect = true
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
    /// Where the start in flight was asked for (a start's command id is the same from both).
    @ObservationIgnored private var starting: Origin = .recordNow

    init(
        send: @escaping SendCommand, titles: any CallTitles = EventKitCallTitles(),
        now: @escaping () -> Date = Date.init, log: ScreenLog = .system
    ) {
        self.send = send
        self.titles = titles
        self.now = now
        self.log = log
    }

    /// What `place` says about the last failure, or nil when it was not asked there.
    func failure(on place: Place) -> String? {
        guard let failure else { return nil }
        switch (place, failure.origin) {
        case (.recordNow, .recordNow), (.drop, .offer):
            return "Couldn't start recording: \(failure.message)"
        case (.drop, .dismiss):
            return "Couldn't dismiss the offer: \(failure.message)"
        case (.liveStop, .stop):
            return "Couldn't stop: \(failure.message)"
        default:
            return nil
        }
    }

    /// The ids of this model's setting commands (CoreCommand gives each setting command one).
    static let settingIDs: Set<String> = [
        ShellSetting.meetingsDetect, .meetingsHeadsetMic, .retentionDays,
    ].reduce(into: []) { $0.insert("setting:\($1.rawValue)") }

    func load() {
        send(.settingGet(.meetingsDetect))
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

    func stop() {
        failure = nil
        send(.meetingStop)
    }

    /// A button on the Drop.
    func perform(_ action: DropText.Action, permissions: PermissionsModel) {
        switch action {
        case .record(let app): record(app: app)
        case .dismiss(let app): dismiss(app: app)
        case .allowSystemAudio: permissions.request(.hearTheOthers)
        }
    }

    func setDetect(_ on: Bool) {
        detect = on
        send(.settingSet(.meetingsDetect, on ? "on" : "off"))
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
            case ShellSetting.meetingsDetect.rawValue: detect = value.value != "off"
            case ShellSetting.meetingsHeadsetMic.rawValue: headsetMic = value.value == "on"
            case ShellSetting.retentionDays.rawValue:
                retention = value.value.flatMap(Retention.init(rawValue:)) ?? .forever
            default: break
            }
        case .meetingStarted:
            failure = nil
        case .commandFailed(let failed) where ["meeting.start", "meeting.stop", "meeting.dismiss"].contains(failed.command):
            let origin: Origin = switch failed.command {
            case "meeting.stop": .stop
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
