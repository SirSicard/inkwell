// Stats: the user's dictation (words, speed against their own past, time saved with its
// assumption, the streak and a heatmap), their meetings (hours, talk time by side, the longest
// monologue, their questions), promises kept, and milestones. Translucent cards over the orb, in
// Glow's type; every number has a VoiceOver label, and the heatmap one sentence.
//
// A new library says plainly what will appear. A count that could not be made says so, never
// zero.
import InkBridge
import SwiftUI

struct StatsScreen: View {
    @Environment(ScreenModels.self) private var screens

    var body: some View {
        let stats = screens.stats
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                Paper.Header(title: "Stats", subtitle: "Counted on this Mac from your library. Nothing leaves it.")
                if let counted = stats.counted {
                    if stats.loadState == .failed {
                        Text("Couldn't count again. These are the numbers from before.")
                            .font(Typography.caption)
                            .foregroundStyle(Theme.alert)
                    }
                    StatsCards(counted: counted, calendar: stats.calendar)
                } else if stats.loadState == .failed {
                    HStack(spacing: 12) {
                        Text("Couldn't count your stats.")
                            .font(Typography.body)
                            .foregroundStyle(Theme.alert)
                        Button("Try again") { stats.load() }
                            .buttonStyle(PaperButtonStyle())
                    }
                } else {
                    ProgressView().controlSize(.small)
                        .accessibilityLabel("Counting your stats")
                }
            }
            .frame(maxWidth: 860, alignment: .leading)
            .padding(.horizontal, 48)
            .padding(.top, 34)
            .padding(.bottom, 28)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .scrollContentBackground(.hidden)
        .onAppear { stats.screenAppeared() }
        .onDisappear { stats.screenDisappeared() }
    }
}

/// The four cards, one under the other.
struct StatsCards: View {
    let counted: StatsCounted
    let calendar: Calendar

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            DictationCard(counted: counted, calendar: calendar)
            MeetingsCard(counted: counted)
            PromisesCard(counted: counted)
            MilestonesCard(milestones: counted.milestones, locale: calendar.locale ?? .current)
        }
    }
}

/// A card's frame: the eyebrow over the content, padded, translucent over the orb.
private struct StatsCard<Content: View>: View {
    let title: String
    @ViewBuilder var content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Paper.Eyebrow(text: title)
            content
        }
        .padding(.horizontal, 22)
        .padding(.vertical, 18)
        .frame(maxWidth: .infinity, alignment: .leading)
        .paperCard()
        .accessibilityElement(children: .contain)
    }
}

/// A number over its label, read as one: "1,234 words today".
struct BigNumber: View {
    let value: String
    let label: String

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(value)
                .font(Typography.heading)
                .foregroundStyle(Theme.text)
                .monospacedDigit()
            Text(label)
                .font(Typography.caption)
                .foregroundStyle(Theme.secondaryText)
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("\(value) \(label)")
    }
}

/// Numbers side by side when they fit, one under another when not.
private struct NumberRow: View {
    let numbers: [(String, String)]

    var body: some View {
        ViewThatFits(in: .horizontal) {
            HStack(alignment: .top, spacing: 32) {
                ForEach(numbers, id: \.1) { BigNumber(value: $0.0, label: $0.1) }
                Spacer(minLength: 0)
            }
            VStack(alignment: .leading, spacing: 10) {
                ForEach(numbers, id: \.1) { BigNumber(value: $0.0, label: $0.1) }
            }
        }
    }
}

/// A line of the body text that wraps.
private struct Line: View {
    let text: String
    var secondary = false

    var body: some View {
        Text(text)
            .font(secondary ? Typography.caption : Typography.body)
            .foregroundStyle(secondary ? Theme.secondaryText : Theme.text)
            .fixedSize(horizontal: false, vertical: true)
    }
}

private struct DictationCard: View {
    let counted: StatsCounted
    let calendar: Calendar

