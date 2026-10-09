// How the Stats screen's numbers read. Pure: the caller passes the calendar and locale, so the
// tests pin both. Every figure keeps its assumption beside it (time saved names the typing speed),
// and speed is only ever against the user's own past.
import Foundation
import InkBridge

enum StatsFormat {
    /// A count in the user's locale: `12,345`.
    static func count(_ n: Int64, locale: Locale = .current) -> String {
        n.formatted(.number.locale(locale))
    }

    /// Time saved, always with the speed it assumes. None (or less than none: slow, paused
    /// dictation) is said as none yet, never as a negative time.
    static func saved(ms: Int64, typingWpm: Int64) -> String {
        ms > 0
            ? "\(LibraryFormat.duration(ms: ms)) saved vs typing at \(typingWpm) wpm"
            : "No time saved yet vs typing at \(typingWpm) wpm"
    }

    /// Words per minute this week against the user's own 30-day average.
    static func speed(week: Int64?, average: Int64?) -> String {
        switch (week, average) {
        case let (week?, average?): "\(week) wpm this week · your 30-day average \(average) wpm"
        case let (week?, nil): "\(week) wpm this week"
        case let (nil, average?): "No speed this week yet · your 30-day average \(average) wpm"
        case (nil, nil): "Your speed shows after a minute of dictation"
        }
    }

    /// The streak, from its second day; before that the longest, when there was one. Always in
    /// active days: one missed day between two doesn't end a streak, so it is not calendar days.
    static func streak(current: Int64, longest: Int64) -> String? {
        if current >= 2 {
            return current == longest
                ? "Streak: \(current) active days" : "Streak: \(current) active days · longest \(longest)"
        }
        return longest >= 2 ? "Longest streak: \(longest) active days" : nil
    }

    /// The streak as the Dictation card says it: running, or once it has ended, by what it reached
    /// and the longest, never as lost (the core keeps the latest). Nothing while it is hidden.
    static func streak(_ d: DictationStats) -> String? {
        if d.streakHidden == true { return nil }
        if d.streakDays >= 2 { return streak(current: d.streakDays, longest: d.longestStreakDays) }
        if let latest = d.latestStreakDays, latest >= 2, latest < d.longestStreakDays {
            return "Latest streak: \(latest) active days · longest \(d.longestStreakDays)"
        }
        return streak(current: d.streakDays, longest: d.longestStreakDays)
    }

    /// What a streak counts, said beside it.
    static let streakRule = "Active days are days you dictated. One missed day between them doesn't end a streak."

    /// The rule, with the rest days when there are some: `Rest days (Sat, Sun) neither count nor
    /// break it.`
    static func streakRule(restDays: [Int64], calendar: Calendar) -> String {
        let names = weekdays(calendar: calendar).filter { restDays.contains(Int64($0.iso)) }.map(\.short)
        guard !names.isEmpty else { return streakRule }
        return streakRule + " Rest days (\(names.joined(separator: ", "))) neither count nor break it."
    }

    /// A pause running since `since` (YYYY-MM-DD).
    static func paused(since: String, calendar: Calendar) -> String {
        let day = Self.day(since, calendar: calendar).map { shortDate($0, calendar: calendar) } ?? since
        return "Paused since \(day): days without a dictation don't count against it."
    }

    /// Once no streak is running: the days dictated this month, so an ended streak still has
    /// something to show (`14 active days this month`).
    static func activeDaysThisMonth(_ d: DictationStats) -> String? {
        guard d.streakHidden != true, d.streakDays < 2, let days = d.activeDaysMonth, days > 0 else { return nil }
        return days == 1 ? "1 active day this month" : "\(days) active days this month"
    }

    /// A weekday as Settings shows it: its ISO number (1 Monday to 7 Sunday), short and full name.
    struct Weekday: Equatable, Sendable {
        let iso: Int
        let short: String
        let name: String
    }

    /// The seven weekdays in the order the user's week runs, named in their locale.
    static func weekdays(calendar: Calendar) -> [Weekday] {
        var named = calendar
        named.locale = calendar.locale ?? .current
        let short = named.shortWeekdaySymbols
        let full = named.weekdaySymbols
        // Foundation counts Sunday as 1; ISO counts Monday as 1.
        let start = StatsModel.isoWeekStart(calendar)
        return (0..<7).map { offset in
            let iso = (start - 1 + offset) % 7 + 1
            let foundation = iso % 7
            return Weekday(iso: iso, short: short[foundation], name: full[foundation])
        }
    }

