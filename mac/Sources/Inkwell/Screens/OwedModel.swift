// Owed: what was promised in meetings and is still open, grouped, with what is overdue.
//
// The core lists the open commitments (not done, and not merged into another: a promise said twice
// is one row with "Said twice"), soonest due first. Grouping: by the person a promise is owed to
// ("To Dana") when the meeting said; else, a commitment with a named owner under that person; the
// rest under the meeting they were made in.
//
// "Looks done" suggestions show above the list: a later meeting in which the user said the work
// was already done (the core matches what was said to the open promises). "Mark done" closes the
// promise; "Not yet" dismisses the suggestion, and the promise stays.
import Foundation
import InkBridge
import Observation

/// When something is due, as the screen shows it.
enum DueLabel: Equatable {
    case undated
    /// Before today: days late.
    case overdue(days: Int)
    case today
    case tomorrow
    /// Within the next six days: the weekday.
    case thisWeek(Date)
    case later(Date)

    var isOverdue: Bool {
        if case .overdue = self { true } else { false }
    }

    /// Due today or within the week, or late.
    var isThisWeek: Bool {
        switch self {
        case .today, .tomorrow, .thisWeek, .overdue: true
        case .undated, .later: false
        }
    }

    func text(calendar: Calendar) -> String {
        switch self {
        case .undated: return "No date"
        case .overdue(let days): return days == 1 ? "1 day overdue" : "\(days) days overdue"
        case .today: return "Due today"
        case .tomorrow: return "Due tomorrow"
        case .thisWeek(let date):
            return "Due " + date.formatted(.dateTime.weekday(.abbreviated))
        case .later(let date):
            return "Due " + date.formatted(.dateTime.day().month(.abbreviated))
        }
    }

    static func of(_ due: Date?, now: Date, calendar: Calendar) -> DueLabel {
        guard let due else { return .undated }
        let today = calendar.startOfDay(for: now)
        let day = calendar.startOfDay(for: due)
        let days = calendar.dateComponents([.day], from: today, to: day).day ?? 0
        switch days {
        case ..<0: return .overdue(days: -days)
        // Due dates are days ("by Friday"): anything due today is due today, not late.
        case 0: return .today
        case 1: return .tomorrow
        case 2...6: return .thisWeek(due)
        default: return .later(due)
        }
    }
}

/// One promise as the screen lists it.
struct OwedRow: Equatable, Identifiable {
    let id: String
    let text: String
    let due: DueLabel
    /// Times it was said again and merged into this one.
    let merged: Int
    /// The record it was said in, and where.
    let record: String
    let saidAtMs: Int64?
    /// The meeting's title, for a row listed under a person.
    let meeting: String?

    /// "Said twice", "Said 3 times", or nil.
    var mergedText: String? {
        switch merged {
        case 0: nil
        case 1: "Said twice · merged"
        default: "Said \(merged + 1) times · merged"
        }
    }
}

/// A heading and its promises.
struct OwedGroup: Equatable, Identifiable {
    let id: String
    let title: String
    let subtitle: String?
    let rows: [OwedRow]
}

/// A later meeting suggests a promise was kept.
struct LooksDone: Equatable, Identifiable {
    let id: String
    /// The commitment it would close.
    let commitment: String
    /// What was said, in the user's words.
    let text: String
    /// Where: the meeting and the time into it.
    let source: String
}

@MainActor
@Observable
final class OwedModel {
    private(set) var items: [OwedItem] = []
    /// Nothing has been listed yet.
    private(set) var loaded = false
    private(set) var suggestions: [LooksDone] = []
    /// The last answer the core refused, in words, until the next answer: the promise is back in
    /// the list as the core has it.
    private(set) var failure: String?
    /// The promise just marked done, while its Undo is offered.
    private(set) var undoable: Undoable?

    /// A promise marked done that can be put back.
    struct Undoable: Equatable {
        /// Increases with every mark, so a later one replaces the toast.
        let serial: Int
        let id: String
        let text: String
    }

