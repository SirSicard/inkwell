// Today's logic: what the needs-you banner says (from the watchdog, the permission cards and the
// library's record of meetings that kept no far end), Up next (the calendar's next meeting), and
// that its minute redraw runs only while the window is on screen. The calendar here is a fake:
// EventKit's grant and deny are checked by a person (mac/SCREENS-A-CHECKLIST.md).
import Foundation
import InkBridge
import SwiftUI
import XCTest

@testable import Inkwell

private func event(_ json: String) -> InkEvent {
    (try? InkEvent.decode(Data(json.utf8))) ?? .unknown(type: "")
}

/// The cards as a permissions check leaves them: `off` names the ones refused, the rest allowed.
private func cards(off: Set<PermissionCard> = []) -> (PermissionCard) -> CardState {
    { off.contains($0) ? .off : .allowed }
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
        permission: @escaping (PermissionCard) -> CardState = cards(), farSilent: Int64 = 0, since: Date? = nil,
        store: CoreStore = CoreStore()
    ) -> [NeedsYouItem] {
        NeedsYou.items(
            permission: permission, farEnd: .checked(meetings: farSilent, since: since),
            meeting: store.meeting, notices: store.notices, now: Date(timeIntervalSince1970: 1_790_000_000),
            calendar: calendar)
    }

    /// Review fix: the counts that tell whether the far end recorded nothing could not be read.
    /// That is said ("couldn't check"), never read as zero meetings, which would silence the one
    /// warning this banner exists for. Tested through Today's own wiring (TodayScreen.needItems)
    /// from the library model's answers.
    func testTodaysBannerSaysWhenItCouldNotCheckTheFarEnd() throws {
        var sent: [String] = []
        let library = LibraryModel(send: { sent.append($0.json) })
        let store = CoreStore()
        let now = Date(timeIntervalSince1970: 1_790_000_000)
        let today = { TodayScreen.needItems(library: library, store: store, permission: cards(), now: now) }
        library.refreshToday()
        let stats = sent.compactMap { json -> String? in
            let c = (try? JSONSerialization.jsonObject(with: Data(json.utf8))) as? [String: Any]
            return c?["cmd"] as? String == "library.stats" ? c?["id"] as? String : nil
        }
        XCTAssertEqual(stats.count, 2)
        XCTAssertEqual(today(), [], "still asking: no guess either way")

        for id in stats {
            library.apply([event(#"{"type":"command.failed","command":"library.stats","id":"\#(id)","message":"the library: disk I/O error"}"#)])
        }
        let failed = today()
        XCTAssertEqual(failed.map(\.id), ["far-unknown"])
        XCTAssertEqual(failed[0].title, "Inkwell couldn't check the other side of your calls")
        XCTAssertEqual(failed[0].action, .retryChecks)
        guard case .commandFailed(let failure) = event(#"{"type":"command.failed","command":"library.stats","id":"\#(stats[0])","message":"x"}"#) else {
            return XCTFail("not a failure")
        }
        XCTAssertTrue(library.handles(failure), "shown on Today, so the model claims it")

        // Asked again, and answered: two meetings kept no far end.
        sent.removeAll()
        library.refreshToday()
        let again = sent.compactMap { json -> String? in
            let c = (try? JSONSerialization.jsonObject(with: Data(json.utf8))) as? [String: Any]
            return c?["cmd"] as? String == "library.stats" ? c?["id"] as? String : nil
        }
        library.apply([event(#"{"type":"library.stats","ref":"\#(again[1])","since_unix_ms":0,"kinds":[],"far_silent_meetings":2,"far_silent_since_unix_ms":1789000000000}"#)])
        XCTAssertEqual(today().map(\.id), ["far-silent"])
    }

    func testNothingNeedsYouWhenAllIsWell() {
        XCTAssertEqual(items(), [])
        XCTAssertEqual(items(permission: { _ in .checking }), [], "before the first check, no guesses")
    }

    func testSystemAudioOffSaysSinceWhenMeetingsKeptOnlyYourVoice() throws {
        let since = Date(timeIntervalSince1970: 1_788_000_000)  // 29 Aug 2026
        let list = items(permission: cards(off: [.hearTheOthers]), farSilent: 4, since: since)
        XCTAssertEqual(list.map(\.id), ["perm-system-audio"], "one item, not the permission and the symptom twice")
        let first = try XCTUnwrap(list.first)
        XCTAssertEqual(first.title, "Inkwell can't hear the other side of your calls")
        XCTAssertEqual(first.detail, "System audio has been off since 29 Aug, so your meetings kept only your own voice.")
        XCTAssertEqual(first.action, .allow(.hearTheOthers), "the Settings card's request: a prompt, or its pane")
    }

    func testMeetingsThatKeptNoFarEndRaiseItEvenWhenThePermissionReadsAllowed() {
        let list = items(farSilent: 2)
        XCTAssertEqual(list.map(\.id), ["far-silent"])
        XCTAssertTrue(list[0].detail.hasPrefix("Your last 2 meetings kept only your own voice"), list[0].detail)
        XCTAssertEqual(items(farSilent: 1)[0].detail, "Your last meeting kept only your own voice. Check that system audio is allowed.")
    }

    func testTheWatchdogOnALiveMeetingComesFirst() {
        let store = CoreStore()
        store.apply([
            event(#"{"type":"meeting.started","record":"r1"}"#),
            event(#"{"type":"meeting.side_state","record":"r1","channel":"far","state":"zeros"}"#),
            event(#"{"type":"meeting.side_state","record":"r1","channel":"mic","state":"stopped"}"#),
        ])
        let list = items(permission: cards(off: [.hearYou, .typeForYou]), store: store)
        XCTAssertEqual(list.map(\.id), ["live-far", "live-mic", "perm-mic", "perm-ax"])
        XCTAssertEqual(list[2].action, .allow(.hearYou))
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

    func testNoAudioWarningNamesOnlyTheSideThatCapturedNothing() throws {
        for (channel, title, detail) in [
            ("far", "The other side of the last meeting recorded nothing", "No audio reached Inkwell from the other side. Your microphone's recording is separate."),
            ("mic", "Your side of the last meeting recorded nothing", "No audio reached Inkwell from your microphone. The other side's recording is separate."),
        ] {
            let store = CoreStore()
            store.apply([event(#"{"type":"meeting.warning","record":"r1","kind":"nothing_captured","channel":"\#(channel)","phase":"final"}"#)])
            let item = try XCTUnwrap(items(store: store).first)
            XCTAssertEqual(item.title, title)
            XCTAssertEqual(item.detail, detail)
        }
        let store = CoreStore()
        store.apply([event(#"{"type":"meeting.warning","record":"r1","kind":"nothing_captured","phase":"final"}"#)])
        let item = try XCTUnwrap(items(store: store).first)
        XCTAssertEqual(item.title, "One side of the last meeting recorded nothing")
        XCTAssertEqual(item.detail, "No audio reached Inkwell from one side. The warning did not identify which side.")
    }
}

/// The calendar's permission, as the test sets it (PermissionsModel's CalendarAccess).
private final class FakeAccess: CalendarAccess, @unchecked Sendable {
    // @unchecked: touched only on the main actor, by the test and the model it drives.
    var current: CardState = .notAsked
    var grant = true
    private(set) var asked = 0

    func state() -> CardState { current }

    func request(done: @escaping @MainActor @Sendable () -> Void) {
        asked += 1
        current = grant ? .allowed : .off
        Task { @MainActor in done() }
    }
}

/// The calendar's events, as the test sets them.
@MainActor
private final class FakeEvents: UpcomingEvents {
    var events: [UpcomingEvent] = []
    var access: FakeAccess

    init(access: FakeAccess) {
        self.access = access
    }

    func nextEvent(after now: Date, within horizon: TimeInterval) -> UpcomingEvent? {
        guard access.current == .allowed else { return nil }
        return events.filter { $0.start > now && $0.start < now.addingTimeInterval(horizon) }.min { $0.start < $1.start }
    }
}

@MainActor
final class UpNextTests: XCTestCase {
    private let now = Date(timeIntervalSince1970: 1_790_000_000)

    private func model(grant: Bool = true) -> (UpNextModel, FakeAccess, FakeEvents) {
        let access = FakeAccess()
        access.grant = grant
        let events = FakeEvents(access: access)
        events.events = [UpcomingEvent(title: "Pilot check-in", start: now.addingTimeInterval(2_520), end: now.addingTimeInterval(4_320), app: "Zoom")]
        let model = UpNextModel(access: access, events: events)
        model.now = { self.now }
        return (model, access, events)
    }

    /// Waits for the request's answer, which comes back on the main actor.
    private func settle() async {
        for _ in 0..<5 { await Task.yield() }
    }

    func testNothingIsReadOrAskedUntilTheUserConnects() async {
        let (model, access, _) = model()
        model.refresh()
        XCTAssertEqual(model.access, .notAsked)
        XCTAssertNil(model.event)
        XCTAssertEqual(access.asked, 0, "never asks on its own")

        model.connect()
        await settle()
        XCTAssertEqual(access.asked, 1)
        XCTAssertEqual(model.access, .allowed)
        XCTAssertEqual(model.event?.title, "Pilot check-in")
        XCTAssertEqual(MeetingApp.startsIn(model.event!.start, now: now), "in 42 min")
    }

    func testADeniedCalendarShowsNoEvent() async {
        let (model, _, _) = model(grant: false)
        model.connect()
        await settle()
        XCTAssertEqual(model.access, .off)
        XCTAssertNil(model.event)
    }

    /// The architecture rule: nothing ticks while idle. "in N min" redraws on the minute only
    /// while an event is shown and the window is on screen; hidden, covered or minimised, its
    /// schedule gives one entry (the frame drawn when it stopped) and then none.
    func testTheMinuteTicksOnlyWhileAnEventIsShownOnScreen() async {
        let (model, _, _) = model()
        model.connect()
        await settle()
        XCTAssertTrue(model.ticks(onScreen: true))
        XCTAssertFalse(model.ticks(onScreen: false), "hidden, occluded or minimised")

        let start = Date(timeIntervalSinceReferenceDate: 1_000_030)  // 30 s past a minute
        let running = Array(MinuteSchedule(paused: false).entries(from: start, mode: .normal).prefix(3))
        XCTAssertEqual(running.map(\.timeIntervalSinceReferenceDate), [1_000_030, 1_000_080, 1_000_140],
                       "now, then each whole minute")
        let paused = Array(MinuteSchedule(paused: true).entries(from: start, mode: .normal).prefix(3))
        XCTAssertEqual(paused, [start], "one frame, then nothing")

        let (empty, _, events) = self.model()
        events.events = []
        empty.connect()
        await settle()
        XCTAssertFalse(empty.ticks(onScreen: true), "no event, nothing to count down")
    }

    func testTheWindowIsOnScreenOnlyVisibleUncoveredAndNotMinimised() {
        XCTAssertTrue(WindowPresence.onScreen(visible: true, miniaturized: false, occlusionVisible: true))
        XCTAssertFalse(WindowPresence.onScreen(visible: true, miniaturized: false, occlusionVisible: false), "covered, another Space, screen locked")
        XCTAssertFalse(WindowPresence.onScreen(visible: true, miniaturized: true, occlusionVisible: true), "minimised")
        XCTAssertFalse(WindowPresence.onScreen(visible: false, miniaturized: false, occlusionVisible: true), "closed")
        let presence = WindowPresence()
        presence.update(nil)
        XCTAssertFalse(presence.onScreen, "no window")
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

/// Review fix: Today said "Hold Control-Shift-Space to dictate" while Settings' key cap showed
/// ⌃⇧Space. Both now come from one formatter (DictationModel.cap).
@MainActor
final class TodayStatusLineTests: XCTestCase {
    func testTodayNamesTheDictationKeyAsSettingsShowsIt() {
        let dictation = DictationModel(send: { _ in })
        dictation.apply(event(#"{"type":"dictation.ready","key":"ctrl+shift+space"}"#))
        XCTAssertEqual(dictation.status, "Hold \u{2303}\u{21E7}Space, speak, let go.")
        let line = TodayScreen.statusLine(listening: "Listening for calls", key: dictation.key, offersDictation: true)
        XCTAssertEqual(line, "Listening for calls · Hold \u{2303}\u{21E7}Space to dictate")
        XCTAssertEqual(DictationModel.cap(dictation.key), "\u{2303}\u{21E7}Space", "Settings' key cap")
        XCTAssertFalse(line.contains("Control"), line)
        XCTAssertEqual(
            TodayScreen.statusLine(listening: "Listening for calls", key: dictation.key, offersDictation: true, spoken: true),
            "Listening for calls · Hold Control-Shift-Space to dictate", "VoiceOver hears the name")
        XCTAssertEqual(TodayScreen.statusLine(listening: "", key: "fn", offersDictation: true), "Hold fn to dictate")
        XCTAssertEqual(TodayScreen.statusLine(listening: "Listening for calls", key: "fn", offersDictation: false), "Listening for calls")
    }
}
