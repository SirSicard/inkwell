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

    /// The streak, from its second day; before that the longest, when there was one.
    static func streak(current: Int64, longest: Int64) -> String? {
        if current >= 2 {
            return current == longest ? "\(current)-day streak" : "\(current)-day streak · longest \(longest) days"
        }
        return longest >= 2 ? "Longest streak \(longest) days" : nil
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

    /// A milestone's name: `1,000 words`, `7-day streak`.
    static func milestoneTitle(kind: MilestoneKind, threshold: Int64, locale: Locale = .current) -> String {
        switch kind {
        case .words: "\(count(threshold, locale: locale)) words"
        case .streak: "\(threshold)-day streak"
        }
    }

    /// The one line a milestone reached gets.
    static func milestoneNote(kind: MilestoneKind, threshold: Int64, locale: Locale = .current) -> String {
        switch kind {
        case .words: "Milestone · \(count(threshold, locale: locale)) words dictated"
        case .streak: "Milestone · a \(threshold)-day streak"
        }
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
