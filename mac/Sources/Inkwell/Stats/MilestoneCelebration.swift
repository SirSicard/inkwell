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

/// The glow, centred on the orb where it is (it wanders: OrbHold holds it there until the glow has
/// gone) at the orb's unit, in your colour fading to theirs. In the window only while a
/// celebration shows and motion is allowed; it plays once per celebration (StatsModel.beginGlow),
/// even if the window leaves the screen and comes back.
struct MilestoneGlow: View {
    let serial: Int
    let you: Color
    let them: Color
    /// The orb's home and unit; its home is the centre only with no orb to hold.
    let placement: OrbPlacement
    let orb: OrbHold?
    let stats: StatsModel
    /// The celebration lit now: a replaced one's task never dims its successor.
    @State private var litSerial: Int?
    /// Where the orb rests, held, under the glow, as fractions of the window.
    @State private var centre: SIMD2<Double>?

    /// Holds the orb (release it when the glow has gone) and returns the glow's centre once the orb
    /// has arrived: its spot, or its home with no orb.
    static func holdCentre(_ orb: OrbHold?, home: OrbPlacement) async -> SIMD2<Double> {
        await orb?.hold() ?? SIMD2(home.x, home.yFromTop)
    }

    var body: some View {
        GeometryReader { geometry in
            let unit = min(geometry.size.width, geometry.size.height) * placement.unit
            Circle()
                .fill(RadialGradient(
                    colors: [you.opacity(0.9), them.opacity(0.5), .clear], center: .center,
                    startRadius: 0, endRadius: unit * 0.55))
                .frame(width: unit * 1.1, height: unit * 1.1)
                .position(
                    x: geometry.size.width * (centre?.x ?? placement.x),
                    y: geometry.size.height * (centre?.y ?? placement.yFromTop))
                // Placed at once: only the opacity eases in, never a slide from the home.
                .animation(nil, value: centre)
                .opacity(litSerial == serial ? MilestoneCelebration.glowPeak : 0)
        }
        .allowsHitTesting(false)
        .accessibilityHidden(true)
        .task(id: serial) {
            let mine = serial
            // The orb stays put under the glow until it has gone (or is cancelled); it lights once
            // the orb has arrived, so a glide on coming on screen finishes first, or where the orb
            // is when it has not arrived within OrbHold.arrivalLimit.
            defer { orb?.release() }
            let spot = await Self.holdCentre(orb, home: placement)
            guard !Task.isCancelled, stats.beginGlow(mine) else { return }
            centre = spot
            withAnimation(.easeOut(duration: MilestoneCelebration.glowIn)) { litSerial = mine }
            // Cancelled (the window left the screen, the line was dismissed): out at once.
            let held = (try? await Task.sleep(for: .seconds(MilestoneCelebration.glowIn) + MilestoneCelebration.glowHeld)) != nil
            withAnimation(held ? .easeIn(duration: MilestoneCelebration.glowOut) : nil) {
                if litSerial == mine { litSerial = nil }
            }
            // Holds the orb through the fade; cancelled, it lets go at once.
            if held { try? await Task.sleep(for: .seconds(MilestoneCelebration.glowOut)) }
        }
    }
}

/// The milestone's one line, for a few seconds, with a way to dismiss it at once. VoiceOver hears
/// it once (StatsModel.beginAnnouncement).
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
                    .frame(width: 24, height: 24)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .foregroundStyle(Theme.secondaryText)
            .accessibilityLabel("Dismiss")
        }
        .padding(.leading, 18)
        .padding(.trailing, 10)
        .padding(.vertical, 8)
        .frame(maxWidth: 520)
        .paperCard()
        .accessibilityElement(children: .contain)
        .task(id: celebration.serial) {
            if stats.beginAnnouncement(celebration.serial) {
                AccessibilityNotification.Announcement(celebration.note).post()
            }
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
