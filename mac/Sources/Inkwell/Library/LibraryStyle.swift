// What Today, the Library and a record add to the screens' shared style (ScreenStyle.swift): a
// few more tones, the reading type, the timestamp chip and the pill buttons.
import AppKit
import SwiftUI

/// What the reading screens add to the shared tones (PaperPalette, ScreenStyle.swift).
extension PaperPalette {
    /// A side panel (the library's list, the player bar): a card's translucency.
    static let panel = Theme.card
    /// A card that needs the user: its border, a tint of the alert colour.
    static let sealTint = Color(nsColor: Theme.dynamic { $0.alert.nsColor.withAlphaComponent(0.3) })
    /// Text a step below the body.
    static let quiet = Theme.secondaryText
    /// A timestamp chip's border.
    static let stampBorder = Theme.border
}

/// The reading screens' type: New York for what was said and written, SF Mono for times, SF Pro
/// for the interface.
enum PaperType {
    /// A section's small label ("Last meeting").
    static let label = Typography.eyebrow
    /// Times, lengths, counts.
    static let meta = Font.system(size: Glow.Size.caption)
    /// A record's title where it leads a screen.
    static let recordTitle = Font.system(size: 28, design: .serif)
    /// Words someone said or wrote.
    static let reading = Font.system(size: Glow.Size.body, design: .serif)
    /// A lede paragraph.
    static let lede = Font.system(size: 18, design: .serif)
    /// A timestamp chip.
    static let chip = Font.system(size: 11, design: .monospaced)
}

/// A timestamp chip: plays the record from its moment.
struct StampChip: View {
    let ms: Int64
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Text(LibraryFormat.stamp(ms: ms))
                .font(PaperType.chip)
                .foregroundStyle(Theme.secondaryText)
                .padding(.horizontal, 4)
                .overlay(RoundedRectangle(cornerRadius: 4).strokeBorder(PaperPalette.stampBorder, lineWidth: 1))
        }
        .buttonStyle(.plain)
        .accessibilityLabel("Play from \(LibraryFormat.stamp(ms: ms))")
        .help("Play from here")
    }
}

/// Glow's pill button: filled with the text colour when prominent, else a faint chip.
struct PaperButtonStyle: ButtonStyle {
    var prominent = false
    /// A larger pill (Today's Record now, Live's Stop).
    var large = false

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.system(size: large ? 15 : 14, weight: prominent ? .semibold : .regular))
            .padding(.horizontal, large ? 22 : 18)
            .frame(minHeight: large ? 46 : 34)
            .foregroundStyle(prominent ? Theme.buttonLabel : Theme.text)
            .background(Capsule().fill(prominent ? Theme.buttonFill : PaperPalette.chip))
            .opacity(configuration.isPressed ? 0.75 : 1)
            .contentShape(Capsule())
    }
}
