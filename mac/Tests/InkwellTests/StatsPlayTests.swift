// The Stats screen's five additions: personal records (and a best's note in the Drop), the gentle
// streak (rest days, a pause, hiding it), last week's review until it is dismissed, what time saved
// is about, and the share card's records, seals and heatmap. What the model sends and keeps, how
// each reads, and that the screen and Settings still fit the 720-pt window. The counting is the
// core's (ink-ffi's stats tests); nothing here counts.
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

private func fields(_ commands: [CoreCommand]) -> [[String: Any]] {
    commands.compactMap { (try? JSONSerialization.jsonObject(with: Data($0.json.utf8))) as? [String: Any] }
}

private struct NoCalendar: CalendarAccess {
    func state() -> CardState { .notAsked }
    func request(done: @escaping @MainActor @Sendable () -> Void) {}
}

/// Every best, as the core lists them.
let allBests = #","bests":[{"id":"longest_dictation","unit":"ms","value":192000,"date":"2026-10-01","record":"r1"},{"id":"fastest_dictation","unit":"wpm","value":168,"date":"2026-09-30","record":"r2"},{"id":"most_words_day","unit":"words","value":2340,"date":"2026-09-29"},{"id":"best_week","unit":"words","value":8120,"date":"2026-09-28"},{"id":"longest_meeting","unit":"ms","value":5520000,"date":"2026-09-24","record":"m1"},{"id":"longest_monologue","unit":"ms","value":250000,"date":"2026-09-24","record":"m1"}]"#

/// Last week with everything in it: a faster week, time saved, meetings and promises kept.
let fullReview = #","week_review":{"week":"2026-09-28","words":8120,"saved_ms":7800000,"saved_about":[{"key":"feature_film","count":1},{"key":"lunch_hour","count":2}],"best_day":"2026-09-29","best_day_words":2340,"wpm":142,"wpm_gain":6,"meetings":3,"meeting_ms":7500000,"promises_kept":2}"#

/// The dictation fields the gentle streak and time pictured add.
let playDictation = #","saved_about_week":[{"key":"coffee_break","count":2}],"saved_about_all":[{"key":"working_day","count":2},{"key":"feature_film","count":5}],"latest_streak_days":12,"active_days_month":3,"rest_days":[6,7],"streak_paused_since":"2026-10-02""#

@MainActor
final class StatsPlayModelTests: XCTestCase {
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

    private func setValues(_ key: String) -> [String] {
        fields(sent).filter { $0["cmd"] as? String == "setting.set" && $0["key"] as? String == key }
            .compactMap { $0["value"] as? String }
    }

    /// Rest days are written as the core reads them, ascending; all seven never are. A value the
    /// core would not take reads as none.
    func testRestDaysAreWrittenAscendingAndNeverAllSeven() {
        let stats = model()
        stats.setRestDay(7, true)
        stats.setRestDay(6, true)
        XCTAssertEqual(stats.restDays, [6, 7])
        XCTAssertEqual(setValues("stats.rest_days"), ["7", "6,7"])
        for day in 1...5 { stats.setRestDay(day, true) }
        XCTAssertEqual(stats.restDays, [1, 2, 3, 4, 6, 7], "the seventh (Friday) is refused")
        XCTAssertEqual(setValues("stats.rest_days").last, "1,2,3,4,6,7")
        XCTAssertEqual(setValues("stats.rest_days").count, 6, "nothing sent for the refused one")
        stats.setRestDay(9, true)
        XCTAssertEqual(setValues("stats.rest_days").count, 6, "no weekday nine")
        for day in 1...7 { stats.setRestDay(day, false) }
        XCTAssertEqual(stats.restDays, [])
        XCTAssertEqual(setValues("stats.rest_days").last, "none")

        XCTAssertEqual(StatsModel.restDays("6,7"), [6, 7])
        XCTAssertEqual(StatsModel.restDays("none"), [])
        XCTAssertEqual(StatsModel.restDays(nil), [])
        for odd in ["7,6", "6,6", "1,2,3,4,5,6,7", "8", "6,", "x", ""] {
            XCTAssertEqual(StatsModel.restDays(odd), [], odd)
        }
        XCTAssertEqual(StatsModel.restDaysValue([7, 1, 3]), "1,3,7")
    }

