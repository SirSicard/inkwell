// Owed: what was promised in meetings and is still open, grouped, with what is overdue.
//
// The core lists the open commitments (not done, and not merged into another: a promise said twice
// is one row with "Said twice"), soonest due first. Grouping: a commitment with a named owner goes
// under that person; the user's own promises (the core leaves their owner unnamed) go under the
// meeting they were made in, because the core does not record who they were made to.
//
// "Looks done" suggestions (a later meeting where the user said it was done) show above the list
// when there are any. The core does not produce them yet.
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
    func markDone(_ id: String) {
        items.removeAll { $0.id == id }
        suggestions.removeAll { $0.commitment == id }
        send(.commitmentSetDone(id: id, done: true))
    }

    /// The user says a suggestion is wrong: it goes, the promise stays.
    func notYet(_ suggestion: LooksDone) {
        suggestions.removeAll { $0.id == suggestion.id }
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
            let key: String
            if let owner, !owner.isEmpty {
                key = "owner:" + owner.lowercased()
                titles[key] = titles[key] ?? (owner, nil)
            } else {
                key = "record:" + item.record
                titles[key] = titles[key] ?? (meetingTitle(item), meetingDay(item, now: now))
            }
            if rows[key] == nil { order.append(key) }
            rows[key, default: []].append(OwedRow(
                id: item.id, text: item.text, due: due(item, now: now), merged: Int(item.merged),
                record: item.record, saidAtMs: item.saidAtMs,
                meeting: key.hasPrefix("owner:") ? meetingTitle(item) : nil))
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
            loaded = true
        case .commitmentUpdated, .meetingCommitments:
            // A promise changed, or a meeting filed new ones: list again.
            load()
        case .commandFailed(let failure) where failure.command == "commitment.set_done":
            // Put it back as the core has it.
            load()
        default:
            break
        }
    }
}
