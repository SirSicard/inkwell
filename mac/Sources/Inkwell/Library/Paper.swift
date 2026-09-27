// The reading screens' surfaces and marks, derived from the canvas's six colours (DesignTokens):
// the canvas draws cards, hairlines and side panels as tints of paper, and speakers as ink (you)
// and sepia (them). Each night value keeps the same role on night paper, and every text colour
// here passes 4.5:1 on the surface it sits on.
import AppKit
import SwiftUI

enum Paper {
    /// A raised card (the needs-you banner, a selected row): paper lifted toward white.
    static let card = dynamic(light: 0xFBF9F4, dark: 0x1B1E25)
    /// Hairlines between regions and around cards.
    static let hairline = dynamic(light: 0xD9D1C3, dark: 0x2C3039)
    /// A side panel (the library's list, the player bar): paper a shade down.
    static let panel = dynamic(light: 0xEFEAE1, dark: 0x171A20)
    /// The current line of the transcript, and a pressed control.
    static let highlight = dynamic(light: 0xE9E3D8, dark: 0x232731)
    /// A card that needs the user: its border, a tint of the seal.
    static let sealTint = dynamic(light: 0xE1C3B7, dark: 0x5C2D24)
    /// Text a step below the body (the canvas's #43454E): 9.2:1 on paper.
    static let quiet = dynamic(light: 0x43454E, dark: 0xC9C3B8)
    /// Your words and your lane: ink, or paper on night paper.
    static let you = dynamic(light: 0x16181F, dark: 0xF2EEE6)
    /// Their words and their lane: sepia. On night paper, a lighter sepia, so it reads as text.
    static let them = dynamic(light: 0x7E5431, dark: 0xC99A6E)
    /// Text that needs the user (overdue, a failure): the seal, or a lighter seal on night paper
    /// (the seal itself is 3.3:1 there).
    static let alert = dynamic(light: 0xB23A26, dark: 0xE8806C)
    /// A timestamp chip's border.
    static let chip = dynamic(light: 0xC9AE95, dark: 0x6B5039)

    private static func dynamic(light: UInt32, dark: UInt32) -> Color {
        Color(nsColor: NSColor(name: nil) { appearance in
            (appearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua ? Swatch(dark) : Swatch(light)).nsColor
        })
    }
}

/// The reading screens' type: New York for what was said and written, SF Mono for times and
/// counts, SF Pro for the interface. Text styles, so the user's text size applies.
enum PaperType {
    /// A section's small caps label (`LAST MEETING`).
    static let label = Font.system(.caption2, design: .monospaced).weight(.medium)
    /// Times, lengths, counts.
    static let meta = Font.system(.caption, design: .monospaced)
    /// A record's title where it leads a screen.
    static let recordTitle = Font.system(.title2, design: .serif, weight: .medium)
    /// Words someone said or wrote.
    static let reading = Font.system(.body, design: .serif)
    /// A lede paragraph.
    static let lede = Font.system(.title3, design: .serif)
    /// A timestamp chip.
    static let chip = Font.system(.caption2, design: .monospaced)
}

/// A section label: small, monospaced, spaced capitals; a heading for VoiceOver.
struct SectionLabel: View {
    let text: String

    init(_ text: String) {
        self.text = text
    }

    var body: some View {
        Text(text.uppercased())
            .font(PaperType.label)
            .tracking(0.9)
            .foregroundStyle(Theme.secondaryText)
            .accessibilityLabel(text)
            .accessibilityAddTraits(.isHeader)
    }
}

/// A timestamp chip: plays the record from its moment.
struct StampChip: View {
    let ms: Int64
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Text(LibraryFormat.stamp(ms: ms))
                .font(PaperType.chip)
                .foregroundStyle(Paper.them)
                .padding(.horizontal, 4)
                .overlay(RoundedRectangle(cornerRadius: 4).strokeBorder(Paper.chip, lineWidth: 1))
        }
        .buttonStyle(.plain)
        .accessibilityLabel("Play from \(LibraryFormat.stamp(ms: ms))")
        .help("Play from here")
    }
}

/// A light, bordered button (the canvas's secondary buttons).
struct PaperButtonStyle: ButtonStyle {
    var prominent = false

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.callout.weight(.medium))
            .padding(.horizontal, 14)
            .frame(minHeight: 32)
            .foregroundStyle(prominent ? Paper.card : Theme.text)
            .background(
                RoundedRectangle(cornerRadius: 10)
                    .fill(prominent ? Paper.you : (configuration.isPressed ? Paper.highlight : Paper.card)))
            .overlay(RoundedRectangle(cornerRadius: 10).strokeBorder(prominent ? Paper.you : Paper.hairline, lineWidth: 1))
            .contentShape(RoundedRectangle(cornerRadius: 10))
    }
}
