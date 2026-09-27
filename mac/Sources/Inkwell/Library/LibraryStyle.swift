// What Today, the Library and a record add to the screens' shared style (ScreenStyle.swift): a
// few more tones of paper, the reading type, the timestamp chip and the light button. Every text
// colour here passes 4.5:1 on the surface it sits on.
import AppKit
import SwiftUI

/// What the reading screens add to the shared tones (PaperPalette, ScreenStyle.swift).
extension PaperPalette {
    /// A side panel (the library's list, the player bar): paper a shade down (#EFEAE1).
    static let panel = tone(light: 0xEFEAE1, dark: 0x171A20)
    /// A card that needs the user: its border, a tint of the seal (#E1C3B7).
    static let sealTint = tone(light: 0xE1C3B7, dark: 0x5C2D24)
    /// Text a step below the body (the canvas's #43454E): 9.2:1 on paper.
    static let quiet = tone(light: 0x43454E, dark: 0xC9C3B8)
    /// Your words and your lane: ink, or paper on night paper.
    static let you = tone(light: 0x16181F, dark: 0xF2EEE6)
    /// A timestamp chip's border (#C9AE95).
    static let stampBorder = tone(light: 0xC9AE95, dark: 0x6B5039)

    private static func tone(light: UInt32, dark: UInt32) -> Color {
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

/// A timestamp chip: plays the record from its moment.
struct StampChip: View {
    let ms: Int64
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Text(LibraryFormat.stamp(ms: ms))
                .font(PaperType.chip)
                .foregroundStyle(PaperPalette.them)
                .padding(.horizontal, 4)
                .overlay(RoundedRectangle(cornerRadius: 4).strokeBorder(PaperPalette.stampBorder, lineWidth: 1))
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
            .foregroundStyle(prominent ? PaperPalette.card : Theme.text)
            .background(
                RoundedRectangle(cornerRadius: 10)
                    .fill(prominent ? PaperPalette.you : (configuration.isPressed ? PaperPalette.chip : PaperPalette.card)))
            .overlay(RoundedRectangle(cornerRadius: 10).strokeBorder(prominent ? PaperPalette.you : PaperPalette.border, lineWidth: 1))
            .contentShape(RoundedRectangle(cornerRadius: 10))
    }
}
