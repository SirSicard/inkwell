// Today's logic: what the needs-you banner says (from the watchdog, the permission probes and the
// library's record of meetings that kept no far end), and Up next (the calendar's next meeting).
// The calendar here is a fake: EventKit's grant and deny are checked by a person
// (mac/SCREENS-A-CHECKLIST.md).
import Foundation
import InkBridge
import XCTest

@testable import Inkwell

private func event(_ json: String) -> InkEvent {
    (try? InkEvent.decode(Data(json.utf8))) ?? .unknown(type: "")
}

private func permissions(mic: String = "granted", audio: String = "granted", ax: String = "granted") -> PermissionsChecked? {
    guard case .permissionsChecked(let p) = event(#"""
    {"type":"permissions.checked","microphone":"\#(mic)","system_audio":"\#(audio)","accessibility":"\#(ax)","input_monitoring":"unknown"}
    """#) else { return nil }
    return p
}

@MainActor
final class NeedsYouTests: XCTestCase {
    private let calendar: Calendar = {
        var c = Calendar(identifier: .gregorian)
        c.timeZone = TimeZone(identifier: "UTC")!
        c.locale = Locale(identifier: "en_GB")
        return c
    }()

    private func items(
        permissions: PermissionsChecked? = nil, farSilent: Int64 = 0, since: Date? = nil,
        store: CoreStore = CoreStore()
    ) -> [NeedsYouItem] {
        NeedsYou.items(
            permissions: permissions, farSilentMeetings: farSilent, farSilentSince: since,
            meeting: store.meeting, notices: store.notices, now: Date(timeIntervalSince1970: 1_790_000_000),
            calendar: calendar)
    }

    func testNothingNeedsYouWhenAllIsWell() {
        XCTAssertEqual(items(permissions: permissions()), [])
        XCTAssertEqual(items(), [], "before the first check, no guesses")
    }

    func testSystemAudioOffSaysSinceWhenMeetingsKeptOnlyYourVoice() throws {
        let since = Date(timeIntervalSince1970: 1_788_000_000)  // 29 Aug 2026
        let list = items(permissions: permissions(audio: "denied"), farSilent: 4, since: since)
        XCTAssertEqual(list.map(\.id), ["perm-system-audio"], "one item, not the permission and the symptom twice")
        let first = try XCTUnwrap(list.first)
        XCTAssertEqual(first.title, "Inkwell can't hear the other side of your calls")
        XCTAssertEqual(first.detail, "System audio has been off since 29 Aug, so your meetings kept only your own voice.")
        XCTAssertEqual(first.action, .openSettings(.systemAudio))
        XCTAssertEqual(NeedsYouItem.SettingsPane.systemAudio.url?.absoluteString,
                       "x-apple.systempreferences:com.apple.preference.security?Privacy_AudioCapture")
    }

    func testMeetingsThatKeptNoFarEndRaiseItEvenWhenThePermissionReadsGranted() {
        let list = items(permissions: permissions(), farSilent: 2)
        XCTAssertEqual(list.map(\.id), ["far-silent"])
        XCTAssertTrue(list[0].detail.hasPrefix("Your last 2 meetings kept only your own voice"), list[0].detail)
    }

    func testTheWatchdogOnALiveMeetingComesFirst() {
        let store = CoreStore()
        store.apply([
            event(#"{"type":"meeting.started","record":"r1"}"#),
            event(#"{"type":"meeting.side_state","record":"r1","channel":"far","state":"zeros"}"#),
            event(#"{"type":"meeting.side_state","record":"r1","channel":"mic","state":"stopped"}"#),
        ])
        let list = items(permissions: permissions(mic: "denied", ax: "denied"), store: store)
        XCTAssertEqual(list.map(\.id), ["live-far", "live-mic", "perm-mic", "perm-ax"])
    }

    func testOneOffNoticesAreDismissable() throws {
        let store = CoreStore()
        store.apply([
            event(#"{"type":"meeting.warning","record":"r1","kind":"captured_only_zeros","phase":"final"}"#),
            event(#"{"type":"dictation.hotkey_lost"}"#),
            event(#"{"type":"model.warmed","id":"m","job":"dictation_final"}"#),
        ])
        let list = items(store: store)
        XCTAssertEqual(list.map(\.title), ["The dictation key stopped working", "A meeting recorded only silence on one side"],
                       "newest first; only what needs the user")
        guard case .dismiss(let id) = list[0].action else { return XCTFail("not dismissable") }
        store.dismissNotice(id)
        XCTAssertEqual(items(store: store).count, 1)
    }
}

/// A calendar the test sets.
@MainActor
private final class FakeCalendar: CalendarSource {
    var access: CalendarAccess = .notDetermined
    var events: [UpcomingEvent] = []
    var grant = true
    private(set) var asked = 0

    func nextEvent(after now: Date, within horizon: TimeInterval) -> UpcomingEvent? {
        guard access == .granted else { return nil }
        return events.filter { $0.start > now && $0.start < now.addingTimeInterval(horizon) }.min { $0.start < $1.start }
    }

    func requestAccess() async -> CalendarAccess {
        asked += 1
        access = grant ? .granted : .denied
        return access
    }
}

@MainActor
final class UpNextTests: XCTestCase {
    private let now = Date(timeIntervalSince1970: 1_790_000_000)

    func testNothingIsReadOrAskedUntilTheUserConnects() async {
        let calendar = FakeCalendar()
        calendar.events = [UpcomingEvent(title: "Pilot check-in", start: now.addingTimeInterval(2_520), end: now.addingTimeInterval(4_320), app: "Zoom")]
        let model = UpNextModel(source: calendar)
        model.now = { self.now }
        model.refresh()
        XCTAssertEqual(model.access, .notDetermined)
        XCTAssertNil(model.event)
        XCTAssertEqual(calendar.asked, 0, "never asks on its own")

        await model.connect()
        XCTAssertEqual(calendar.asked, 1)
        XCTAssertEqual(model.access, .granted)
        XCTAssertEqual(model.event?.title, "Pilot check-in")
        XCTAssertEqual(MeetingApp.startsIn(model.event!.start, now: now), "in 42 min")
    }

    func testADeniedCalendarShowsNoEvent() async {
        let calendar = FakeCalendar()
        calendar.grant = false
        calendar.events = [UpcomingEvent(title: "x", start: now.addingTimeInterval(60), end: now.addingTimeInterval(120), app: nil)]
        let model = UpNextModel(source: calendar)
        model.now = { self.now }
        await model.connect()
        XCTAssertEqual(model.access, .denied)
        XCTAssertNil(model.event)
    }

    func testTheMeetingAppComesFromTheLinkPlaceOrNotes() {
        XCTAssertEqual(MeetingApp.name(url: URL(string: "https://us02web.zoom.us/j/123"), location: nil, notes: nil), "Zoom")
        XCTAssertEqual(MeetingApp.name(url: nil, location: "Microsoft Teams Meeting", notes: "Join: https://teams.microsoft.com/l/meetup-join/x"), "Teams")
        XCTAssertEqual(MeetingApp.name(url: nil, location: nil, notes: "meet.google.com/abc-defg-hij"), "Meet")
        XCTAssertNil(MeetingApp.name(url: nil, location: "Room 4", notes: nil))
    }

    func testStartsInReadsInMinutesAndHours() {
        XCTAssertEqual(MeetingApp.startsIn(now.addingTimeInterval(30), now: now), "in 1 min")
        XCTAssertEqual(MeetingApp.startsIn(now.addingTimeInterval(3_900), now: now), "in 1 h 5 min")
        XCTAssertEqual(MeetingApp.startsIn(now.addingTimeInterval(7_200), now: now), "in 2 h")
        XCTAssertEqual(MeetingApp.startsIn(now.addingTimeInterval(-10), now: now), "now")
    }
}