    /// A short span with its seconds: `45 s`, `4 min 10 s`, `1 h 5 min`.
    static func span(ms: Int64) -> String {
        let seconds = max(ms, 0) / 1000
        if seconds < 60 { return "\(seconds) s" }
        if seconds < 3600 {
            let (m, s) = (seconds / 60, seconds % 60)
            return s == 0 ? "\(m) min" : "\(m) min \(s) s"
        }
        let (h, m) = (seconds / 3600, (seconds / 60) % 60)
        return m == 0 ? "\(h) h" : "\(h) h \(m) min"
    }

    /// Talk time as whole percentages that add up to 100; nil when nobody spoke.
    static func talkShare(you: Int64, them: Int64) -> (you: Int, them: Int)? {
        let total = you + them
        guard total > 0 else { return nil }
        let mine = Int((Double(you) * 100 / Double(total)).rounded())
        return (mine, 100 - mine)
    }

    /// `Kept 12 of 14 this month`.
    static func kept(_ p: PromiseStats, period: String) -> String {
        "Kept \(p.kept) of \(p.made) \(period)"
    }

    /// `1 open · 1 overdue`.
    static func promiseDetail(_ p: PromiseStats) -> String {
        "\(p.open) open · \(p.overdue) overdue"
    }

    /// A milestone's count: `1,000 words`, `7 active days in a row`.
    static func milestoneTitle(kind: MilestoneKind, threshold: Int64, locale: Locale = .current) -> String {
        switch kind {
        case .words: "\(count(threshold, locale: locale)) words"
        case .streak: "\(threshold) active days in a row"
        }
    }

    /// A milestone's name (the core's key), the same on the chip, in the celebration and on the
    /// share card's seal.
    static func milestoneName(_ name: MilestoneName) -> String {
        switch name {
        case .firstPage: "First page"
        case .notebook: "A notebook"
        case .shortNovel: "A short novel"
        case .novelsWorth: "A novel's worth"
        case .sevenDays: "Seven-day run"
        case .thirtyDays: "Thirty-day run"
        case .hundredDays: "Hundred-day run"
        }
    }

    /// A milestone's chip: its name and its count, `First page · 1,000 words`.
    static func milestoneChip(_ m: MilestoneRow, locale: Locale = .current) -> String {
        let title = milestoneTitle(kind: m.kind, threshold: m.threshold, locale: locale)
        return m.name.map { "\(milestoneName($0)) · \(title)" } ?? title
    }

    /// The one line a milestone reached gets.
    static func milestoneNote(kind: MilestoneKind, threshold: Int64, name: MilestoneName? = nil, locale: Locale = .current) -> String {
        let what = switch kind {
        case .words: "\(count(threshold, locale: locale)) words dictated"
        case .streak: "\(threshold) active days in a row"
        }
        return "\(name.map(milestoneName) ?? "Milestone") · \(what)"
    }

    /// A seal's mark: the count, short (`1k`, `100k`, `30`).
    static func sealMark(kind: MilestoneKind, threshold: Int64) -> String {
        kind == .words && threshold >= 1000 && threshold % 1000 == 0 ? "\(threshold / 1000)k" : "\(threshold)"
    }

    // MARK: Time saved, pictured

    /// What time saved is about, in words, from the core's largest picture that fits:
    /// `about 2 feature films`. Nil when the core has none (too little time, or between counts).
    static func about(_ equivalents: [TimeEquivalent]?) -> String? {
        guard let first = equivalents?.first, first.count >= 1 else { return nil }
        return "about " + equivalent(first.key, count: first.count)
    }

    /// One of the core's pictures of time (TIME_EQUIVALENTS): `a lunch hour`, `3 working days`.
    static func equivalent(_ key: TimeEquivalentKey, count: Int64) -> String {
        let (one, many) = switch key {
        case .workingWeek: ("a working week", "working weeks")
        case .workingDay: ("a working day", "working days")
        case .featureFilm: ("a feature film", "feature films")
        case .lunchHour: ("a lunch hour", "lunch hours")
        case .coffeeBreak: ("a coffee break", "coffee breaks")
        }
        return count == 1 ? one : "\(count) \(many)"
    }

