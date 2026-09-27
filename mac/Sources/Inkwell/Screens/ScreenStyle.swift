// What the Live, Owed and Settings screens share: the canvas's secondary paper tones, the chips,
// the card frame, and the small monospaced labels. Every colour derives from the palette in
// DesignTokens.swift or is the canvas's own; each says where it comes from.
import AppKit
import InkBridge
import SwiftUI

enum PaperPalette {
    /// A raised card on paper (the canvas's #FBF9F4), and a slightly lighter night paper.
    static let card = dynamic(light: 0xFBF9F4, dark: 0x181B21)
    /// Hairlines and card borders (#D9D1C3 by day, #2A2D35 by night).
    static let border = dynamic(light: 0xD9D1C3, dark: 0x2A2D35)
    /// Row separators, lighter than borders (#E4DDD0).
    static let separator = dynamic(light: 0xE4DDD0, dark: 0x22252C)
    /// A neutral chip (#E9E3D8).
    static let chip = dynamic(light: 0xE9E3D8, dark: 0x22252C)
    /// A due-date chip (#EFE3D6 with #6B4424 text).
    static let dueChip = dynamic(light: 0xEFE3D6, dark: 0x2E241C)
    static let dueText = dynamic(light: 0x6B4424, dark: 0xE2C3A5)
    /// An overdue chip, and a card that needs the user (#F3DDD7 with #8E2A1A; #FBF3EF behind).
    static let alertChip = dynamic(light: 0xF3DDD7, dark: 0x3A1D18)
    static let alertText = dynamic(light: 0x8E2A1A, dark: 0xF2B4A6)
    static let alertCard = dynamic(light: 0xFBF3EF, dark: 0x2A1714)
    /// The far end's ink: sepia by day; by night the canvas's lighter #C8956C, because sepia on
    /// night paper is 2.8:1.
    static let them = dynamic(light: 0x7E5431, dark: 0xC8956C)
    /// The recording dot (seal by day, the canvas's #E0644E by night).
    static let recording = dynamic(light: 0xB23A26, dark: 0xE0644E)

    private static func dynamic(light: UInt32, dark: UInt32) -> Color {
        Color(nsColor: NSColor(name: nil) { appearance in
            appearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua ? Swatch(dark).nsColor : Swatch(light).nsColor
        })
    }
}

/// The shared pieces, under one name so they cannot collide with another screen's.
enum Paper {
    /// A small rounded label.
    struct Chip: View {
        enum Tone {
            case neutral
            case due
            case alert
        }

        let text: String
        var tone: Tone = .neutral

        var body: some View {
            Text(text)
                .font(.system(.caption, weight: tone == .neutral ? .regular : .medium))
                .foregroundStyle(foreground)
                .padding(.horizontal, 8)
                .padding(.vertical, 2)
                .background(background, in: Capsule())
        }

        private var foreground: Color {
            switch tone {
            case .neutral: Theme.text
            case .due: PaperPalette.dueText
            case .alert: PaperPalette.alertText
            }
        }

        private var background: Color {
            switch tone {
            case .neutral: PaperPalette.chip
            case .due: PaperPalette.dueChip
            case .alert: PaperPalette.alertChip
            }
        }
    }

    /// A small uppercase label in SF Mono ("YOUR NOTES").
    struct Eyebrow: View {
        let text: String

        var body: some View {
            Text(text.uppercased())
                .font(.system(.caption2, design: .monospaced))
                .tracking(0.8)
                .foregroundStyle(Theme.secondaryText)
                .accessibilityAddTraits(.isHeader)
        }
    }

    /// A screen's title and the line under it, with anything on the right.
    struct Header<Trailing: View>: View {
        let title: String
        let subtitle: String?
        @ViewBuilder var trailing: Trailing

        var body: some View {
            HStack(alignment: .lastTextBaseline, spacing: 16) {
                VStack(alignment: .leading, spacing: 6) {
                    Text(title)
                        .font(Typography.screenTitle)
                        .foregroundStyle(Theme.text)
                        .accessibilityAddTraits(.isHeader)
                    if let subtitle {
                        Text(subtitle)
                            .font(Typography.timestamp)
                            .foregroundStyle(Theme.secondaryText)
                    }
                }
                Spacer(minLength: 0)
                trailing
            }
        }
    }
}

extension Paper.Header where Trailing == EmptyView {
    init(title: String, subtitle: String?) {
        self.init(title: title, subtitle: subtitle) { EmptyView() }
    }
}

extension View {
    /// The canvas's card: raised paper, a hairline border, 14 pt corners.
    func paperCard(alert: Bool = false) -> some View {
        background(alert ? PaperPalette.alertCard : PaperPalette.card, in: RoundedRectangle(cornerRadius: 14))
            .overlay(RoundedRectangle(cornerRadius: 14).strokeBorder(PaperPalette.border))
    }
}
