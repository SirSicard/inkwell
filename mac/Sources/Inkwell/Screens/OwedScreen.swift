// Owed: the promises still open, grouped, overdue ones in the alert colour, a promise said twice
// shown once. Marking one done takes it off the list.
import InkBridge
import SwiftUI

struct OwedScreen: View {
    @Environment(ScreenModels.self) private var screens

    var body: some View {
        let owed = screens.owed
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                Paper.Header(title: "Owed", subtitle: owed.loaded ? owed.summary(now: Date()) : nil)
                if let failure = owed.failure {
                    Text(failure)
                        .font(Typography.caption)
                        .foregroundStyle(Theme.alert)
                }
                ForEach(owed.suggestions) { suggestion in
                    LooksDoneCard(suggestion: suggestion, owed: owed)
                }
                if !owed.loaded {
                    ProgressView().controlSize(.small)
                        .accessibilityLabel("Loading what you owe")
                } else if owed.items.isEmpty {
                    Text("Nothing owed. Promises you make in meetings show up here.")
                        .font(Typography.body)
                        .foregroundStyle(Theme.secondaryText)
                } else {
                    ForEach(owed.groups(now: Date())) { group in
                        OwedGroupView(group: group, owed: owed)
                    }
                }
            }
            .frame(maxWidth: 860, alignment: .leading)
            .padding(.horizontal, 48)
            .padding(.top, 34)
            .padding(.bottom, 28)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .onAppear { owed.load() }
    }
}

private struct LooksDoneCard: View {
    let suggestion: LooksDone
    let owed: OwedModel

    var body: some View {
        HStack(spacing: 14) {
            Image(systemName: "checkmark.circle")
                .font(.title2)
                .foregroundStyle(Theme.accent)
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 3) {
                // What was said, verbatim: never read as markdown.
                Text(verbatim: "Looks done: " + suggestion.text)
                    .font(Typography.body)
                    .foregroundStyle(Theme.text)
                Text(suggestion.source)
                    .font(Typography.timestamp)
                    .foregroundStyle(Theme.secondaryText)
            }
            Spacer(minLength: 0)
            Button("Mark done") { owed.markDone(suggestion.commitment) }
                .buttonStyle(.borderedProminent)
            Button("Not yet") { owed.notYet(suggestion) }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 14)
        .paperCard()
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Looks done")
    }
}

private struct OwedGroupView: View {
    let group: OwedGroup
    let owed: OwedModel

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(alignment: .firstTextBaseline, spacing: 10) {
                Text(group.title)
                    .font(.system(.headline))
                    .foregroundStyle(Theme.text)
                    .accessibilityAddTraits(.isHeader)
                if let subtitle = group.subtitle {
                    Text(subtitle)
                        .font(Typography.timestamp)
                        .foregroundStyle(Theme.secondaryText)
                }
            }
            .padding(.bottom, 8)
            Rectangle().fill(PaperPalette.border).frame(height: 1).accessibilityHidden(true)
            ForEach(group.rows) { row in
                OwedRowView(row: row, owed: owed)
                if row.id != group.rows.last?.id {
                    Rectangle().fill(PaperPalette.separator).frame(height: 1).accessibilityHidden(true)
                }
            }
        }
    }
}

private struct OwedRowView: View {
    let row: OwedRow
    let owed: OwedModel
    @Environment(\.calendar) private var calendar

    var body: some View {
        HStack(alignment: .top, spacing: 14) {
            Button {
                owed.markDone(row.id)
            } label: {
                Circle()
                    .strokeBorder(row.due.isOverdue ? Theme.alert : Theme.text, lineWidth: 1.5)
                    .frame(width: 20, height: 20)
                    .contentShape(Circle())
            }
            .buttonStyle(.plain)
            .padding(.top, 1)
            .accessibilityLabel("Mark done: \(row.text)")
            VStack(alignment: .leading, spacing: 6) {
                Text(row.text)
                    .font(.system(size: 15))
                    .foregroundStyle(Theme.text)
                    .fixedSize(horizontal: false, vertical: true)
                HStack(spacing: 8) {
                    Paper.Chip(
                        text: row.due.text(calendar: calendar),
                        tone: row.due.isOverdue ? .alert : (row.due == .undated ? .neutral : .due))
                    if let merged = row.mergedText {
                        Paper.Chip(text: merged)
                    }
                    if let at = row.saidAtMs {
                        Text([row.meeting, "at \(liveClock(ms: at))"].compactMap { $0 }.joined(separator: " "))
                            .font(Typography.timestamp)
                            .foregroundStyle(Theme.secondaryText)
                    }
                }
            }
        }
        .padding(.vertical, 11)
        .accessibilityElement(children: .contain)
    }
}
