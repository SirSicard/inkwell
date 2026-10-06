// Settings > Sound: the microphone picker and the mic test (SoundModel). macOS has no output
// picker: the far end is tapped from its app, wherever it plays.
import InkBridge
import SwiftUI

struct SoundSection: View {
    let sound: SoundModel
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            SectionTitle(text: "Sound")
            SettingRow(title: "Microphone") {
                if sound.devices == nil {
                    Text(sound.caption)
                        .font(Typography.caption)
                        .foregroundStyle(Theme.secondaryText)
                } else {
                    Picker("Microphone", selection: Binding(
                        get: { sound.devices?.input ?? "auto" }, set: { sound.choose($0) })) {
                        ForEach(sound.choices) { choice in
                            Text(choice.title).tag(choice.id)
                        }
                    }
                    .labelsHidden()
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(maxWidth: 420, alignment: .leading)
                    .accessibilityLabel("Microphone")
                    .accessibilityHint(sound.caption)
                    Text(sound.caption)
                        .font(Typography.caption)
                        .foregroundStyle(Theme.secondaryText)
                        .fixedSize(horizontal: false, vertical: true)
                        .accessibilityHidden(true)
                }
                if let problem = sound.problem {
                    Text(problem)
                        .font(Typography.caption)
                        .foregroundStyle(Theme.alert)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            SettingRow(title: "Test") {
                HStack(spacing: 12) {
                    Button(sound.isTesting ? "Stop" : "Test") { sound.toggleTest() }
                        .accessibilityLabel(sound.isTesting ? "Stop the microphone test" : "Test the microphone")
                        .disabled(sound.devices?.inputs.isEmpty ?? false)
                    LevelMeter(level: sound.level, live: sound.isTesting, animated: !reduceMotion)
                        .frame(maxWidth: 240)
                }
                Text(sound.testLine)
                    .font(Typography.caption)
                    .foregroundStyle(sound.testLineIsProblem ? Theme.alert : Theme.secondaryText)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        // A chosen mic that left is said, not only shown.
        .onChange(of: sound.missingLine) { _, line in
            if let line {
                AccessibilityNotification.Announcement(line).post()
            }
        }
    }
}

/// The test's level: a bar that fills from the left, 0 at -60 dBFS, full at full scale. It moves
/// only as the core reports levels (about ten times a second, only while a test runs); at rest it
/// is drawn once and left alone.
struct LevelMeter: View {
    let level: Double
    /// A test is running: the bar is filled, and VoiceOver reads the level.
    let live: Bool
    /// Each new level eases in over the tenth of a second it stands for (off under Reduce Motion).
    var animated = true

    static let height: CGFloat = 8

    var body: some View {
        GeometryReader { proxy in
            ZStack(alignment: .leading) {
                Capsule().fill(PaperPalette.separator)
                Capsule()
                    .fill(Theme.text)
                    .frame(width: proxy.size.width * CGFloat(live ? level : 0))
            }
        }
        .frame(height: Self.height)
        .animation(animated && live ? .linear(duration: 0.1) : nil, value: level)
        .accessibilityElement()
        .accessibilityLabel("Microphone level")
        .accessibilityValue(live ? "\(Int((level * 100).rounded())) percent" : "No test running")
    }
}
