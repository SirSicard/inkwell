// Today's "Up next": the next meeting on the user's calendars, from EventKit.
//
// Calendar access is a permission (TCC). Inkwell never asks for it on its own: Today shows a
// "Show your next meeting" button, and only a click on it asks. Denied, Today says where to turn it
// back on. What each state looks like is in mac/SCREENS-A-CHECKLIST.md, for a person to check: an
// agent's shell cannot hold the permission.
import AppKit
import EventKit
import Foundation
import Observation

/// Whether Inkwell may read the calendars.
enum CalendarAccess: Equatable, Sendable {
    case notDetermined
    case denied
    case granted
}

/// A calendar event that is coming up.
struct UpcomingEvent: Equatable, Sendable {
    let title: String
    let start: Date
    let end: Date
    /// The meeting app its link or place names (Zoom, Teams, Meet, Webex), if any.
    let app: String?
}

/// Where Today reads calendars from. EventKit in the app; a fake in the tests.
@MainActor
protocol CalendarSource: AnyObject {
    var access: CalendarAccess { get }
    /// The next timed event starting after `now`, within `horizon`.
    func nextEvent(after now: Date, within horizon: TimeInterval) -> UpcomingEvent?
    /// Asks for access (the system prompt). Only on the user's click.
    func requestAccess() async -> CalendarAccess
}

enum MeetingApp {
    /// The meeting app a calendar event's link, place or notes name.
    static func name(url: URL?, location: String?, notes: String?) -> String? {
        let haystack = [url?.absoluteString, location, notes].compactMap { $0?.lowercased() }.joined(separator: " ")
        let apps: [(String, [String])] = [
            ("Zoom", ["zoom.us/", "zoomgov.com/"]),
            ("Teams", ["teams.microsoft.com/", "teams.live.com/"]),
            ("Meet", ["meet.google.com/"]),
            ("Webex", ["webex.com/"]),
            ("FaceTime", ["facetime.apple.com/"]),
            ("Slack", ["slack.com/huddle", "app.slack.com/huddle"]),
        ]
        return apps.first(where: { _, needles in needles.contains(where: haystack.contains) })?.0
    }

    /// "in 42 min", "in 1 h 5 min", "now" for an event starting at `start`.
    static func startsIn(_ start: Date, now: Date) -> String {
        let minutes = Int((start.timeIntervalSince(now) / 60).rounded(.up))
        if minutes <= 0 { return "now" }
        if minutes < 60 { return "in \(minutes) min" }
        let (h, m) = (minutes / 60, minutes % 60)
        return m == 0 ? "in \(h) h" : "in \(h) h \(m) min"
    }
}

/// EventKit, read only.
@MainActor
final class EventKitCalendar: CalendarSource {
    private let store = EKEventStore()

    var access: CalendarAccess {
        switch EKEventStore.authorizationStatus(for: .event) {
        case .fullAccess: .granted
        case .notDetermined: .notDetermined
        // Write-only access cannot read events: for Up next it is the same as none.
        case .denied, .restricted, .writeOnly: .denied
        @unknown default: .denied
        }
    }

    func nextEvent(after now: Date, within horizon: TimeInterval) -> UpcomingEvent? {
        guard access == .granted else { return nil }
        let predicate = store.predicateForEvents(withStart: now, end: now.addingTimeInterval(horizon), calendars: nil)
        return store.events(matching: predicate)
            .filter { !$0.isAllDay && $0.startDate > now && $0.status != .canceled }
            .min { $0.startDate < $1.startDate }
            .map {
                UpcomingEvent(
                    title: $0.title?.isEmpty == false ? $0.title : "Untitled event",
                    start: $0.startDate, end: $0.endDate,
                    app: MeetingApp.name(url: $0.url, location: $0.location, notes: $0.notes))
            }
    }

    func requestAccess() async -> CalendarAccess {
        _ = try? await store.requestFullAccessToEvents()
        return access
    }

    /// Calls `changed` when the calendars change. Holds the observer until the source goes.
    func observe(_ changed: @escaping @MainActor () -> Void) {
        observer = NotificationCenter.default.addObserver(
            forName: .EKEventStoreChanged, object: store, queue: .main
        ) { _ in
            MainActor.assumeIsolated { changed() }
        }
    }

    /// Written once on the main actor, read only by deinit, when nothing else can reach it.
    nonisolated(unsafe) private var observer: NSObjectProtocol?

    deinit {
        if let observer { NotificationCenter.default.removeObserver(observer) }
    }
}

/// What Today's Up next shows.
@MainActor
@Observable
final class UpNextModel {
    private(set) var access: CalendarAccess
    private(set) var event: UpcomingEvent?

    /// How far ahead Up next looks.
    static let horizon: TimeInterval = 12 * 3600

    @ObservationIgnored private let source: CalendarSource
    @ObservationIgnored var now: () -> Date = Date.init
    /// Moves on to the following event when this one starts: one wake at that moment, not a timer
    /// that polls.
    @ObservationIgnored private var rollover: Task<Void, Never>?

    init(source: CalendarSource) {
        self.source = source
        access = source.access
    }

    /// Reads the calendars again.
    func refresh() {
        access = source.access
        event = source.nextEvent(after: now(), within: Self.horizon)
        rollover?.cancel()
        if let start = event?.start {
            let wait = max(start.timeIntervalSince(now()), 0) + 1
            rollover = Task { [weak self] in
                try? await Task.sleep(for: .seconds(wait))
                guard !Task.isCancelled else { return }
                self?.refresh()
            }
        }
    }

    /// Asks for calendar access (the user clicked), then reads.
    func connect() async {
        access = await source.requestAccess()
        refresh()
    }
}
