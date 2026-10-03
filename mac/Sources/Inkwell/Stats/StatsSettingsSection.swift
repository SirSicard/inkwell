// Settings > Stats: whether milestones are celebrated, and the typing speed time saved is measured
// against. Both live in the core's store (stats.celebrate, stats.typing_wpm), read at launch.
import SwiftUI

struct StatsSettingsSection: View {
    let stats: StatsModel

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            SectionTitle(text: "Stats")
            row("Celebrate milestones", detail: "A quiet glow on the orb and one line, once for each milestone: 1,000 to 100,000 words, and streaks of 7, 30 and 100 active days. With Always still or Reduce Motion, only the line.") {
                // A closure literal, not a method reference: see MeetingsSection's toggle.
                Toggle("Celebrate milestones", isOn: Binding(get: { stats.celebrate }, set: { stats.setCelebrate($0) }))
                    .toggleStyle(.switch)
                    .labelsHidden()
            }
            row("Typing speed", detail: "Time saved is typing the same words at this speed, less the time spent speaking.") {
                Stepper(value: Binding(get: { stats.typingWpm }, set: { stats.setTypingWpm($0) }),
                        in: StatsModel.typingWpmRange, step: 5) {
                    Text("\(stats.typingWpm) wpm").monospacedDigit()
                }
                .accessibilityLabel("Typing speed")
                .accessibilityValue("\(stats.typingWpm) words per minute")
            }
            if stats.settingsFailed {
                Text("Couldn't read or save a Stats setting. It may not be what it shows.")
                    .font(Typography.caption)
                    .foregroundStyle(Theme.alert)
            }
            HStack(alignment: .firstTextBaseline, spacing: 12) {
                Text("Where").font(.system(.body, weight: .semibold)).frame(width: 150, alignment: .leading)
                Text("Counted on this Mac from your library. Nothing is sent, and nothing is compared with anyone.")
                    .font(Typography.body)
                    .foregroundStyle(Theme.secondaryText)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .accessibilityElement(children: .combine)
        }
    }

    private func row(_ title: String, detail: String, @ViewBuilder control: () -> some View) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 12) {
            Text(title).frame(width: 150, alignment: .leading)
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 4) {
                control()
                    .accessibilityHint(detail)
                Text(detail)
                    .font(Typography.caption)
                    .foregroundStyle(Theme.secondaryText)
                    .fixedSize(horizontal: false, vertical: true)
                    .accessibilityHidden(true)
            }
        }
        .font(Typography.body)
    }
}
