// Today's "Up next": the next meeting on the user's calendars, from EventKit.
//
// Calendar access is a permission (TCC), the Settings card "Know your meetings"
// (PermissionsModel's CalendarAccess: its state and its request). Inkwell never asks on its own:
// Today shows a "Show my next meeting" button, and only a click on it asks. Denied, Today says
// where to turn it back on. What each state looks like is in mac/SCREENS-A-CHECKLIST.md, for a
// person to check: an agent's shell cannot hold the permission.
//
// "in 42 min" is the one thing on Today that changes with the clock. It redraws once a minute,
// and only while an event is shown and the window is on screen (WindowPresence): hidden,
// occluded or minimised, nothing ticks (architecture rule 9).
import AppKit
import EventKit
import Foundation
import Observation
import SwiftUI

/// A calendar event that is coming up.
struct UpcomingEvent: Equatable, Sendable {
    let title: String
    let start: Date
    let end: Date
    /// The meeting app its link, place or notes name (Zoom, Teams, Meet, Webex), if any.
    let app: String?
}

/// Where Today reads the next event. EventKit in the app; a fake in the tests.
@MainActor
protocol UpcomingEvents: AnyObject {
    /// The next timed event starting after `now`, within `horizon`. Nil without access.
    func nextEvent(after now: Date, within horizon: TimeInterval) -> UpcomingEvent?
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

/// EventKit's events, read only.
@MainActor
final class EventKitEvents: UpcomingEvents {
    private let store = EKEventStore()

    func nextEvent(after now: Date, within horizon: TimeInterval) -> UpcomingEvent? {
        guard EKEventStore.authorizationStatus(for: .event) == .fullAccess else { return nil }
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
    private(set) var access: CardState
    private(set) var event: UpcomingEvent?

    /// How far ahead Up next looks.
    static let horizon: TimeInterval = 12 * 3600

    @ObservationIgnored private let calendarAccess: any CalendarAccess
    @ObservationIgnored private let events: any UpcomingEvents
    @ObservationIgnored var now: () -> Date = Date.init
    /// Moves on to the following event when this one starts: one wake at that moment, not a timer
    /// that polls.
    @ObservationIgnored private var rollover: Task<Void, Never>?

    init(access: any CalendarAccess, events: any UpcomingEvents) {
        calendarAccess = access
        self.events = events
        self.access = access.state()
    }

    /// Reads the calendars again.
    func refresh() {
        access = calendarAccess.state()
        event = access == .allowed ? events.nextEvent(after: now(), within: Self.horizon) : nil
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

    /// Asks for calendar access (the user clicked), then reads: the prompt the first time, else
    /// System Settings.
    func connect() {
        calendarAccess.request { [weak self] in self?.refresh() }
    }

    /// Whether "in N min" redraws on the minute now: an event is shown and the window is on
    /// screen.
    func ticks(onScreen: Bool) -> Bool {
        onScreen && event != nil
    }
}

/// A schedule of minute boundaries while running, and nothing after its first entry while paused:
/// a paused view draws once and then stays still.
struct MinuteSchedule: TimelineSchedule {
    let paused: Bool

    func entries(from startDate: Date, mode: TimelineScheduleMode) -> AnyIterator<Date> {
        var next: Date? = startDate
        let paused = paused
        return AnyIterator {
            guard let date = next else { return nil }
            if paused {
                next = nil
            } else {
                // The next whole minute.
                let seconds = (date.timeIntervalSinceReferenceDate / 60).rounded(.down) * 60 + 60
                next = Date(timeIntervalSinceReferenceDate: seconds)
            }
            return date
        }
    }
}

/// Whether the main window is on screen: open, not minimised, and not fully covered. What redraws
/// on a clock reads it and stops while it is false.
@MainActor
@Observable
final class WindowPresence {
    private(set) var onScreen = false

    /// On screen: visible, not minimised, and some of it not covered.
    nonisolated static func onScreen(visible: Bool, miniaturized: Bool, occlusionVisible: Bool) -> Bool {
        visible && !miniaturized && occlusionVisible
    }

    /// Reads `window` now (the controller calls it on every change of these).
    func update(_ window: NSWindow?) {
        let now = window.map {
            Self.onScreen(
                visible: $0.isVisible, miniaturized: $0.isMiniaturized,
                occlusionVisible: $0.occlusionState.contains(.visible))
        } ?? false
        if now != onScreen { onScreen = now }
    }
}