    /// The line under time saved: `That's about 2 working days.`
    static func savedAbout(_ equivalents: [TimeEquivalent]?) -> String? {
        about(equivalents).map { "That's \($0)." }
    }

    /// This week's time saved: `This week: 25 min, about 2 coffee breaks`.
    static func savedThisWeek(ms: Int64, about equivalents: [TimeEquivalent]?) -> String {
        "This week: \(LibraryFormat.duration(ms: ms))" + (about(equivalents).map { ", \($0)" } ?? "")
    }

    // MARK: Records

    /// A best on the Records card: what it is, its value, and when.
    struct Record: Equatable, Identifiable, Sendable {
        var id: BestId
        /// `3 min 12 s`, `168 wpm`, `2,340 words`.
        let value: String
        /// `longest dictation · 4 Oct`.
        let label: String
        /// What VoiceOver hears: `Longest dictation: 3 min 12 s, on 4 Oct`.
        let spoken: String
    }

    /// What a best is, in a line: `longest dictation`.
    static func bestName(_ id: BestId) -> String {
        switch id {
        case .longestDictation: "longest dictation"
        case .fastestDictation: "fastest dictation"
        case .mostWordsDay: "most words in a day"
        case .bestWeek: "most words in a week"
        case .longestMeeting: "longest meeting"
        case .longestMonologue: "longest monologue"
        }
    }

    /// A best's value in its unit.
    static func bestValue(_ value: Int64, unit: BestUnit, locale: Locale = .current) -> String {
        switch unit {
        case .ms: span(ms: value)
        case .wpm: "\(value) wpm"
        case .words: "\(count(value, locale: locale)) words"
        }
    }

    /// When a best was set: its day (`4 Oct`), or for the best week, `week of 28 Sep`.
    static func bestWhen(_ id: BestId, date: String, calendar: Calendar) -> String {
        let day = Self.day(date, calendar: calendar).map { shortDate($0, calendar: calendar) } ?? date
        return id == .bestWeek ? "week of \(day)" : day
    }

    /// The Records card's rows, in the core's order; none for a best not held yet.
    static func records(_ bests: [BestRow]?, calendar: Calendar) -> [Record] {
        let locale = calendar.locale ?? .current
        return (bests ?? []).map { b in
            let value = bestValue(b.value, unit: b.unit, locale: locale)
            let when = bestWhen(b.id, date: b.date, calendar: calendar)
            let name = bestName(b.id)
            return Record(
                id: b.id, value: value, label: "\(name) · \(when)",
                spoken: "\(name.prefix(1).uppercased() + name.dropFirst()): \(value), \(b.id == .bestWeek ? "the " : "on ")\(when)")
        }
    }

    /// What the Records card counts, said under it.
    static let recordsRule = "From what you dictate and record on this Mac, never an import. The fastest counts dictations of 30 seconds or more."

    /// The Records card before there is a best.
    static let recordsEmpty = "Your longest and fastest dictation, your best day and week, and your longest meeting and monologue appear here."

    /// The Drop's note for a best just set: `Longest dictation yet` over `3 min 12 s · previous best 2 min 40 s`.
    static func bestNote(_ news: BestNews, locale: Locale = .current) -> DropText {
        let title = switch news.id {
        case .longestDictation: "Longest dictation yet"
        case .fastestDictation: "Fastest dictation yet"
        case .mostWordsDay: "Most words in a day yet"
        case .bestWeek: "Most words in a week yet"
        case .longestMeeting: "Longest meeting yet"
        case .longestMonologue: "Longest monologue yet"
        }
        let (new, old) = (bestValue(news.new, unit: news.unit, locale: locale), bestValue(news.old, unit: news.unit, locale: locale))
        return DropText(title: title, detail: "\(new) · previous best \(old)", yields: true)
    }

    // MARK: Week in review

    /// Which week the review is of: `Week of 28 Sep`.
    static func reviewWeek(_ r: WeekReview, calendar: Calendar) -> String {
        let day = Self.day(r.week, calendar: calendar).map { shortDate($0, calendar: calendar) } ?? r.week
        return "Week of \(day)"
    }