    /// Two quick changes: the core's echo of the first, arriving after the second, is not taken
    /// over it.
    func testEchoesOfEarlierStreakWritesDoNotStepBack() {
        let stats = model()
        stats.setRestDay(6, true)
        stats.setRestDay(7, true)
        stats.apply(event(#"{"type":"setting.value","key":"stats.rest_days","value":"6"}"#))
        XCTAssertEqual(stats.restDays, [6, 7])
        stats.apply(event(#"{"type":"setting.value","key":"stats.rest_days","value":"6,7"}"#))
        XCTAssertEqual(stats.restDays, [6, 7])
        stats.setStreakShown(false)
        stats.setStreakShown(true)
        stats.apply(event(#"{"type":"setting.value","key":"stats.streak","value":"hidden"}"#))
        XCTAssertTrue(stats.streakShown)
        stats.apply(event(#"{"type":"setting.value","key":"stats.streak","value":"shown"}"#))
        XCTAssertTrue(stats.streakShown)
        // Nothing of its own in flight: a value from elsewhere is taken.
        stats.apply(event(#"{"type":"setting.value","key":"stats.rest_days","value":"7"}"#))
        XCTAssertEqual(stats.restDays, [7])
    }

    /// The streak settings are read at launch and taken from the core's echoes; showing Stats
    /// counts again when the rest days or the streak's visibility change.
    func testTheStreakSettingsAreReadAndRecount() throws {
        let stats = model()
        stats.apply(event(#"{"type":"setting.value","key":"stats.rest_days","value":"6,7"}"#))
        stats.apply(event(#"{"type":"setting.value","key":"stats.streak","value":"hidden"}"#))
        stats.apply(event(#"{"type":"setting.value","key":"stats.share_heatmap","value":"on"}"#))
        XCTAssertEqual(stats.restDays, [6, 7])
        XCTAssertFalse(stats.streakShown)
        XCTAssertTrue(stats.shareHeatmap)
        stats.apply(event(#"{"type":"setting.value","key":"stats.streak","value":"shown"}"#))
        stats.apply(event(#"{"type":"setting.value","key":"stats.share_heatmap","value":"nonsense"}"#))
        XCTAssertTrue(stats.streakShown)
        XCTAssertFalse(stats.shareHeatmap, "off unless on")

        stats.setStreakShown(false)
        stats.setShareHeatmap(true)
        XCTAssertEqual(setValues("stats.streak"), ["hidden"])
        XCTAssertEqual(setValues("stats.share_heatmap"), ["on"])

        stats.screenAppeared()
        let ref = try XCTUnwrap(lastRef("stats.get"))
        stats.apply(event(statsCounted(ref: ref, dictationExtra: #","rest_days":[6,7],"streak_hidden":false"#)))
        sent.removeAll()
        // Its own echo: hidden now, counted shown, so it counts again.
        stats.apply(event(#"{"type":"setting.value","key":"stats.streak","value":"hidden"}"#))
        XCTAssertEqual(fields(sent).compactMap { $0["cmd"] as? String }, ["stats.get"])
        sent.removeAll()
        stats.apply(event(#"{"type":"setting.value","key":"stats.rest_days","value":"6,7"}"#))
        XCTAssertTrue(sent.isEmpty, "the same rest days: nothing to count again")
        stats.apply(event(#"{"type":"setting.value","key":"stats.rest_days","value":"7"}"#))
        XCTAssertEqual(fields(sent).compactMap { $0["cmd"] as? String }, ["stats.get"])

        // A failed save of one of them is said in Settings.
        stats.apply(event(#"{"type":"command.failed","command":"setting.set","id":"setting:stats.rest_days","message":"x"}"#))
        XCTAssertTrue(stats.settingsFailed)
    }

    /// A pause (or a resume) carries the user's calendar and is answered with the numbers; one at
    /// a time; a failure or no answer is said, and the button works again.
    func testAPauseIsAnsweredWithTheNumbersAndAFailureIsSaid() throws {
        let stats = model()
        stats.pauseStreak()
        let asked = try XCTUnwrap(fields(sent).last)
        XCTAssertEqual(asked["cmd"] as? String, "streak.pause")
        XCTAssertEqual(asked["week_start"] as? Int, 1)
        XCTAssertNotNil(asked["utc_offsets"] as? [[String: Any]])
        let ref = try XCTUnwrap(asked["id"] as? String)
        XCTAssertEqual(stats.streakChanging, .pausing)
        stats.resumeStreak()
        XCTAssertEqual(fields(sent).count, 1, "one change at a time")

        stats.apply(event(statsCounted(ref: ref, dictations: 3, dictationExtra: #","streak_paused_since":"2026-10-03""#)))
        XCTAssertNil(stats.streakChanging)
        XCTAssertEqual(stats.counted?.dictation.streakPausedSince, "2026-10-03")

        stats.resumeStreak()
        let resume = try XCTUnwrap(lastRef("streak.resume"))
        guard case .commandFailed(let failed) = event(#"{"type":"command.failed","command":"streak.resume","id":"\#(resume)","message":"x"}"#) else {
            return XCTFail("not a failure")
        }
        XCTAssertTrue(stats.handles(failed))
        stats.apply(.commandFailed(failed))
        XCTAssertNil(stats.streakChanging)
        XCTAssertEqual(stats.streakChangeFailed, .resuming)
        XCTAssertEqual(stats.counted?.dictation.streakPausedSince, "2026-10-03", "still paused")

        // Never answered: said once the time limit passes; a late limit for another changes nothing.
        stats.pauseStreak()
        XCTAssertNil(stats.streakChangeFailed, "a new try clears the old failure")
        let pending = try XCTUnwrap(stats.pendingStreak)
        stats.streakTimedOut("streak-0")
        XCTAssertEqual(stats.streakChanging, .pausing)
        stats.streakTimedOut(pending)
        XCTAssertEqual(stats.streakChangeFailed, .pausing)
        XCTAssertNil(stats.pendingStreak)

        // Answered after all: what it did shows, and the failure goes.
        stats.apply(event(statsCounted(ref: pending, dictations: 3, dictationExtra: #","streak_paused_since":"2026-10-03""#)))
        XCTAssertNil(stats.streakChangeFailed)
    }

    /// Review fix: a pause that is never answered while the screen has nothing to show ends in
    /// "couldn't count", not a spinner for ever (its answer was to be the screen's numbers).
    func testAPauseNeverAnsweredDoesNotSpinTheScreenForEver() throws {
        let stats = model()
        stats.load()
        stats.pauseStreak()
        let pause = try XCTUnwrap(stats.pendingStreak)
        if let get = stats.pendingLoad { stats.loadTimedOut(get) }
        XCTAssertEqual(stats.loadState, .loading, "the get's limit is no longer the one that counts")
        stats.streakTimedOut(pause)
        XCTAssertEqual(stats.loadState, .failed)
        XCTAssertNil(stats.pendingLoad)
    }

    /// A stats.get sent before a pause is answered after it would show the numbers from before:
    /// only the pause's answer is taken.
    func testAnEarlierCountDoesNotUndoAPause() throws {
        let stats = model()
        stats.load()
        let get = try XCTUnwrap(lastRef("stats.get"))
        stats.pauseStreak()
        let pause = try XCTUnwrap(lastRef("streak.pause"))
        stats.apply(event(statsCounted(ref: get, dictations: 1)))
        XCTAssertNil(stats.counted)
        stats.apply(event(statsCounted(ref: pause, dictations: 1, dictationExtra: #","streak_paused_since":"2026-10-03""#)))
        XCTAssertEqual(stats.counted?.dictation.streakPausedSince, "2026-10-03")
        XCTAssertEqual(stats.loadState, .loaded)
    }

    /// Settings counts once, the first time it shows, so the pause row knows whether one runs.
    func testSettingsCountsOnceWhenItFirstShows() {
        let stats = model()
        stats.settingsAppeared()
        XCTAssertEqual(fields(sent).compactMap { $0["cmd"] as? String }, ["stats.get"])
        stats.settingsAppeared()
        XCTAssertEqual(fields(sent).count, 1, "already asked")
    }

    /// Last week's review stays until the user dismisses it: then it goes at once, for that week,
    /// and the core keeps the dismissal. One that could not be saved comes back, and says so.
    func testTheReviewStaysUntilDismissedAndComesBackIfNotSaved() throws {
        let stats = model()
        stats.load()
        let ref = try XCTUnwrap(lastRef("stats.get"))
        stats.apply(event(statsCounted(ref: ref, words: (1, 2, 3), dictations: 2, extra: fullReview)))
        let review = try XCTUnwrap(stats.weekReview)
        XCTAssertEqual(review.week, "2026-09-28")
        stats.dismissReview(review)
        XCTAssertNil(stats.weekReview)
        XCTAssertEqual(setValues("stats.review_dismissed"), ["2026-09-28"], "the week's first day, as it came")
        stats.apply(event(#"{"type":"command.failed","command":"setting.set","id":"setting:stats.review_dismissed","message":"x"}"#))
        XCTAssertEqual(stats.weekReview, review)
        XCTAssertTrue(stats.reviewDismissFailed)
        XCTAssertFalse(stats.settingsFailed, "said on the card, not in Settings")

        // A later count of the same week still says so; the next week's review does not.
        stats.load()
        let again = try XCTUnwrap(lastRef("stats.get"))
        stats.apply(event(statsCounted(ref: again, words: (1, 2, 3), dictations: 2, extra: fullReview)))
        XCTAssertTrue(stats.reviewDismissFailed)
        stats.load()
        let next = try XCTUnwrap(lastRef("stats.get"))
        stats.apply(event(statsCounted(ref: next, words: (1, 2, 3), dictations: 2, extra: fullReview.replacingOccurrences(of: "2026-09-28", with: "2026-10-05"))))
        XCTAssertFalse(stats.reviewDismissFailed, "a new week's review was never dismissed")
        stats.load()
        let back = try XCTUnwrap(lastRef("stats.get"))
        stats.apply(event(statsCounted(ref: back, words: (1, 2, 3), dictations: 2, extra: fullReview)))

        stats.dismissReview(review)
        XCTAssertFalse(stats.reviewDismissFailed)
        stats.apply(event(#"{"type":"setting.value","key":"stats.review_dismissed","value":"2026-09-28"}"#))
        XCTAssertNil(stats.weekReview)

        // A later week's review is a new one.
        stats.load()
        let later = try XCTUnwrap(lastRef("stats.get"))
        stats.apply(event(statsCounted(ref: later, words: (1, 2, 3), dictations: 2, extra: fullReview.replacingOccurrences(of: "2026-09-28", with: "2026-10-05"))))
        XCTAssertEqual(stats.weekReview?.week, "2026-10-05")
    }

    /// A best the core just reported is a note in the Drop, plain, once per answer; an answer
    /// without one says nothing. A milestone still gets its line, by its name.
    func testABestIsANoteInTheDrop() throws {
        let best = event(#"{"type":"milestones.reached","ref":"milestones-4","milestones":[],"best":{"id":"longest_dictation","unit":"ms","old":160000,"new":192000,"date":"2026-10-03","record":"r9"}}"#)
        let note = try XCTUnwrap(DictationModel.note(for: best, hasLanguageModel: true))
        XCTAssertEqual(note, DropText(title: "Longest dictation yet", detail: "3 min 12 s · previous best 2 min 40 s", yields: true))
        XCTAssertEqual(note.tone, .plain)
        XCTAssertTrue(note.yields, "never over the take's own note")
        XCTAssertNil(DictationModel.note(for: event(#"{"type":"milestones.reached","ref":"milestones-5","milestones":[]}"#), hasLanguageModel: true))

        let dictation = DictationModel(send: { _ in })
        dictation.apply(best)
        let shown = try XCTUnwrap(dictation.note)
        dictation.apply(event(#"{"type":"milestones.reached","ref":"milestones-6","milestones":[]}"#))
        XCTAssertEqual(dictation.note, shown, "nothing new to say")

        // In the Drop: never over a note that shows or waits (the take's own, an alert above all).
        let ink = ShellInk(store: CoreStore())
        let drop = DropController(ink: ink, notes: dictation)
        dictation.apply(event(#"{"type":"dictation.inserted","outcome":"blocked","text":"x"}"#))
        drop.update()
        XCTAssertEqual(drop.noteShowing?.text.tone, .alert)
        dictation.apply(best)
        drop.update()
        XCTAssertEqual(drop.noteShowing?.text.tone, .alert, "the alert stays")
        XCTAssertNotEqual(drop.shownText?.title, "Longest dictation yet")

        let stats = model()
        stats.checkMilestones()
        let ref = try XCTUnwrap(lastRef("milestones.check"))
        stats.apply(event(#"{"type":"milestones.reached","ref":"\#(ref)","milestones":[{"id":"words_10000","kind":"words","threshold":10000,"reached":true,"name":"notebook"}],"best":{"id":"fastest_dictation","unit":"wpm","old":150,"new":168,"date":"2026-10-03"}}"#))
        XCTAssertEqual(stats.celebration?.note, "A notebook · 10,000 words dictated")
    }
}

/// One of the core's pictures of time, as it sends them.
private func picture(_ key: TimeEquivalentKey, _ count: Int) -> TimeEquivalent {
    // swiftlint:disable:next force_try
    try! JSONDecoder().decode(TimeEquivalent.self, from: Data(#"{"key":"\#(key.rawValue)","count":\#(count)}"#.utf8))
}

@MainActor
final class StatsPlayFormatTests: XCTestCase {
    private var calendar: Calendar {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(identifier: "UTC")!
        calendar.locale = Locale(identifier: "en_GB")
        return calendar
    }

    private func counted(_ json: String) throws -> StatsCounted {
        try JSONDecoder().decode(StatsCounted.self, from: Data(json.utf8))
    }

    /// Each of the core's pictures of time, one and many; always "about", never more than the
    /// core's largest that fits; nothing when the core has none.
    func testTimeSavedIsPicturedPlainly() {
        let one: [TimeEquivalentKey: String] = [
            .workingWeek: "a working week", .workingDay: "a working day", .featureFilm: "a feature film",
            .lunchHour: "a lunch hour", .coffeeBreak: "a coffee break",
        ]
        let many: [TimeEquivalentKey: String] = [
            .workingWeek: "12 working weeks", .workingDay: "3 working days", .featureFilm: "3 feature films",
            .lunchHour: "3 lunch hours", .coffeeBreak: "3 coffee breaks",
        ]
        for key in TimeEquivalentKey.allCases {
            XCTAssertEqual(StatsFormat.equivalent(key, count: 1), one[key], "\(key)")
            XCTAssertEqual(StatsFormat.equivalent(key, count: key == .workingWeek ? 12 : 3), many[key], "\(key)")
        }
        let film = [picture(.featureFilm, 2), picture(.lunchHour, 4)]
        XCTAssertEqual(StatsFormat.about(film), "about 2 feature films", "the largest first")
        XCTAssertEqual(StatsFormat.savedAbout(film), "That's about 2 feature films.")
        XCTAssertNil(StatsFormat.about(nil))
        XCTAssertNil(StatsFormat.about([]))
        XCTAssertEqual(StatsFormat.savedThisWeek(ms: 1_500_000, about: [picture(.coffeeBreak, 2)]),
                       "This week: 25 min, about 2 coffee breaks")
        XCTAssertEqual(StatsFormat.savedThisWeek(ms: 600_000, about: nil), "This week: 10 min")
        // Understated: no exclamation, no praise.
        for key in TimeEquivalentKey.allCases {
            let words = StatsFormat.equivalent(key, count: 2)
            XCTAssertFalse(words.contains("!"), words)
        }
    }

    /// Each best reads with its value in its unit and when it was set; none held, none shown.
    func testRecordsReadWithTheirDay() throws {
        let c = try counted(statsCounted(ref: "x", dictations: 9, extra: allBests))
        let records = StatsFormat.records(c.bests, calendar: calendar)
        XCTAssertEqual(records.map(\.value), ["3 min 12 s", "168 wpm", "2,340 words", "8,120 words", "1 h 32 min", "4 min 10 s"])
        // en_GB's short months, as this Mac's Foundation writes them ("Sep" or "Sept").
        let on = { (date: String) in StatsFormat.shortDate(StatsFormat.day(date, calendar: self.calendar)!, calendar: self.calendar) }
        XCTAssertEqual(on("2026-10-01"), "1 Oct")
        XCTAssertEqual(records.map(\.label), [
            "longest dictation · 1 Oct", "fastest dictation · \(on("2026-09-30"))", "most words in a day · \(on("2026-09-29"))",
            "most words in a week · week of \(on("2026-09-28"))", "longest meeting · \(on("2026-09-24"))",
            "longest monologue · \(on("2026-09-24"))",
        ])
        XCTAssertEqual(records[0].spoken, "Longest dictation: 3 min 12 s, on 1 Oct")
        XCTAssertEqual(records[3].spoken, "Most words in a week: 8,120 words, the week of \(on("2026-09-28"))")
        XCTAssertEqual(StatsFormat.records(nil, calendar: calendar), [])
        XCTAssertTrue(StatsFormat.recordsRule.contains("30 seconds"))
        XCTAssertTrue(StatsFormat.recordsRule.contains("never an import"))
    }

    /// The Drop's note says what the best is, the new value and the old: one for each best.
    func testEachBestHasItsNote() throws {
        let cases: [(String, String, Int, Int, String, String)] = [
            ("longest_dictation", "ms", 160_000, 192_000, "Longest dictation yet", "3 min 12 s · previous best 2 min 40 s"),
            ("fastest_dictation", "wpm", 150, 168, "Fastest dictation yet", "168 wpm · previous best 150 wpm"),
            ("most_words_day", "words", 1_980, 2_340, "Most words in a day yet", "2,340 words · previous best 1,980 words"),
            ("best_week", "words", 7_400, 8_120, "Most words in a week yet", "8,120 words · previous best 7,400 words"),
            ("longest_meeting", "ms", 4_200_000, 5_520_000, "Longest meeting yet", "1 h 32 min · previous best 1 h 10 min"),
            ("longest_monologue", "ms", 185_000, 250_000, "Longest monologue yet", "4 min 10 s · previous best 3 min 5 s"),
        ]
        XCTAssertEqual(Set(cases.map(\.0)), Set(BestId.allCases.map(\.rawValue)), "every best")
        for (id, unit, old, new, title, detail) in cases {
            let news = try JSONDecoder().decode(BestNews.self, from: Data(#"{"id":"\#(id)","unit":"\#(unit)","old":\#(old),"new":\#(new),"date":"2026-10-03"}"#.utf8))
            let note = StatsFormat.bestNote(news, locale: Locale(identifier: "en_GB"))
            XCTAssertEqual(note.title, title)
            XCTAssertEqual(note.detail, detail)
            XCTAssertLessThanOrEqual(note.detail.count, 44, "fits the Drop's line: \(note.detail)")
        }
    }

    /// The review says gains and plain facts: the week, its numbers, its busiest day, a faster
    /// speed, what time saved is about and the promises kept. A slower week says only its speed.
    func testTheReviewSaysGainsAndPlainFactsOnly() throws {
        let c = try counted(statsCounted(ref: "x", dictations: 9, extra: fullReview))
        let review = try XCTUnwrap(c.weekReview)
        XCTAssertTrue(StatsFormat.reviewWeek(review, calendar: calendar).hasPrefix("Week of 28 "))
        XCTAssertEqual(StatsFormat.reviewNumbers(review, locale: Locale(identifier: "en_GB")).map(\.0), ["8,120", "2 h 10 min", "2 h 5 min"])
        XCTAssertEqual(StatsFormat.reviewNumbers(review).map(\.1), ["words", "saved", "in 3 meetings"])
        XCTAssertEqual(StatsFormat.reviewLines(review, calendar: calendar), [
            "Busiest day: Tuesday, 2,340 words",
            "142 wpm, 6 faster than the four weeks before",
            "Time saved: about a feature film",
            "2 promises from its meetings kept",
        ])
        let slower = try counted(statsCounted(ref: "x", dictations: 9, extra: #","week_review":{"week":"2026-09-28","words":1,"wpm":120,"meetings":0,"meeting_ms":0}"#))
        let plain = try XCTUnwrap(slower.weekReview)
        XCTAssertEqual(StatsFormat.reviewLines(plain, calendar: calendar), ["120 wpm"])
        XCTAssertEqual(StatsFormat.reviewNumbers(plain).map(\.1), ["word"])
        // A week of meetings alone leads with them, not "0 words".
        let meetingsOnly = try XCTUnwrap(try counted(statsCounted(ref: "x", dictations: 9, extra: #","week_review":{"week":"2026-09-28","words":0,"meetings":2,"meeting_ms":3600000}"#)).weekReview)
        XCTAssertEqual(StatsFormat.reviewNumbers(meetingsOnly).map(\.1), ["in 2 meetings"])
        for line in StatsFormat.reviewLines(review, calendar: calendar) + StatsFormat.reviewLines(plain, calendar: calendar) {
            for word in ["down", "slower", "fewer", "less", "lost", "!"] {
                XCTAssertFalse(line.lowercased().contains(word), "\(line) says \(word)")
            }
        }
    }

    /// The streak once it has ended is shown by what it reached and the longest, never as lost;
    /// a pause and the rest days are said; a hidden streak says nothing.
    func testTheStreakIsGentle() throws {
        let ended = try counted(statsCounted(ref: "x", dictations: 9, streak: 0, longest: 23, dictationExtra: playDictation))
        XCTAssertEqual(StatsFormat.streak(ended.dictation), "Latest streak: 12 active days · longest 23")
        XCTAssertEqual(StatsFormat.activeDaysThisMonth(ended.dictation), "3 active days this month")
        XCTAssertEqual(StatsFormat.streakRule(restDays: [6, 7], calendar: calendar),
                       StatsFormat.streakRule + " Rest days (Sat, Sun) neither count nor break it.")
        XCTAssertEqual(StatsFormat.streakRule(restDays: [], calendar: calendar), StatsFormat.streakRule)
        XCTAssertTrue(StatsFormat.paused(since: "2026-10-02", calendar: calendar).hasPrefix("Paused since 2 Oct"))
        let running = try counted(statsCounted(ref: "x", dictations: 9, streak: 5, longest: 23, dictationExtra: playDictation))
        XCTAssertEqual(StatsFormat.streak(running.dictation), "Streak: 5 active days · longest 23")
        XCTAssertNil(StatsFormat.activeDaysThisMonth(running.dictation), "said only once no streak runs")
        let hidden = try counted(statsCounted(ref: "x", dictations: 9, streak: 5, longest: 23, dictationExtra: #","streak_hidden":true"#))
        XCTAssertNil(StatsFormat.streak(hidden.dictation))
        XCTAssertNil(StatsFormat.activeDaysThisMonth(hidden.dictation))
        for line in [StatsFormat.streak(ended.dictation)!, StatsFormat.activeDaysThisMonth(ended.dictation)!] {
            XCTAssertFalse(line.lowercased().contains("lost") || line.lowercased().contains("broke"), line)
        }
    }

    /// The weekdays run as the user's week does, named in their locale.
    func testWeekdaysFollowTheUsersWeek() {
        XCTAssertEqual(StatsFormat.weekdays(calendar: calendar).map(\.iso), [1, 2, 3, 4, 5, 6, 7])
        XCTAssertEqual(StatsFormat.weekdays(calendar: calendar).first?.name, "Monday")
        var us = Calendar(identifier: .gregorian)
        us.locale = Locale(identifier: "en_US")
        XCTAssertEqual(StatsFormat.weekdays(calendar: us).map(\.iso), [7, 1, 2, 3, 4, 5, 6])
        XCTAssertEqual(StatsFormat.weekdays(calendar: us).map(\.short).first, "Sun")
    }

    /// Each milestone's name, on its chip, in its line and as a seal's mark.
    func testMilestonesHaveNames() throws {
        XCTAssertEqual(MilestoneName.allCases.map(StatsFormat.milestoneName), [
            "First page", "A notebook", "A short novel", "A novel's worth", "Seven-day run", "Thirty-day run", "Hundred-day run",
        ])
        let c = try counted(statsCounted(ref: "x", reached: ["words_1000"], named: true))
        XCTAssertEqual(StatsFormat.milestoneChip(c.milestones[0], locale: Locale(identifier: "en_GB")), "First page · 1,000 words")
        XCTAssertEqual(StatsFormat.milestoneChip(c.milestones[4]), "Seven-day run · 7 active days in a row")
        XCTAssertEqual(StatsFormat.milestoneNote(kind: .words, threshold: 50_000, name: .shortNovel, locale: Locale(identifier: "en_GB")),
                       "A short novel · 50,000 words dictated")
        XCTAssertEqual(StatsFormat.milestoneNote(kind: .streak, threshold: 7), "Milestone · 7 active days in a row", "unnamed: as before")
        XCTAssertEqual([(MilestoneKind.words, 1000), (.words, 100_000), (.streak, 30)].map { StatsFormat.sealMark(kind: $0.0, threshold: Int64($0.1)) },
                       ["1k", "100k", "30"])
    }
}

/// The share card's additions: a records line, a seal for each milestone reached that the user
/// keeps ticked, and the heatmap only once the user has turned it on. A hidden streak is offered
/// nowhere.
@MainActor
final class SharePlayTests: XCTestCase {
    private let gb = Locale(identifier: "en_GB")
    private var calendar: Calendar {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(identifier: "UTC")!
        calendar.locale = gb
        return calendar
    }

    private func counted(_ json: String) throws -> StatsCounted {
        try JSONDecoder().decode(StatsCounted.self, from: Data(json.utf8))
    }

    func testTheCardCarriesRecordsSealsAndTheHeatmapOnlyWhenAsked() throws {
        var heat = Array(repeating: 0, count: 83)
        heat[80] = 200
        let c = try counted(statsCounted(
            ref: "x", words: (10, 20, 12_000), dictations: 40, streak: 9, longest: 9, heatmap: heat,
            reached: ["words_1000", "words_10000", "streak_7"], named: true, extra: allBests))
        let seals = ShareSeal.available(c)
        XCTAssertEqual(seals.map(\.name), ["First page", "A notebook", "Seven-day run"])
        XCTAssertEqual(seals.map(\.mark), ["1k", "10k", "7"])
        let records = try XCTUnwrap(ShareRecords.line(c, locale: gb))
        XCTAssertEqual(records.replacingOccurrences(of: "\u{00A0}", with: " "),
                       "Longest dictation: 3 min 12 s · Fastest dictation: 168 wpm · Most words in a day: 2,340")
        XCTAssertEqual(records.components(separatedBy: " ").count, 5, "it breaks only between records")

        let plain = ShareContent.make(c, selected: [.wordsAll], records: false, heatmap: false, seals: [], locale: gb, calendar: calendar)
        XCTAssertNil(plain.records)
        XCTAssertNil(plain.heatmap, "off unless the user turned it on")
        XCTAssertEqual(plain.seals, [])
        let full = ShareContent.make(c, selected: [.wordsAll], records: true, heatmap: true, seals: ["words_10000", "streak_100"],
                                     locale: gb, calendar: calendar)
        XCTAssertNotNil(full.records)
        XCTAssertEqual(full.heatmap?.count, 83)
        XCTAssertEqual(full.heatmap?[80], 4)
        XCTAssertEqual(full.seals.map(\.id), ["words_10000"], "only seals reached")
        XCTAssertTrue(full.spoken.contains("seals: A notebook"), full.spoken)
        XCTAssertTrue(full.spoken.contains("heatmap"), full.spoken)
        XCTAssertEqual(ShareContent().spoken, "The card is empty")
        XCTAssertFalse(ShareContent(seals: seals).isEmpty, "a seal alone is a card")

        // A library with no dictation has no heatmap to share, whatever the setting.
        let none = try counted(statsCounted(ref: "x"))
        XCTAssertNil(ShareContent.make(none, selected: [], records: true, heatmap: true, seals: [], locale: gb, calendar: calendar).heatmap)
        XCTAssertNil(ShareRecords.line(none, locale: gb))

        for dark in [false, true] {
            let png = try XCTUnwrap(StatsShare.png(content: full, dark: dark, you: .blue, them: .orange))
            let image = try XCTUnwrap(NSBitmapImageRep(data: png))
            XCTAssertEqual(image.pixelsWide, Int(ShareCard.width * StatsShare.scale), "the seals and the heatmap fit its width")
        }
    }

    func testAHiddenStreakIsOfferedNowhere() throws {
        let c = try counted(statsCounted(ref: "x", words: (1, 2, 3), dictations: 2, streak: 9, longest: 9, dictationExtra: #","streak_hidden":true"#))
        XCTAssertFalse(ShareStat.streak.available(in: c))
        XCTAssertFalse(ShareStat.lines(c, selected: [.streak], locale: gb).contains { $0.label.contains("streak") })
    }

    /// With every number, the records, the heatmap and the seals, the sheet still fits a laptop.
    func testTheSheetFitsWithEverything() throws {
        let c = try counted(statsCounted(
            ref: "x", words: (10, 1_234, 56_789), dictations: 40, wpmWeek: 142, savedAll: 12_000_000, streak: 12, longest: 31,
            meetings: #"{"meetings":3,"recorded_ms":5400000,"you_ms":1000,"them_ms":3000,"longest_monologue_ms":1,"questions":2}"#,
            promises: #"{"made":14,"kept":12,"open":2,"overdue":0}"#,
            reached: ["words_1000", "words_10000", "words_50000", "words_100000", "streak_7", "streak_30", "streak_100"],
            named: true, extra: allBests))
        let stats = StatsModel(send: { _ in })
        stats.apply(event(#"{"type":"setting.value","key":"stats.share_heatmap","value":"on"}"#))
        let theme = GlowTheme(send: { _ in }, applyAppearance: { _ in })
        let hosting = NSHostingController(rootView: ShareCardSheet(counted: c, stats: stats).environment(theme))
        let size = hosting.sizeThatFits(in: CGSize(width: 10_000, height: 10_000))
        XCTAssertLessThanOrEqual(size.width, 640.5)
        XCTAssertLessThan(size.height, 760, "a 13-inch screen's height, less the menu bar and the window's title")
    }
}

/// The screen with last week's review and every record, in the app's window at its 720-pt minimum:
/// nothing widens the window or runs past the page.
@MainActor
final class StatsPlayLayoutTests: XCTestCase {
    private final class NoEvents: UpcomingEvents {
        func nextEvent(after now: Date, within horizon: TimeInterval) -> UpcomingEvent? { nil }
    }

    private func full() -> String {
        var words = Array(repeating: 0, count: 83)
        for i in stride(from: 0, to: 83, by: 2) { words[i] = 120 * (i % 7 + 1) }
        return statsCounted(
            ref: "REF", words: (1_234, 12_345, 123_456), dictations: 900, wpmWeek: 142, wpmAverage: 135,
            savedWeek: 1_500_000, savedAll: 72_000_000, streak: 0, longest: 31, heatmap: words,
            meetings: #"{"meetings":14,"recorded_ms":33600000,"you_ms":12000000,"them_ms":18000000,"longest_monologue_ms":250000,"questions":37}"#,
            promises: #"{"made":14,"kept":12,"open":1,"overdue":1}"#,
            reached: ["words_1000", "words_10000", "words_50000", "words_100000", "streak_7", "streak_30"], named: true,
            dictationExtra: playDictation, extra: allBests + fullReview)
    }

    private func screens() throws -> ScreenModels {
        var sent: [CoreCommand] = []
        let screens = ScreenModels(send: { sent.append($0) }, calendar: NoCalendar(), apps: WorkspaceApps())
        screens.stats.apply(event(#"{"type":"setting.value","key":"stats.rest_days","value":"6,7"}"#))
        screens.stats.apply(event(#"{"type":"setting.value","key":"stats.share_heatmap","value":"on"}"#))
        screens.stats.load()
        let ref = try XCTUnwrap(sent.last?.commandID)
        screens.stats.apply(event(full().replacingOccurrences(of: "REF", with: ref)))
        // The review's failure line too: the tallest the card gets.
        let review = try XCTUnwrap(screens.stats.weekReview)
        screens.stats.dismissReview(review)
        screens.stats.apply(event(#"{"type":"command.failed","command":"setting.set","id":"setting:stats.review_dismissed","message":"x"}"#))
        XCTAssertTrue(screens.stats.reviewDismissFailed)
        return screens
    }

    func testTheCardsFitTheNarrowestContentColumn() throws {
        let screens = try screens()
        let counted = try XCTUnwrap(screens.stats.counted)
        XCTAssertNotNil(screens.stats.weekReview)
        // The narrowest the content gets: the window's 720 less the sidebar and the margins.
        let width: CGFloat = 720 - 280 - 96
        let hosting = NSHostingController(
            rootView: StatsCards(counted: counted, calendar: screens.stats.calendar, review: screens.stats.weekReview,
                                 reviewFailed: true).environment(screens.theme))
        let fitted = hosting.sizeThatFits(in: CGSize(width: width, height: 10_000))
        XCTAssertLessThanOrEqual(fitted.width, width + 0.5)
    }

    /// The Records card's columns: as many as fit at their least width, never narrower than it
    /// (one column when even one doesn't fit), never more than there are records.
    func testRecordsTakeEvenColumnsThatFit() {
        let columns = { (width: CGFloat, count: Int) in EvenColumns.columns(width, minimum: 150, spacing: 24, count: count) }
        XCTAssertEqual(columns(300, 6).count, 1)
        XCTAssertEqual(columns(324, 6).count, 2)
        XCTAssertEqual(columns(324, 6).width, 150)
        XCTAssertEqual(columns(560, 6).count, 3)
        XCTAssertEqual(columns(560, 2).count, 2, "no empty columns")
        XCTAssertEqual(columns(100, 6).width, 100, "one column, as wide as there is")
        for width in stride(from: CGFloat(100), through: 900, by: 7) {
            let (n, w) = columns(width, 6)
            XCTAssertLessThanOrEqual(CGFloat(n) * w + CGFloat(n - 1) * 24, width + 0.5)
            if n > 1 { XCTAssertGreaterThanOrEqual(w, 150) }
        }
    }

    func testTheScreenKeepsTheWindowAt720() throws {
        let (window, content) = try layOut(try screens(), width: 720)
        defer { window.close() }
        XCTAssertEqual(content.frame.width, 720, accuracy: 0.5, "the screen widened the window")
        XCTAssertEqual(window.contentMinSize.width, 720, accuracy: 0.5)
        // The page: the scroll view with the tallest document, no wider than the window.
        let page = try XCTUnwrap(scrollViews(in: content).max { ($0.documentView?.frame.height ?? 0) < ($1.documentView?.frame.height ?? 0) })
        XCTAssertLessThanOrEqual(page.frame.width, 720.5)
        XCTAssertLessThanOrEqual(page.documentView?.frame.width ?? 0, page.frame.width + 0.5)
    }

    /// Not a check: the Stats page, the Settings page and the share card drawn offscreen in Light
    /// and Dark at 720 and 1040 for the eye, when INK_STATS_RENDER names a folder. AppKit's cache
    /// draws neither the orb (Metal) nor the cards' blur: what it shows is the layout.
    func testRenderStats() throws {
        guard let folder = ProcessInfo.processInfo.environment["INK_STATS_RENDER"] else {
            throw XCTSkip("INK_STATS_RENDER is not set")
        }
        let out = URL(fileURLWithPath: folder, isDirectory: true)
        try FileManager.default.createDirectory(at: out, withIntermediateDirectories: true)
        let screens = try screens()
        defer { NSApp.appearance = nil }
        for mode in [GlowTheme.Mode.light, .dark] {
            screens.theme.setMode(mode)
            for route in [Route.stats, .settings] {
                for width in [720, 1040] as [CGFloat] {
                    let (window, content) = try layOut(screens, width: width, route: route)
                    let page = try XCTUnwrap(scrollViews(in: content).max { ($0.documentView?.frame.height ?? 0) < ($1.documentView?.frame.height ?? 0) })
                    try draw(try XCTUnwrap(page.documentView), to: out.appendingPathComponent("\(route.rawValue)-\(mode.rawValue)-\(Int(width)).png"))
                    window.close()
                }
            }
            let counted = try XCTUnwrap(screens.stats.counted)
            let content = ShareContent.make(
                counted, selected: ShareStat.defaults, records: true, heatmap: true,
                seals: Set(ShareSeal.available(counted).map(\.id)), locale: Locale(identifier: "en_GB"), calendar: screens.stats.calendar)
            let png = try XCTUnwrap(StatsShare.png(content: content, dark: mode == .dark, you: screens.theme.you, them: screens.theme.them))
            try png.write(to: out.appendingPathComponent("share-\(mode.rawValue).png"))
        }
    }

    private func draw(_ view: NSView, to url: URL) throws {
        let rep = try XCTUnwrap(view.bitmapImageRepForCachingDisplay(in: view.bounds))
        view.cacheDisplay(in: view.bounds, to: rep)
        try XCTUnwrap(rep.representation(using: .png, properties: [:])).write(to: url)
    }

    private func layOut(_ screens: ScreenModels, width: CGFloat, route: Route = .stats) throws -> (NSWindow, NSView) {
        let store = CoreStore()
        let router = Router()
        router.open(route)
        let root = ShellView(router: router).environment(store).environment(ShellInk(store: store))
            .environment(Updates(infoDictionary: nil)).environment(screens).environment(LibraryModel(send: { _ in }))
            .environment(UpNextModel(access: NoCalendar(), events: NoEvents())).environment(router)
            .environment(WindowPresence()).environment(screens.theme).tint(Theme.buttonFill)
        let window = MainWindowController.makeWindow(root: root)
        let sentinel = NSSize(width: 100, height: 100)
        window.contentMinSize = sentinel
        window.setContentSize(NSSize(width: width, height: 700))
        let content = try XCTUnwrap(window.contentViewController?.view)
        let deadline = Date().addingTimeInterval(2)
        repeat {
            content.layoutSubtreeIfNeeded()
            RunLoop.main.run(until: Date().addingTimeInterval(0.05))
        } while window.contentMinSize == sentinel && Date() < deadline
        RunLoop.main.run(until: Date().addingTimeInterval(0.2))
        content.layoutSubtreeIfNeeded()
        XCTAssertNotEqual(window.contentMinSize, sentinel, "SwiftUI set the minimum")
        return (window, content)
    }

    private func scrollViews(in view: NSView) -> [NSScrollView] {
        view.subviews.flatMap { ([$0 as? NSScrollView].compactMap { $0 }) + scrollViews(in: $0) }
    }
}