    var body: some View {
        let d = counted.dictation
        let locale = calendar.locale ?? .current
        StatsCard(title: "Dictation") {
            if d.dictationsAll == 0 {
                Line(text: "Your words, speed and streak appear here after your first dictation.", secondary: true)
            } else {
                NumberRow(numbers: [
                    (StatsFormat.count(d.wordsToday, locale: locale), "words today"),
                    (StatsFormat.count(d.wordsWeek, locale: locale), "this week"),
                    (StatsFormat.count(d.wordsAll, locale: locale), "all time"),
                ])
                Line(text: StatsFormat.speed(week: d.wpmWeek, average: d.wpmAverage))
                Line(text: StatsFormat.saved(ms: d.savedMsAll, typingWpm: counted.typingWpm))
                if d.savedMsWeek > 0 {
                    Line(text: "This week: \(LibraryFormat.duration(ms: d.savedMsWeek))", secondary: true)
                }
                if let streak = StatsFormat.streak(current: d.streakDays, longest: d.longestStreakDays) {
                    Line(text: streak)
                }
            }
            Heatmap(cells: StatsFormat.heatmap(d, calendar: calendar), calendar: calendar)
        }
    }
}

/// Words per day over the last 12 weeks: a column per week, a row per weekday, shaded in your
/// colour by words against the busiest day. VoiceOver reads one sentence for it.
struct Heatmap: View {
    let cells: [StatsFormat.Cell]
    let calendar: Calendar
    @Environment(GlowTheme.self) private var theme

    static let cell: CGFloat = 12
    static let gap: CGFloat = 3

    var body: some View {
        let weeks = (cells.last?.week ?? 0) + 1
        VStack(alignment: .leading, spacing: 6) {
            HStack(alignment: .top, spacing: Self.gap) {
                ForEach(0..<weeks, id: \.self) { week in
                    VStack(spacing: Self.gap) {
                        ForEach(0..<7, id: \.self) { weekday in
                            let cell = cells.first { $0.week == week && $0.weekday == weekday }
                            RoundedRectangle(cornerRadius: 3)
                                .fill(fill(cell))
                                .frame(width: Self.cell, height: Self.cell)
                        }
                    }
                }
            }
            Text("Last 12 weeks")
                .font(Typography.caption)
                .foregroundStyle(Theme.secondaryText)
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(StatsFormat.heatmapSummary(cells, calendar: calendar, locale: calendar.locale ?? .current))
    }

    /// A day's shade: none, faint, in your colour stronger with more words; a day not yet come
    /// (after today in this week) is left out.
    private func fill(_ cell: StatsFormat.Cell?) -> Color {
        guard let cell else { return .clear }
        return cell.level == 0 ? PaperPalette.chip : theme.you.opacity([0, 0.3, 0.5, 0.75, 1][cell.level])
    }
}

private struct MeetingsCard: View {
    let counted: StatsCounted

    var body: some View {
        let month = counted.meetingsMonth
        let all = counted.meetingsAll
        StatsCard(title: "Meetings this month") {
            if all.meetings == 0 {
                Line(text: "Hours, talk time and the questions you asked appear here after your first recorded meeting.", secondary: true)
            } else {
                if month.meetings == 0 {
                    Line(text: "No meetings recorded this month yet.")
                } else {
                    NumberRow(numbers: [
                        (LibraryFormat.duration(ms: month.recordedMs), "recorded"),
                        ("\(month.meetings)", month.meetings == 1 ? "meeting" : "meetings"),
                        ("\(month.questions)", month.questions == 1 ? "question you asked" : "questions you asked"),
                    ])
                    TalkTime(you: month.youMs, them: month.themMs)
                    Line(text: "Your longest monologue: \(StatsFormat.span(ms: month.longestMonologueMs))")
                    Line(text: "Questions are your own lines that end in \u{201C}?\u{201D}.", secondary: true)
                }
                Line(
                    text: "All time: \(all.meetings) \(all.meetings == 1 ? "meeting" : "meetings") · \(LibraryFormat.duration(ms: all.recordedMs)) recorded",
                    secondary: true)
            }
        }
    }
}

/// You and them, as a bar in your two colours and in words. Exact: your microphone is you, the
/// call's sound is them.
struct TalkTime: View {
    let you: Int64
    let them: Int64
    @Environment(GlowTheme.self) private var theme

