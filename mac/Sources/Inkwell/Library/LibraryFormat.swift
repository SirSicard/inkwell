// How the library's times, lengths and records read on screen. Pure: the caller passes the moment
// and the calendar (time zone and locale), so the tests pin both.
import Foundation
import InkBridge

enum LibraryFormat {
    /// A position in a record: `12:41`, or `1:02:05` from an hour on.
    static func stamp(ms: Int64) -> String {
        let seconds = max(ms, 0) / 1000
        let (h, m, s) = (seconds / 3600, (seconds / 60) % 60, seconds % 60)
        return h > 0
            ? String(format: "%lld:%02lld:%02lld", h, m, s)
            : String(format: "%02lld:%02lld", m, s)
    }

    /// A length: `under 1 min`, `42 min`, `2 h 10 min`.
    static func duration(ms: Int64) -> String {
        let minutes = Int((Double(max(ms, 0)) / 60_000).rounded())
        if minutes < 1 { return "under 1 min" }
        if minutes < 60 { return "\(minutes) min" }
        let (h, m) = (minutes / 60, minutes % 60)
        return m == 0 ? "\(h) h" : "\(h) h \(m) min"
    }

    /// The day relative to `now`: `Today`, `Yesterday`, else `Mon 21 Sep`.
    static func day(_ date: Date, now: Date, calendar: Calendar) -> String {
        if calendar.isDate(date, inSameDayAs: now) { return "Today" }
        if let yesterday = calendar.date(byAdding: .day, value: -1, to: now),
            calendar.isDate(date, inSameDayAs: yesterday)
        {
            return "Yesterday"
        }
        return date.formatted(
            Date.FormatStyle(timeZone: calendar.timeZone)
                .locale(calendar.locale ?? .current)
                .weekday(.abbreviated).day().month(.abbreviated))
    }

    /// The clock time: `14:02` (or the locale's form).
    static func time(_ date: Date, calendar: Calendar) -> String {
        date.formatted(
            Date.FormatStyle(timeZone: calendar.timeZone)
                .locale(calendar.locale ?? .current)
                .hour(.defaultDigits(amPM: .abbreviated)).minute())
    }

    /// Today's heading: `Saturday 27 September`.
    static func longDay(_ date: Date, calendar: Calendar) -> String {
        date.formatted(
            Date.FormatStyle(timeZone: calendar.timeZone)
                .locale(calendar.locale ?? .current)
                .weekday(.wide).day().month(.wide))
    }

    /// A greeting for the hour.
    static func greeting(_ date: Date, calendar: Calendar) -> String {
        switch calendar.component(.hour, from: date) {
        case 5..<12: "Good morning"
        case 12..<18: "Good afternoon"
        default: "Good evening"
        }
    }

    static func date(unixMs: Int64) -> Date {
        Date(timeIntervalSince1970: Double(unixMs) / 1000)
    }

    /// A record's length, when it has ended.
    static func length(of record: RecordRow) -> Int64? {
        record.endedAtUnixMs.map { max($0 - record.startedAtUnixMs, 0) }
    }

    /// What a record is called: its title, else the start of its words, else what it is. Never a
    /// placeholder like "Untitled" when there is something better to say.
    static func title(of record: RecordRow) -> String {
        if let title = record.title?.trimmingCharacters(in: .whitespacesAndNewlines), !title.isEmpty {
            return title
        }
        if let preview = record.preview, !preview.isEmpty {
            return preview
        }
        switch record.kind {
        case .meeting: return "Meeting"
        case .dictation: return "Dictation"
        case .fileImport: return "Imported file"
        }
    }

    /// A list row's second line: `Today · 14:02 · 42 min`.
    static func listLine(_ record: RecordRow, now: Date, calendar: Calendar) -> String {
        let start = date(unixMs: record.startedAtUnixMs)
        var parts = [day(start, now: now, calendar: calendar), time(start, calendar: calendar)]
        if let length = length(of: record) {
            parts.append(duration(ms: length))
        } else if record.kind == .meeting {
            parts.append("recording")
        }
        return parts.joined(separator: " · ")
    }

    /// A record's heading line: `Today 14:02 · 42 min · Zoom · You, Alex, Robin`.
    static func headerLine(_ record: RecordRow, people: [String], now: Date, calendar: Calendar) -> String {
        let start = date(unixMs: record.startedAtUnixMs)
        var parts = ["\(day(start, now: now, calendar: calendar)) \(time(start, calendar: calendar))"]
        if let length = length(of: record) {
            parts.append(duration(ms: length))
        }
        if let app = record.sourceApp, !app.isEmpty {
            parts.append(app)
        }
        if !people.isEmpty {
            parts.append(people.joined(separator: ", "))
        }
        return parts.joined(separator: " · ")
    }
}
