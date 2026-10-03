// A milestone reached: a short, quiet glow over the orb in your two colours, and one line at the
// foot of the window, once. Only while the window is on screen (nothing moves unseen): a milestone
// reached while the user dictates in another app waits for them. Under "Always still" or Reduce
// Motion there is no glow, only the line. VoiceOver hears the line.
import InkRenderer
import SwiftUI

enum MilestoneCelebration {
    /// How long the line shows.
    static let shown: Duration = .seconds(6)
    /// The glow: in, held, out. About two and a half seconds, once.
    static let glowIn = 0.8
    static let glowHeld: Duration = .milliseconds(600)
    static let glowOut = 1.2
    /// The glow's brightest, over the orb: quiet, under the screens' text.
    static let glowPeak = 0.45

    /// The celebration to show now: the pending one, while the window is on screen.
    static func showing(_ pending: StatsModel.Celebration?, onScreen: Bool) -> StatsModel.Celebration? {
        onScreen ? pending : nil
    }

    /// Whether the glow moves at all: never under "Always still" or Reduce Motion.
    static func glows(still: Bool, reduceMotion: Bool) -> Bool {
        !still && !reduceMotion
    }
}

/// The glow, centred on the orb (its placement's centre and unit), in your colour fading to theirs.
/// Draws nothing until a celebration starts it, and nothing after.
struct MilestoneGlow: View {
    /// The celebration showing, by serial; nil for none.
    let serial: Int?
    let you: Color
    let them: Color
    let placement: OrbPlacement
    let glows: Bool
    @State private var lit = false

    var body: some View {
        GeometryReader { geometry in
            let unit = min(geometry.size.width, geometry.size.height) * placement.unit
            Circle()
                .fill(RadialGradient(
                    colors: [you.opacity(0.9), them.opacity(0.5), .clear], center: .center,
                    startRadius: 0, endRadius: unit * 0.55))
                .frame(width: unit * 1.1, height: unit * 1.1)
                .position(x: geometry.size.width * placement.x, y: geometry.size.height * placement.yFromTop)
                .opacity(lit ? MilestoneCelebration.glowPeak : 0)
        }
        .allowsHitTesting(false)
        .accessibilityHidden(true)
        .task(id: serial) {
            guard serial != nil, glows else { return }
            withAnimation(.easeOut(duration: MilestoneCelebration.glowIn)) { lit = true }
            // Cancelled (the window left the screen): out at once, never left lit.
            let held = (try? await Task.sleep(for: .seconds(MilestoneCelebration.glowIn) + MilestoneCelebration.glowHeld)) != nil
            withAnimation(held ? .easeIn(duration: MilestoneCelebration.glowOut) : nil) { lit = false }
        }
    }
}

/// The milestone's one line, for a few seconds, with a way to dismiss it at once.
struct MilestoneNote: View {
    let celebration: StatsModel.Celebration
    let stats: StatsModel

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: "sparkle")
                .foregroundStyle(Theme.text)
                .accessibilityHidden(true)
            Text(celebration.note)
                .font(Typography.body)
                .foregroundStyle(Theme.text)
                .lineLimit(1)
            Button {
                stats.dismissCelebration(celebration.serial)
            } label: {
                Image(systemName: "xmark")
            }
            .buttonStyle(.plain)
            .foregroundStyle(Theme.secondaryText)
            .accessibilityLabel("Dismiss")
        }
        .padding(.horizontal, 18)
        .padding(.vertical, 10)
        .frame(maxWidth: 520)
        .paperCard()
        .accessibilityElement(children: .contain)
        .task(id: celebration.serial) {
            AccessibilityNotification.Announcement(celebration.note).post()
            // Cancelled (the window left the screen before its time): it stays pending, and shows
            // again when the window is back.
            do {
                try await Task.sleep(for: MilestoneCelebration.shown)
            } catch {
                return
            }
            stats.dismissCelebration(celebration.serial)
        }
    }
}