    var body: some View {
        if let share = StatsFormat.talkShare(you: you, them: them) {
            let words = "You \(share.you) % · \(StatsFormat.span(ms: you))   Them \(share.them) % · \(StatsFormat.span(ms: them))"
            VStack(alignment: .leading, spacing: 6) {
                GeometryReader { geometry in
                    HStack(spacing: 2) {
                        Capsule().fill(theme.you)
                            .frame(width: max(0, (geometry.size.width - 2) * CGFloat(share.you) / 100))
                        Capsule().fill(theme.them)
                    }
                }
                .frame(height: 8)
                Text(words)
                    .font(Typography.caption)
                    .foregroundStyle(Theme.secondaryText)
                    .monospacedDigit()
            }
            .accessibilityElement(children: .ignore)
            .accessibilityLabel("Talk time: you \(share.you) percent, \(StatsFormat.span(ms: you)); them \(share.them) percent, \(StatsFormat.span(ms: them))")
        } else {
            Line(text: "No talk time recorded this month.", secondary: true)
        }
    }
}

private struct PromisesCard: View {
    let counted: StatsCounted

    var body: some View {
        let month = counted.promisesMonth
        let all = counted.promisesAll
        StatsCard(title: "Promises") {
            if all.made == 0 {
                Line(text: "Promises from your meetings, kept and open, appear here once a meeting has some.", secondary: true)
            } else {
                if month.made == 0 {
                    Line(text: "No promises made this month.")
                } else {
                    Text(StatsFormat.kept(month, period: "this month"))
                        .font(Typography.heading)
                        .foregroundStyle(Theme.text)
                    Line(text: StatsFormat.promiseDetail(month))
                }
                Line(text: "All time: kept \(all.kept) of \(all.made) · " + StatsFormat.promiseDetail(all), secondary: true)
            }
        }
    }
}

private struct MilestonesCard: View {
    let milestones: [MilestoneRow]
    let locale: Locale

    var body: some View {
        StatsCard(title: "Milestones") {
            FlowRow(spacing: 8) {
                ForEach(milestones, id: \.id) { m in
                    let title = StatsFormat.milestoneTitle(kind: m.kind, threshold: m.threshold, locale: locale)
                    HStack(spacing: 5) {
                        Image(systemName: m.reached ? "checkmark.circle.fill" : "circle")
                            .accessibilityHidden(true)
                        Text(title)
                    }
                    .font(.system(size: Glow.Size.eyebrow, weight: m.reached ? .medium : .regular))
                    .foregroundStyle(m.reached ? Theme.text : Theme.secondaryText)
                    .padding(.horizontal, 10)
                    .padding(.vertical, 4)
                    .background(m.reached ? PaperPalette.chip : Color.clear, in: Capsule())
                    .overlay(Capsule().strokeBorder(PaperPalette.border, lineWidth: m.reached ? 0 : 1))
                    .accessibilityElement(children: .ignore)
                    .accessibilityLabel("\(title), \(m.reached ? "reached" : "not yet")")
                }
            }
        }
    }
}

/// Chips left to right, wrapping onto new rows.
struct FlowRow: Layout {
    var spacing: CGFloat = 8

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let rows = arrange(subviews, width: proposal.width ?? .infinity)
        let width = rows.map { $0.width }.max() ?? 0
        let height = rows.map(\.height).reduce(0, +) + spacing * CGFloat(max(rows.count - 1, 0))
        return CGSize(width: width, height: height)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        var y = bounds.minY
        for row in arrange(subviews, width: bounds.width) {
            var x = bounds.minX
            for index in row.items {
                let size = subviews[index].sizeThatFits(.unspecified)
                subviews[index].place(at: CGPoint(x: x, y: y), proposal: ProposedViewSize(size))
                x += size.width + spacing
            }
            y += row.height + spacing
        }
    }

    private struct Row {
        var items: [Int] = []
        var width: CGFloat = 0
        var height: CGFloat = 0
    }

    private func arrange(_ subviews: Subviews, width: CGFloat) -> [Row] {
        var rows = [Row()]
        for index in subviews.indices {
            let size = subviews[index].sizeThatFits(.unspecified)
            let extra = rows[rows.count - 1].items.isEmpty ? size.width : spacing + size.width
            if rows[rows.count - 1].width + extra > width, !rows[rows.count - 1].items.isEmpty {
                rows.append(Row())
            }
            let added = rows[rows.count - 1].items.isEmpty ? size.width : spacing + size.width
            rows[rows.count - 1].items.append(index)
            rows[rows.count - 1].width += added
            rows[rows.count - 1].height = max(rows[rows.count - 1].height, size.height)
        }
        return rows.filter { !$0.items.isEmpty }
    }
}
