// The Stats screen's model and words: what it asks the core (stats.get on the user's calendar,
// milestones.check when a dictation or a meeting ends), what it keeps of the answers, and how the
// numbers read. The counting is the core's (ink-ffi's stats tests); nothing here counts.
import AppKit
import Foundation
import InkBridge
import SwiftUI
import XCTest

@testable import Inkwell

private func event(_ json: String, file: StaticString = #filePath, line: UInt = #line) -> InkEvent {
    guard let decoded = try? InkEvent.decode(Data(json.utf8)) else {
        XCTFail("not an event: \(json)", file: file, line: line)
        return .unknown(type: "")
    }
    if case .undecodable = decoded {
        XCTFail("not this build's event: \(json)", file: file, line: line)
    }
    return decoded
}

private struct NoCalendar: CalendarAccess {
    func state() -> CardState { .notAsked }
    func request(done: @escaping @MainActor @Sendable () -> Void) {}
}

/// The fields of each command sent, as the core reads them.
private func fields(_ commands: [CoreCommand]) -> [[String: Any]] {
    commands.compactMap { (try? JSONSerialization.jsonObject(with: Data($0.json.utf8))) as? [String: Any] }
}

/// A `stats.counted` answer, with the dictation part given and the rest empty unless given.
func statsCounted(
    ref: String, words: (today: Int, week: Int, all: Int) = (0, 0, 0), dictations: Int = 0,
    wpmWeek: Int? = nil, wpmAverage: Int? = nil, savedWeek: Int64 = 0, savedAll: Int64 = 0,
    streak: Int = 0, longest: Int = 0, heatmap: [Int] = Array(repeating: 0, count: 83),
    meetings: String = #"{"meetings":0,"recorded_ms":0,"you_ms":0,"them_ms":0,"longest_monologue_ms":0,"questions":0}"#,
    promises: String = #"{"made":0,"kept":0,"open":0,"overdue":0}"#,
    reached: Set<String> = [], typingWpm: Int = 40
) -> String {
    var dictation = #""words_today":\#(words.today),"words_week":\#(words.week),"words_all":\#(words.all),"dictations_all":\#(dictations),"saved_ms_week":\#(savedWeek),"saved_ms_all":\#(savedAll),"streak_days":\#(streak),"longest_streak_days":\#(longest),"heatmap_first_day":"2026-07-13","heatmap_words":[\#(heatmap.map(String.init).joined(separator: ","))]"#
    if let wpmWeek { dictation += #","wpm_week":\#(wpmWeek)"# }
    if let wpmAverage { dictation += #","wpm_average":\#(wpmAverage)"# }
    let milestones = [("words_1000", "words", 1000), ("words_10000", "words", 10000), ("words_50000", "words", 50000),
                      ("words_100000", "words", 100000), ("streak_7", "streak", 7), ("streak_30", "streak", 30),
                      ("streak_100", "streak", 100)]
        .map { #"{"id":"\#($0.0)","kind":"\#($0.1)","threshold":\#($0.2),"reached":\#(reached.contains($0.0))}"# }
        .joined(separator: ",")
    return #"{"type":"stats.counted","ref":"\#(ref)","today":"2026-10-03","typing_wpm":\#(typingWpm),"dictation":{\#(dictation)},"meetings_month":\#(meetings),"meetings_all":\#(meetings),"promises_month":\#(promises),"promises_all":\#(promises),"milestones":[\#(milestones)]}"#
}

@MainActor
final class StatsModelTests: XCTestCase {
    private var sent: [CoreCommand] = []

    private func model() -> StatsModel {
        let stats = StatsModel(send: { [unowned self] in self.sent.append($0) })
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(identifier: "UTC")!
        calendar.locale = Locale(identifier: "en_GB")
        stats.calendar = calendar
        stats.now = { Date(timeIntervalSince1970: 1_791_028_800) }
        return stats
    }

    private func lastRef(_ cmd: String) -> String? {
        fields(sent).last { $0["cmd"] as? String == cmd }?["id"] as? String
    }

    func testLoadAsksOnTheUsersCalendarAndKeepsOnlyTheNewestAnswer() throws {
        let stats = model()
        stats.load()
        let first = try XCTUnwrap(lastRef("stats.get"))
        let asked = try XCTUnwrap(fields(sent).last)
        XCTAssertEqual(asked["week_start"] as? Int, 1, "en_GB weeks start on Monday")
        let offsets = try XCTUnwrap(asked["utc_offsets"] as? [[String: Any]])
        XCTAssertEqual(offsets.count, 1, "UTC never changes")
        XCTAssertEqual(offsets[0]["minutes"] as? Int, 0)
        XCTAssertEqual(stats.loadState, .loading)

        stats.load()
        let second = try XCTUnwrap(lastRef("stats.get"))
        XCTAssertNotEqual(first, second)
        stats.apply(event(statsCounted(ref: first, words: (1, 1, 1))))
        XCTAssertNil(stats.counted, "a stale answer is not shown")
        stats.apply(event(statsCounted(ref: second, words: (5, 50, 500))))
        XCTAssertEqual(stats.counted?.dictation.wordsAll, 500)
        XCTAssertEqual(stats.loadState, .loaded)
    }

    func testAFailureIsSaidNeverShownAsZero() throws {
        let stats = model()
        stats.load()
        let ref = try XCTUnwrap(lastRef("stats.get"))
        guard case .commandFailed(let failed) = event(#"{"type":"command.failed","command":"stats.get","id":"\#(ref)","message":"the library: disk I/O error"}"#) else {
            return XCTFail("not a failure")
        }
        XCTAssertTrue(stats.handles(failed))
        stats.apply(.commandFailed(failed))
        XCTAssertEqual(stats.loadState, .failed)
        XCTAssertNil(stats.counted)
    }

    /// A zone with daylight saving sends each change, oldest first, so an old record keeps the
    /// offset it was made under; weeks start where the user's calendar starts them.
    func testTheCalendarFieldsCarryEveryOffsetChange() {
        let now = Date(timeIntervalSince1970: 1_791_028_800)
        let madrid = StatsModel.utcOffsets(timeZone: TimeZone(identifier: "Europe/Madrid")!, now: now)
        XCTAssertGreaterThan(madrid.count, 15, "two changes a year for ten years")
        XCTAssertEqual(madrid.map(\.fromUnixMs), madrid.map(\.fromUnixMs).sorted(), "oldest first")
        XCTAssertEqual(Set(madrid.map(\.minutes)), [60, 120])
        XCTAssertEqual(madrid.last?.minutes, 120, "summer time on 3 October")
        XCTAssertLessThan(madrid.count, 400)
        XCTAssertEqual(StatsModel.utcOffsets(timeZone: TimeZone(identifier: "Asia/Tokyo")!, now: now).map(\.minutes), [540])

        var us = Calendar(identifier: .gregorian)
        us.locale = Locale(identifier: "en_US")
        XCTAssertEqual(StatsModel.isoWeekStart(us), 7, "Sunday")
        var gb = Calendar(identifier: .gregorian)
        gb.locale = Locale(identifier: "en_GB")
        XCTAssertEqual(StatsModel.isoWeekStart(gb), 1, "Monday")
        var saturday = Calendar(identifier: .gregorian)
        saturday.firstWeekday = 7
        XCTAssertEqual(StatsModel.isoWeekStart(saturday), 6)
    }

    /// Milestones are checked at launch (the core notes what is already reached) and whenever a
    /// dictation is saved or a meeting ends; a dictation that was not saved changes nothing.
    func testMilestonesAreCheckedAtLaunchAndWhenSomethingIsSaved() {
        let stats = model()
        stats.apply(event(#"{"type":"core.ready","version":"1.0.0","abi":2}"#))
        let atLaunch = fields(sent).compactMap { $0["cmd"] as? String }
        XCTAssertTrue(atLaunch.contains("milestones.check"), "\(atLaunch)")
        XCTAssertEqual(
            Set(fields(sent).filter { $0["cmd"] as? String == "setting.get" }.compactMap { $0["key"] as? String }),
            ["stats.celebrate", "stats.typing_wpm"])

        sent.removeAll()
        stats.apply(event(#"{"type":"dictation.inserted","outcome":"pasted","text":"x"}"#))
        XCTAssertTrue(sent.isEmpty, "not saved: nothing new to count")
        stats.apply(event(#"{"type":"dictation.inserted","outcome":"pasted","record":"r1","text":"x"}"#))
        XCTAssertEqual(fields(sent).compactMap { $0["cmd"] as? String }, ["milestones.check"])
        sent.removeAll()
        stats.apply(event(#"{"type":"meeting.finished","record":"m1"}"#))
        XCTAssertEqual(fields(sent).compactMap { $0["cmd"] as? String }, ["milestones.check"])
    }

    /// While the screen shows, a new dictation counts at once.
    func testAShownScreenCountsAgainWhenSomethingIsSaved() {
        let stats = model()
        stats.screenAppeared()
        XCTAssertEqual(fields(sent).compactMap { $0["cmd"] as? String }, ["stats.get"])
        sent.removeAll()
        stats.apply(event(#"{"type":"dictation.inserted","outcome":"pasted","record":"r1","text":"x"}"#))
        XCTAssertEqual(Set(fields(sent).compactMap { $0["cmd"] as? String }), ["milestones.check", "stats.get"])
        stats.screenDisappeared()
        sent.removeAll()
        stats.apply(event(#"{"type":"dictation.inserted","outcome":"pasted","record":"r2","text":"x"}"#))
        XCTAssertEqual(fields(sent).compactMap { $0["cmd"] as? String }, ["milestones.check"])
    }

    func testAMilestoneReachedIsCelebratedWithOneLine() throws {
        let stats = model()
        stats.checkMilestones()
        let ref = try XCTUnwrap(lastRef("milestones.check"))
        stats.apply(event(#"{"type":"milestones.reached","ref":"\#(ref)","milestones":[]}"#))
        XCTAssertNil(stats.celebration, "nothing new")
        stats.apply(event(#"{"type":"milestones.reached","ref":"\#(ref)","milestones":[{"id":"words_1000","kind":"words","threshold":1000,"reached":true},{"id":"streak_7","kind":"streak","threshold":7,"reached":true}]}"#))
        let shown = try XCTUnwrap(stats.celebration)
        XCTAssertEqual(shown.note, "Milestone · a 7-day streak", "the last one, said once")
        stats.dismissCelebration(shown.serial + 1)
        XCTAssertNotNil(stats.celebration, "a later serial is not this one")
        stats.dismissCelebration(shown.serial)
        XCTAssertNil(stats.celebration)
        XCTAssertEqual(StatsFormat.milestoneNote(kind: .words, threshold: 100_000, locale: Locale(identifier: "en_GB")),
                       "Milestone · 100,000 words dictated")
    }

    func testTheSettingsAreReadAndSetAndTheSpeedRecountsTimeSaved() throws {
        let stats = model()
        XCTAssertTrue(stats.celebrate)
        XCTAssertEqual(stats.typingWpm, 40)
        stats.apply(event(#"{"type":"setting.value","key":"stats.celebrate","value":"off"}"#))
        stats.apply(event(#"{"type":"setting.value","key":"stats.typing_wpm","value":"55"}"#))
        XCTAssertFalse(stats.celebrate)
        XCTAssertEqual(stats.typingWpm, 55)
        stats.apply(event(#"{"type":"setting.value","key":"stats.typing_wpm","value":"nonsense"}"#))
        XCTAssertEqual(stats.typingWpm, 40, "unreadable reads as the default, as the core reads it")

        stats.setCelebrate(true)
        XCTAssertTrue(stats.celebrate)
        stats.setTypingWpm(500)
        stats.setTypingWpm(3)
        let set = fields(sent).filter { $0["cmd"] as? String == "setting.set" }
        XCTAssertEqual(set.compactMap { $0["value"] as? String }, ["on", "200", "10"], "clamped to what the core takes")

        // A setting that could not be saved is said in Settings.
        XCTAssertFalse(stats.settingsFailed)
        guard case .commandFailed(let failed) = event(#"{"type":"command.failed","command":"setting.set","id":"setting:stats.celebrate","message":"disk I/O error"}"#) else {
            return XCTFail("not a failure")
        }
        XCTAssertTrue(stats.handles(failed))
        stats.apply(.commandFailed(failed))
        XCTAssertTrue(stats.settingsFailed)

        // With the screen loaded, a new speed recounts.
        stats.load()
        let ref = try XCTUnwrap(lastRef("stats.get"))
        stats.apply(event(statsCounted(ref: ref)))
        sent.removeAll()
        stats.apply(event(#"{"type":"setting.value","key":"stats.typing_wpm","value":"60"}"#))
        XCTAssertEqual(fields(sent).compactMap { $0["cmd"] as? String }, ["stats.get"])
    }
}

@MainActor
final class StatsFormatTests: XCTestCase {
    private let gb = Locale(identifier: "en_GB")

    func testTimeSavedAlwaysNamesItsAssumption() {
        XCTAssertEqual(StatsFormat.saved(ms: 12_000_000, typingWpm: 40), "3 h 20 min saved vs typing at 40 wpm")
        XCTAssertEqual(StatsFormat.saved(ms: 20_000, typingWpm: 55), "under 1 min saved vs typing at 55 wpm")
        XCTAssertEqual(StatsFormat.saved(ms: 0, typingWpm: 40), "No time saved yet vs typing at 40 wpm")
        XCTAssertEqual(StatsFormat.saved(ms: -90_000, typingWpm: 40), "No time saved yet vs typing at 40 wpm")
    }

    func testSpeedIsAgainstYourOwnAverage() {
        XCTAssertEqual(StatsFormat.speed(week: 142, average: 135), "142 wpm this week · your 30-day average 135 wpm")
        XCTAssertEqual(StatsFormat.speed(week: 142, average: nil), "142 wpm this week")
        XCTAssertEqual(StatsFormat.speed(week: nil, average: 135), "No speed this week yet · your 30-day average 135 wpm")
        XCTAssertEqual(StatsFormat.speed(week: nil, average: nil), "Your speed shows after a minute of dictation")
    }

    /// The streak shows from its second day; before that, the longest one, if there was one.
    func testTheStreakShowsFromDayTwo() {
        XCTAssertNil(StatsFormat.streak(current: 0, longest: 0))
        XCTAssertNil(StatsFormat.streak(current: 1, longest: 1))
        XCTAssertEqual(StatsFormat.streak(current: 1, longest: 9), "Longest streak 9 days")
        XCTAssertEqual(StatsFormat.streak(current: 2, longest: 2), "2-day streak")
        XCTAssertEqual(StatsFormat.streak(current: 5, longest: 12), "5-day streak · longest 12 days")
    }

    func testShortSpansReadInSecondsAndMinutes() {
        XCTAssertEqual(StatsFormat.span(ms: 0), "0 s")
        XCTAssertEqual(StatsFormat.span(ms: 45_400), "45 s")
        XCTAssertEqual(StatsFormat.span(ms: 120_000), "2 min")
        XCTAssertEqual(StatsFormat.span(ms: 250_000), "4 min 10 s")
        XCTAssertEqual(StatsFormat.span(ms: 3_900_000), "1 h 5 min")
    }

    func testTalkTimeSharesAddUpAndPromisesReadAsKeptOfMade() throws {
        let share = try XCTUnwrap(StatsFormat.talkShare(you: 1, them: 2))
        XCTAssertEqual(share.you + share.them, 100)
        XCTAssertEqual(share.you, 33)
        XCTAssertNil(StatsFormat.talkShare(you: 0, them: 0))
        let promises = try XCTUnwrap(try? JSONDecoder().decode(PromiseStats.self, from: Data(#"{"made":14,"kept":12,"open":1,"overdue":1}"#.utf8)))
        XCTAssertEqual(StatsFormat.kept(promises, period: "this month"), "Kept 12 of 14 this month")
        XCTAssertEqual(StatsFormat.promiseDetail(promises), "1 open · 1 overdue")
    }

    /// The heatmap is shaded against the busiest day, and VoiceOver hears a summary, not 83 cells.
    func testTheHeatmapIsShadedAndSummarised() throws {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(identifier: "UTC")!
        calendar.locale = gb
        var words = Array(repeating: 0, count: 83)
        words[0] = 10
        words[30] = 1_204
        words[82] = 600
        let stats = try JSONDecoder().decode(StatsCounted.self, from: Data(statsCounted(ref: "x", heatmap: words).utf8))
        let cells = StatsFormat.heatmap(stats.dictation, calendar: calendar)
        XCTAssertEqual(cells.count, 83)
        XCTAssertEqual(cells[0].level, 1)
        XCTAssertEqual(cells[30].level, 4)
        XCTAssertEqual(cells[82].level, 2)
        XCTAssertEqual(cells[1].level, 0)
        XCTAssertEqual(cells[0].weekday, 0, "13 July 2026 starts the first column")
        XCTAssertEqual(
            StatsFormat.heatmapSummary(cells, calendar: calendar, locale: gb),
            "Words dictated per day over the last 12 weeks: 3 of 83 days with dictation. The most was 1,204 words, on Wed 12 Aug.")
        let empty = StatsFormat.heatmap(try JSONDecoder().decode(StatsCounted.self, from: Data(statsCounted(ref: "x").utf8)).dictation, calendar: calendar)
        XCTAssertEqual(StatsFormat.heatmapSummary(empty, calendar: calendar, locale: gb),
                       "Words dictated per day over the last 12 weeks: no dictation yet.")
    }
}

/// The Stats screen never raises the window's minimum size (460 high, 720 wide), full or empty,
/// and its cards fit a narrow window: numbers go one under another rather than clip.
@MainActor
final class StatsLayoutTests: XCTestCase {
    private func screens(answer: String?) -> ScreenModels {
        var sent: [CoreCommand] = []
        let screens = ScreenModels(send: { sent.append($0) }, calendar: NoCalendar(), apps: WorkspaceApps())
        screens.stats.load()
        if let answer, let ref = sent.last?.commandID {
            screens.stats.apply(event(answer.replacingOccurrences(of: "REF", with: ref)))
        }
        return screens
    }

    private func minimum(_ screens: ScreenModels) -> CGSize {
        let hosting = NSHostingController(
            rootView: StatsScreen().environment(screens).environment(CoreStore()).environment(screens.theme)
                .environment(LibraryModel(send: { _ in })).environment(Router()))
        return hosting.sizeThatFits(in: .zero)
    }

    func testAFullLibraryAndAnEmptyOneNeverRaiseTheWindowsMinimumSize() {
        var words = Array(repeating: 0, count: 83)
        for i in stride(from: 0, to: 83, by: 2) { words[i] = 120 * (i % 7 + 1) }
        let full = statsCounted(
            ref: "REF", words: (1_234, 12_345, 123_456), dictations: 900, wpmWeek: 142, wpmAverage: 135,
            savedWeek: 1_500_000, savedAll: 72_000_000, streak: 12, longest: 31, heatmap: words,
            meetings: #"{"meetings":14,"recorded_ms":33600000,"you_ms":12000000,"them_ms":18000000,"longest_monologue_ms":250000,"questions":37}"#,
            promises: #"{"made":14,"kept":12,"open":1,"overdue":1}"#,
            reached: ["words_1000", "words_10000", "words_50000", "words_100000", "streak_7", "streak_30"])
        for (name, answer) in [("full", full), ("empty", statsCounted(ref: "REF")), ("loading", nil)] {
            let screens = screens(answer: answer)
            XCTAssertEqual(screens.stats.counted == nil, answer == nil, "\(name): answered as meant")
            let size = minimum(screens)
            XCTAssertLessThan(size.height, 460, name)
            XCTAssertLessThan(size.width, 720, name)
        }
    }

    func testTheCardsFitANarrowContentColumn() {
        let screens = screens(answer: statsCounted(ref: "REF", words: (1_234_567, 12_345_678, 123_456_789), dictations: 5))
        guard let counted = screens.stats.counted else { return XCTFail("not answered") }
        // The narrowest the content gets: the window's 720 less the sidebar and the margins.
        let width: CGFloat = 720 - 280 - 96
        let hosting = NSHostingController(
            rootView: StatsCards(counted: counted, calendar: screens.stats.calendar).environment(screens.theme))
        let fitted = hosting.sizeThatFits(in: CGSize(width: width, height: 10_000))
        XCTAssertLessThanOrEqual(fitted.width, width + 0.5)
    }
}