    @ObservationIgnored private var undoSerial = 0
    /// What the last commitment.set_done asked: done (true) or open again (an Undo).
    @ObservationIgnored private var lastSetDone = true

    @ObservationIgnored private let send: SendCommand
    @ObservationIgnored private let calendar: Calendar

    init(send: @escaping SendCommand, calendar: Calendar = .current) {
        self.send = send
        self.calendar = calendar
    }

    func load() {
        send(.commitmentsList)
    }

    /// Marks a promise done: it leaves the list at once, and the core's list replaces it.
    /// `offerUndo`: the screen asking shows the Undo toast (Owed), until the next mark or until the
    /// toast goes.
    func markDone(_ id: String, offerUndo: Bool = false) {
        failure = nil
        undoable = nil
        if offerUndo, let item = items.first(where: { $0.id == id }) {
            undoSerial += 1
            undoable = Undoable(serial: undoSerial, id: id, text: item.text)
        }
        items.removeAll { $0.id == id }
        suggestions.removeAll { $0.commitment == id }
        lastSetDone = true
        send(.commitmentSetDone(id: id, done: true))
    }

    /// Undo: the promise is open again (the core's list brings it back).
    func undoDone() {
        guard let undoable else { return }
        self.undoable = nil
        failure = nil
        lastSetDone = false
        send(.commitmentSetDone(id: undoable.id, done: false))
    }

    /// The toast's time is up.
    func expireUndo(_ serial: Int) {
        if undoable?.serial == serial { undoable = nil }
    }

    /// The user says a suggestion is wrong: it goes, the promise stays.
    func notYet(_ suggestion: LooksDone) {
        failure = nil
        suggestions.removeAll { $0.id == suggestion.id }
        send(.commitmentNotYet(id: suggestion.commitment))
    }

    /// The suggestions in the core's list: each open promise a later meeting says looks done.
    static func suggestions(_ items: [OwedItem]) -> [LooksDone] {
        items.compactMap { item in
            guard let evidence = item.looksDone else { return nil }
            let meeting = evidence.recordTitle?.trimmingCharacters(in: .whitespaces).nilIfEmpty
                ?? evidence.recordStartedAtUnixMs.map {
                    "a meeting on " + Date(timeIntervalSince1970: Double($0) / 1_000)
                        .formatted(.dateTime.weekday(.abbreviated).day().month(.abbreviated))
                }
                ?? "a later meeting"
            let said = evidence.text?.trimmingCharacters(in: .whitespacesAndNewlines).nilIfEmpty
            let text = said.map { "in \(meeting) you said \u{201C}\($0)\u{201D}" }
                ?? "\(meeting) suggests \u{201C}\(item.text)\u{201D} is done"
            let title = evidence.recordTitle?.trimmingCharacters(in: .whitespaces).nilIfEmpty ?? "That meeting"
            return LooksDone(
                id: "looks-done:\(item.id)", commitment: item.id, text: text,
                source: "\(title) ▸ \(liveClock(ms: evidence.span.startMs))")
        }
    }

    /// "5 open · 2 due this week · 1 overdue".
    func summary(now: Date) -> String {
        let dues = items.map { due($0, now: now) }
        var parts = ["\(items.count) open"]
        let week = dues.filter { $0.isThisWeek && !$0.isOverdue }.count
        let late = dues.filter(\.isOverdue).count
        if week > 0 { parts.append("\(week) due this week") }
        if late > 0 { parts.append("\(late) overdue") }
        return parts.joined(separator: " · ")
    }

    /// How many promises are late (the sidebar's count).
    func overdueCount(now: Date) -> Int {
        items.count { due($0, now: now).isOverdue }
    }

    func due(_ item: OwedItem, now: Date) -> DueLabel {
        DueLabel.of(
            item.dueAtUnixMs.map { Date(timeIntervalSince1970: Double($0) / 1_000) },
            now: now, calendar: calendar)
    }

