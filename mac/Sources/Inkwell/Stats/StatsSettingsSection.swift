// Settings > Stats: whether milestones and bests are celebrated, the typing speed time saved is
// measured against, and the gentle streak: whether it shows, the days it rests on, and a pause.
// All live in the core's store (stats.celebrate, stats.typing_wpm, stats.streak, stats.rest_days,
// the pauses), read at launch; the pause's state comes with the numbers, counted when Settings
// first shows. Rest days and a pause never make a streak shorter: the core counts neither as
// missed.
import SwiftUI

struct StatsSettingsSection: View {
    let stats: StatsModel

    static let celebrateDetail = "A quiet glow on the orb and one line, once for each milestone: 1,000 to 100,000 words, and streaks of 7, 30 and 100 active days. With Always still or Reduce Motion, only the line. A personal best gets a short note in the Drop."
    static let typingDetail = "Time saved is typing the same words at this speed, less the time spent speaking."
    static let streakDetail = "Off, no streak shows on Stats or the share card, and its milestones aren't celebrated. It's still counted, so turning it back on loses nothing."
    static let restDaysDetail = "Days you take off. They neither count toward a streak nor break it, even if you dictate on one."
    static let pauseDetail = "For a holiday or time off: days without a dictation don't count against the streak, for up to 90 days."

    /// What the pause row says before the numbers are in: counting, or that they couldn't be.
    static func pauseUnknown(failed: Bool) -> String {
        failed ? "Couldn't read whether the streak is paused." : "Counting\u{2026}"
    }

    /// What the pause row says: running since a day, or what a pause does.
    static func pauseCaption(pausedSince: String?, calendar: Calendar) -> String {
        guard let since = pausedSince else { return pauseDetail }
        let day = StatsFormat.day(since, calendar: calendar).map { StatsFormat.shortDate($0, calendar: calendar) } ?? since
        return "Paused since \(day). It ends by itself after 90 days, or when you resume."
    }

    /// What a failed pause or resume says.
    static func streakFailure(_ change: StatsModel.StreakChange) -> String {
        change == .pausing ? "Couldn't pause the streak. Try again." : "Couldn't resume the streak. Try again."
    }

    var body: some View {
        let paused = stats.counted?.dictation.streakPausedSince
        VStack(alignment: .leading, spacing: 10) {
            SectionTitle(text: "Stats")
            row("Celebrate milestones", detail: Self.celebrateDetail) {
                // A closure literal, not a method reference: see MeetingsSection's toggle.
                Toggle("Celebrate milestones", isOn: Binding(get: { stats.celebrate }, set: { stats.setCelebrate($0) }))
                    .toggleStyle(.switch)
                    .labelsHidden()
            }
            row("Typing speed", detail: Self.typingDetail) {
                Stepper(value: Binding(get: { stats.typingWpm }, set: { stats.setTypingWpm($0) }),
                        in: StatsModel.typingWpmRange, step: 5) {
                    Text("\(stats.typingWpm) wpm").monospacedDigit()
                }
                .accessibilityLabel("Typing speed")
                .accessibilityValue("\(stats.typingWpm) words per minute")
            }
            row("Show the streak", detail: Self.streakDetail) {
                Toggle("Show the streak", isOn: Binding(get: { stats.streakShown }, set: { stats.setStreakShown($0) }))
                    .toggleStyle(.switch)
                    .labelsHidden()
            }
            row("Rest days", detail: Self.restDaysDetail) {
                RestDayPicker(stats: stats)
            }
            // Unknown until counted: never a Pause offered over a pause that may be running.
            let known = stats.counted != nil
            row("Pause the streak", detail: known
                ? Self.pauseCaption(pausedSince: paused, calendar: stats.calendar)
                : Self.pauseUnknown(failed: stats.loadState == .failed)) {
                VStack(alignment: .leading, spacing: 4) {
                    Button(paused == nil ? "Pause" : "Resume") {
                        if paused == nil { stats.pauseStreak() } else { stats.resumeStreak() }
                    }
                    .disabled(!known || stats.streakChanging != nil)
                    .accessibilityLabel(paused == nil ? "Pause the streak" : "Resume the streak")
                    if let failed = stats.streakChangeFailed {
                        Text(Self.streakFailure(failed))
                            .font(Typography.caption)
                            .foregroundStyle(Theme.alert)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
            }
            if stats.settingsFailed {
                Text("Couldn't read or save a Stats setting. It may not be what it shows.")
                    .font(Typography.caption)
                    .foregroundStyle(Theme.alert)
            }
            SettingColumns {
                Text("Where").font(.system(.body, weight: .semibold))
            } controls: {
                Text("Counted on this Mac from your library. Nothing is sent, and nothing is compared with anyone.")
                    .font(Typography.body)
                    .foregroundStyle(Theme.secondaryText)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .accessibilityElement(children: .combine)
        }
        .onAppear { stats.settingsAppeared() }
        // A pause or resume that failed is read aloud: the line appears away from the focus.
        .onChange(of: stats.streakChangeFailed) { _, failed in
            if let failed { AccessibilityNotification.Announcement(Self.streakFailure(failed)).post() }
        }
        // A pause that is never answered is said, and the button works again: one wait per change.
        .task(id: stats.pendingStreak) {
            guard let ref = stats.pendingStreak else { return }
            do {
                try await Task.sleep(for: StatsModel.loadLimit)
            } catch {
                return
            }
            stats.streakTimedOut(ref)
        }
    }

    private func row(_ title: String, detail: String, @ViewBuilder control: () -> some View) -> some View {
        SettingColumns {
            Text(title)
                .accessibilityHidden(true)
        } controls: {
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

/// A toggle drawn as a pill, as PaperButtonStyle's buttons: filled in the button colour while on,
/// the light wash while off. The state is in the fill, not only a tint, so it reads in both modes.
struct ChipToggleStyle: ToggleStyle {
    func makeBody(configuration: Configuration) -> some View {
        ChipToggle(configuration: configuration)
    }

    private struct ChipToggle: View {
        let configuration: ToggleStyleConfiguration
        @Environment(\.isEnabled) private var enabled

        var body: some View {
            Button {
                configuration.isOn.toggle()
            } label: {
                configuration.label
                    .font(.system(size: 13, weight: configuration.isOn ? .semibold : .regular))
                    .padding(.horizontal, 12)
                    .frame(minHeight: 26)
                    .foregroundStyle(configuration.isOn ? Theme.buttonLabel : Theme.text)
                    .background(Capsule().fill(configuration.isOn ? Theme.buttonFill : PaperPalette.chip))
                    .contentShape(Capsule())
                    .opacity(enabled ? 1 : 0.45)
            }
            .buttonStyle(.plain)
            .accessibilityAddTraits(.isToggle)
            .accessibilityValue(configuration.isOn ? "On" : "Off")
        }
    }
}

/// The seven weekdays as buttons that stay pressed, in the order the user's week runs. The last
/// day not resting can't be made one: a streak needs a day to count.
struct RestDayPicker: View {
    let stats: StatsModel

    var body: some View {
        let days = StatsFormat.weekdays(calendar: stats.calendar)
        FlowRow(spacing: 6) {
            ForEach(days, id: \.iso) { day in
                let on = stats.restDays.contains(day.iso)
                let last = !on && stats.restDays.count == 6
                Toggle(day.short, isOn: Binding(get: { on }, set: { stats.setRestDay(day.iso, $0) }))
                    .toggleStyle(ChipToggleStyle())
                    .disabled(last)
                    .accessibilityLabel(day.name)
                    .help(last ? "At least one day counts toward the streak" : day.name)
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Rest days")
    }
}