    /// The review's numbers: words, time saved when there was some, meetings when there were some.
    /// A week of meetings alone does not lead with "0 words".
    static func reviewNumbers(_ r: WeekReview, locale: Locale = .current) -> [(String, String)] {
        var numbers: [(String, String)] = []
        if r.words > 0 || r.meetings == 0 {
            numbers.append((count(r.words, locale: locale), r.words == 1 ? "word" : "words"))
        }
        if let saved = r.savedMs, saved > 0 { numbers.append((LibraryFormat.duration(ms: saved), "saved")) }
        if r.meetings > 0 {
            numbers.append((LibraryFormat.duration(ms: r.meetingMs), r.meetings == 1 ? "in 1 meeting" : "in \(r.meetings) meetings"))
        }
        return numbers
    }

    /// The review's lines, gains and plain facts only: never a fall, never a red arrow.
    static func reviewLines(_ r: WeekReview, calendar: Calendar) -> [String] {
        let locale = calendar.locale ?? .current
        var lines: [String] = []
        if let best = r.bestDay, let words = r.bestDayWords, words > 0, let day = Self.day(best, calendar: calendar) {
            let weekday = day.formatted(Date.FormatStyle(locale: locale, calendar: calendar, timeZone: calendar.timeZone).weekday(.wide))
            lines.append("Busiest day: \(weekday), \(count(words, locale: locale)) words")
        }
        if let wpm = r.wpm, wpm > 0 {
            if let gain = r.wpmGain, gain > 0 {
                lines.append("\(wpm) wpm, \(gain) faster than the four weeks before")
            } else {
                lines.append("\(wpm) wpm")
            }
        }
        if let about = about(r.savedAbout) {
            lines.append("Time saved: \(about)")
        }
        if let kept = r.promisesKept, kept > 0 {
            lines.append(kept == 1 ? "1 promise from its meetings kept" : "\(kept) promises from its meetings kept")
        }
        return lines
    }

    /// `4 Oct` (or `Oct 4`) in the user's locale.
    static func shortDate(_ date: Date, calendar: Calendar) -> String {
        date.formatted(
            Date.FormatStyle(locale: calendar.locale ?? .current, calendar: calendar, timeZone: calendar.timeZone)
                .day().month(.abbreviated))
    }

    // MARK: Heatmap

    /// One day of the heatmap.
    struct Cell: Equatable, Identifiable, Sendable {
        var id: Int { index }
        /// Its place: days from the map's first.
        let index: Int
        let date: Date
        let words: Int64
        /// 0 for none, then 1 to 4 against the busiest day shown.
        let level: Int
        /// Its row: days from the first day of its week (the map's columns are weeks).
        var weekday: Int { index % 7 }
        /// Its column.
        var week: Int { index / 7 }
    }

    /// The heatmap's days, shaded by words against the busiest one.
    static func heatmap(_ d: DictationStats, calendar: Calendar) -> [Cell] {
        guard let first = day(d.heatmapFirstDay, calendar: calendar) else { return [] }
        let busiest = d.heatmapWords.max() ?? 0
        return d.heatmapWords.enumerated().map { index, words in
            let level: Int = if words <= 0 || busiest <= 0 {
                0
            } else {
                // Quarters of the busiest day: a quiet day still shows, faintly.
                min(4, max(1, Int((Double(words) * 4 / Double(busiest)).rounded(.up))))
            }
            return Cell(
                index: index, date: calendar.date(byAdding: .day, value: index, to: first) ?? first,
                words: words, level: level)
        }
    }

    /// What VoiceOver hears for the heatmap: one sentence, not a cell per day.
    static func heatmapSummary(_ cells: [Cell], calendar: Calendar, locale: Locale = .current) -> String {
        let lead = "Words dictated per day over the last 12 weeks: "
        let active = cells.filter { $0.words > 0 }
        guard let most = active.max(by: { $0.words < $1.words }) else {
            return lead + "no dictation yet."
        }
        var style = Date.FormatStyle(date: .omitted, time: .omitted, locale: locale, calendar: calendar,
                                     timeZone: calendar.timeZone)
        style = style.weekday(.abbreviated).day().month(.abbreviated)
        return lead + "\(active.count) of \(cells.count) days with dictation. "
            + "The most was \(count(most.words, locale: locale)) words, on \(most.date.formatted(style))."
    }

    /// `YYYY-MM-DD` as the start of that day in `calendar`.
    static func day(_ text: String, calendar: Calendar) -> Date? {
        let parts = text.split(separator: "-").compactMap { Int($0) }
        guard parts.count == 3 else { return nil }
        return calendar.date(from: DateComponents(year: parts[0], month: parts[1], day: parts[2]))
    }
}
