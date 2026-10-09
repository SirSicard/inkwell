// Stats: last week's review until it is dismissed, the user's dictation (words, speed against
// their own past, time saved with its assumption and what it is about, the streak and a heatmap),
// their meetings (hours, talk time by side, the longest monologue, their questions), promises
// kept, their records, and milestones. Translucent cards over the orb, in Glow's type; every
// number has a VoiceOver label, and the heatmap one sentence.
//
// A new library says plainly what will appear. A count that could not be made says so, never
// zero.
import InkBridge
import SwiftUI

struct StatsScreen: View {
    @Environment(ScreenModels.self) private var screens
    @Environment(WindowPresence.self) private var presence
    @State private var sharing = false

    var body: some View {
        let stats = screens.stats
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                Paper.Header(title: "Stats", subtitle: "Counted on this Mac from your library. Nothing leaves it.") {
                    if stats.counted != nil {
                        Button("Share card\u{2026}") { sharing = true }
                            .buttonStyle(PaperButtonStyle())
                            .accessibilityHint("Makes an image of the numbers you pick, to copy or save")
                    }
                }
                if let counted = stats.counted {
                    if stats.loadState == .failed {
                        Text("Couldn't count again. These are the numbers from before.")
                            .font(Typography.caption)
                            .foregroundStyle(Theme.alert)
                    }
                    StatsCards(
                        counted: counted, calendar: stats.calendar, review: stats.weekReview,
                        reviewFailed: stats.reviewDismissFailed, dismissReview: { stats.dismissReview($0) })
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
        // Off screen it waits; back on screen it counts again.
        .onChange(of: presence.onScreen, initial: true) { _, onScreen in stats.windowPresence(onScreen: onScreen) }
        // An answer that never comes is said, not spun for ever: one wait per question.
        .task(id: stats.pendingLoad) {
            guard let ref = stats.pendingLoad else { return }
            do {
                try await Task.sleep(for: StatsModel.loadLimit)
            } catch {
                return
            }
            stats.loadTimedOut(ref)
        }
        .sheet(isPresented: $sharing) {
            if let counted = stats.counted {
                // The sheet follows the app's appearance, as the first run's does.
                ShareCardSheet(counted: counted, stats: stats).followsAppMode()
            }
        }
    }
}

/// The cards, one under the other: last week first while its review shows.
struct StatsCards: View {
    let counted: StatsCounted
    let calendar: Calendar
    /// Last week's review, until it is dismissed (StatsModel.weekReview).
    var review: WeekReview?
    /// Its dismissal could not be saved.
    var reviewFailed = false
    var dismissReview: (WeekReview) -> Void = { _ in }

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            if let review {
                WeekReviewCard(review: review, calendar: calendar, failed: reviewFailed) { dismissReview(review) }
            }
            DictationCard(counted: counted, calendar: calendar)
            MeetingsCard(counted: counted)
            PromisesCard(counted: counted)
            RecordsCard(records: StatsFormat.records(counted.bests, calendar: calendar))
            MilestonesCard(milestones: counted.milestones, locale: calendar.locale ?? .current)
        }
    }
}

/// Last week, reviewed: gains and plain facts only, until the user dismisses it. It never goes by
/// itself.
struct WeekReviewCard: View {
    let review: WeekReview
    let calendar: Calendar
    let failed: Bool
    let dismiss: () -> Void

    var body: some View {
        let locale = calendar.locale ?? .current
        VStack(alignment: .leading, spacing: 12) {
            HStack(alignment: .firstTextBaseline, spacing: 12) {
                VStack(alignment: .leading, spacing: 2) {
                    Paper.Eyebrow(text: "Last week")
                    Text(StatsFormat.reviewWeek(review, calendar: calendar))
                        .font(Typography.caption)
                        .foregroundStyle(Theme.secondaryText)
                }
                Spacer(minLength: 0)
                Button("Dismiss", action: dismiss)
                    .buttonStyle(PaperButtonStyle())
                    .accessibilityLabel("Dismiss last week's review")
                    .accessibilityHint("It doesn't come back for this week")
            }
            NumberRow(numbers: StatsFormat.reviewNumbers(review, locale: locale))
            ForEach(StatsFormat.reviewLines(review, calendar: calendar), id: \.self) { Line(text: $0) }
            if failed {
                Line(text: Self.failedText, alert: true)
            }
        }
        .padding(.horizontal, 22)
        .padding(.vertical, 18)
        .frame(maxWidth: .infinity, alignment: .leading)
        .paperCard()
        .accessibilityElement(children: .contain)
        // Read aloud when the dismissal could not be saved: the card the user dismissed is back.
        .onChange(of: failed, initial: true) { _, failed in
            if failed { AccessibilityNotification.Announcement(Self.failedText).post() }
        }
    }

    static let failedText = "Couldn't dismiss it. It stays until you try again."
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
    /// What VoiceOver hears, when not the value and the label.
    var spoken: String?

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
        .accessibilityLabel(spoken ?? "\(value) \(label)")
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
    var alert = false