    /// The groups, in the order of their soonest promise (the core's order).
    func groups(now: Date) -> [OwedGroup] {
        var order: [String] = []
        var titles: [String: (String, String?)] = [:]
        var rows: [String: [OwedRow]] = [:]
        for item in items {
            let owner = item.owner?.trimmingCharacters(in: .whitespaces)
            let recipient = item.recipient?.trimmingCharacters(in: .whitespaces)
            // The summary names people as the transcript does: "You", "Them", a name. The user's
            // own promises are grouped by meeting, like those with no owner; "Them" alone is no
            // heading.
            let ownedBySomeoneElse = owner.map { !$0.isEmpty && $0.lowercased() != "you" } ?? false
            let key: String
            if let recipient, !recipient.isEmpty {
                key = "to:" + recipient.lowercased()
                titles[key] = titles[key] ?? (recipient.lowercased() == "you" ? "Owed to you" : "To " + recipient, nil)
            } else if let owner, ownedBySomeoneElse {
                key = "owner:" + owner.lowercased()
                titles[key] = titles[key] ?? (owner.lowercased() == "them" ? "Owed by the others" : owner, nil)
            } else {
                key = "record:" + item.record
                // An untitled meeting is already named by its day: no second date beside it.
                let titled = item.recordTitle?.trimmingCharacters(in: .whitespaces).isEmpty == false
                titles[key] = titles[key] ?? (meetingTitle(item), titled ? meetingDay(item, now: now) : nil)
            }
            if rows[key] == nil { order.append(key) }
            rows[key, default: []].append(OwedRow(
                id: item.id, text: item.text, due: due(item, now: now), merged: Int(item.merged),
                record: item.record, saidAtMs: item.saidAtMs,
                meeting: key.hasPrefix("record:") ? nil : meetingTitle(item)))
        }
        return order.map { key in
            OwedGroup(id: key, title: titles[key]?.0 ?? "", subtitle: titles[key]?.1, rows: rows[key] ?? [])
        }
    }

    private func started(_ item: OwedItem) -> Date {
        Date(timeIntervalSince1970: Double(item.recordStartedAtUnixMs) / 1_000)
    }

    private func meetingTitle(_ item: OwedItem) -> String {
        if let title = item.recordTitle?.trimmingCharacters(in: .whitespaces), !title.isEmpty {
            return title
        }
        return "A meeting on " + started(item).formatted(.dateTime.weekday(.abbreviated).day().month(.abbreviated))
    }

    private func meetingDay(_ item: OwedItem, now: Date) -> String {
        let day = started(item)
        if calendar.isDate(day, inSameDayAs: now) { return "today" }
        if let yesterday = calendar.date(byAdding: .day, value: -1, to: now), calendar.isDate(day, inSameDayAs: yesterday) {
            return "yesterday"
        }
        return day.formatted(.dateTime.weekday(.abbreviated).day().month(.abbreviated))
    }

    func apply(_ event: InkEvent) {
        switch event {
        case .commitmentsListed(let listed):
            items = listed.items
            suggestions = Self.suggestions(listed.items)
            loaded = true
        case .commitmentUpdated, .meetingCommitments, .meetingLooksDone, .librarySwept, .recordDeleted:
            // A promise changed, a meeting filed new ones or found some done, or old ones (or a
            // record the user deleted) went: list again.
            load()
        case .commandFailed(let failed) where ["commitment.set_done", "commitment.not_yet"].contains(failed.command):
            // Nothing to undo: the core did not take it.
            undoable = nil
            // Put it back as the core has it, and say why it came back.
            failure = failed.command != "commitment.set_done"
                ? "Couldn't keep it open: \(failed.message)"
                : lastSetDone ? "Couldn't mark it done: \(failed.message)" : "Couldn't open it again: \(failed.message)"
            load()
        default:
            break
        }
    }
}

private extension String {
    var nilIfEmpty: String? { isEmpty ? nil : self }
}