    var body: some View {
        Text(text)
            .font(secondary || alert ? Typography.caption : Typography.body)
            .foregroundStyle(alert ? Theme.alert : secondary ? Theme.secondaryText : Theme.text)
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
                if d.savedMsAll > 0, let about = StatsFormat.savedAbout(d.savedAboutAll) {
                    Line(text: about, secondary: true)
                }
                if d.savedMsWeek > 0 {
                    Line(text: StatsFormat.savedThisWeek(ms: d.savedMsWeek, about: d.savedAboutWeek), secondary: true)
                }
                if let streak = StatsFormat.streak(d) {
                    Line(text: streak)
                    if let month = StatsFormat.activeDaysThisMonth(d) {
                        Line(text: month)
                    }
                }
                // A pause shows before there is a streak to carry, as long as the streak shows.
                if d.streakHidden != true, let since = d.streakPausedSince {
                    Line(text: StatsFormat.paused(since: since, calendar: calendar), secondary: true)
                }
                if StatsFormat.streak(d) != nil {
                    Line(text: StatsFormat.streakRule(restDays: d.restDays ?? [], calendar: calendar), secondary: true)
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
                            let index = week * 7 + weekday
                            let cell = index < cells.count ? cells[index] : nil
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
                    Line(text: "Questions are your own lines that end in a question mark.", secondary: true)
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
            VStack(alignment: .leading, spacing: 6) {
                GeometryReader { geometry in
                    HStack(spacing: 2) {
                        Capsule().fill(theme.you)
                            .frame(width: max(0, (geometry.size.width - 2) * CGFloat(share.you) / 100))
                        Capsule().fill(theme.them)
                    }
                }
                .frame(height: 8)
                // The key: a dot in each colour before its words (no words in the dot colours).
                HStack(spacing: 14) {
                    key(theme.you, "You \(share.you) % · \(StatsFormat.span(ms: you))")
                    key(theme.them, "Them \(share.them) % · \(StatsFormat.span(ms: them))")
                }
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

    private func key(_ colour: Color, _ words: String) -> some View {
        HStack(spacing: 5) {
            Circle().fill(colour).frame(width: 8, height: 8)
            Text(words)
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
                    let title = StatsFormat.milestoneChip(m, locale: locale)
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

/// The user's personal bests, each with when it was set: from what was dictated and recorded
/// here, never an import's. A best not held yet is left out, never shown as zero.
private struct RecordsCard: View {
    let records: [StatsFormat.Record]

    /// Each record's least width, so two or three sit in even columns side by side.
    static let tileWidth: CGFloat = 150

    var body: some View {
        StatsCard(title: "Records") {
            if records.isEmpty {
                Line(text: StatsFormat.recordsEmpty, secondary: true)
            } else {
                // Even columns, as many as fit; a label too long for its column wraps in it.
                EvenColumns(minimum: Self.tileWidth, spacing: 24, rowSpacing: 14) {
                    ForEach(records) { record in
                        BigNumber(value: record.value, label: record.label, spoken: record.spoken)
                    }
                }
                Line(text: StatsFormat.recordsRule, secondary: true)
            }
        }
    }
}

/// Items in even columns, as many as fit at `minimum` wide each, left to right in rows; each item
/// as wide as its column (its text wraps there), each row as tall as its tallest.
struct EvenColumns: Layout {
    var minimum: CGFloat
    var spacing: CGFloat
    var rowSpacing: CGFloat

    private func columns(_ width: CGFloat?, count: Int) -> (count: Int, width: CGFloat) {
        Self.columns(width, minimum: minimum, spacing: spacing, count: count)
    }

    /// How many columns of at least `minimum` fit `width` (at most `count`, at least one), and
    /// their width.
    static func columns(_ width: CGFloat?, minimum: CGFloat, spacing: CGFloat, count: Int) -> (count: Int, width: CGFloat) {
        guard let width, width.isFinite else { return (max(1, min(count, 2)), minimum) }
        let fit = max(1, Int((width + spacing) / (minimum + spacing)))
        let n = max(1, min(fit, count))
        return (n, (width - spacing * CGFloat(n - 1)) / CGFloat(n))
    }

    private func rows(_ subviews: Subviews, _ column: CGFloat, _ n: Int) -> [CGFloat] {
        stride(from: 0, to: subviews.count, by: n).map { start in
            subviews[start..<min(start + n, subviews.count)]
                .map { $0.sizeThatFits(ProposedViewSize(width: column, height: nil)).height }.max() ?? 0
        }
    }

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let (n, column) = columns(proposal.width, count: subviews.count)
        let heights = rows(subviews, column, n)
        let width = proposal.width.flatMap { $0.isFinite ? $0 : nil } ?? (column * CGFloat(n) + spacing * CGFloat(n - 1))
        return CGSize(width: width, height: heights.reduce(0, +) + rowSpacing * CGFloat(max(heights.count - 1, 0)))
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        let (n, column) = columns(bounds.width, count: subviews.count)
        let heights = rows(subviews, column, n)
        var y = bounds.minY
        for (row, height) in heights.enumerated() {
            for i in 0..<n where row * n + i < subviews.count {
                subviews[row * n + i].place(
                    at: CGPoint(x: bounds.minX + CGFloat(i) * (column + spacing), y: y),
                    proposal: ProposedViewSize(width: column, height: nil))
            }
            y += height + rowSpacing
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
