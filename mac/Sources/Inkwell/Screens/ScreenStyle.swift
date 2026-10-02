// What the screens share: the secondary tones, the chips, the card, and the small labels. Every
// colour derives from the mode's tokens (DesignTokens.swift, Glow.swift); the dot colours are the
// theme's (GlowTheme), and no text is drawn in them.
import AppKit
import InkBridge
import SwiftUI

enum PaperPalette {
    /// A raised card: translucent over the orb.
    static let card = Theme.card
    /// Hairlines and card borders.
    static let border = Theme.border
    /// Row separators, lighter than borders.
    static let separator = Color(nsColor: Theme.dynamic { $0.border.nsColor.withAlphaComponent(0.7) })
    /// A neutral chip: the text colour, faint.
    static let chip = Color(nsColor: Theme.dynamic { $0.text.nsColor.withAlphaComponent(0.07) })
    /// A due-date chip.
    static let dueChip = chip
    static let dueText = Theme.text
    /// An overdue chip, and a card that needs the user.
    static let alertChip = Color(nsColor: Theme.dynamic { $0.alert.nsColor.withAlphaComponent(0.14) })
    static let alertText = Theme.alert
    static let alertCard = Theme.alertCard
    /// The recording dot.
    static let recording = Theme.alert
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
                .font(.system(size: Glow.Size.eyebrow, weight: tone == .neutral ? .regular : .medium))
                .foregroundStyle(foreground)
                .padding(.horizontal, 10)
                .padding(.vertical, 3)
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

    /// A small label over a section ("Your notes").
    struct Eyebrow: View {
        let text: String

        var body: some View {
            Text(text)
                .font(Typography.eyebrow)
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
                        // Counts in words ("5 open · 1 overdue"), not a time: the caption's face.
                        Text(subtitle)
                            .font(Typography.caption)
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
    /// A card: translucent over the orb, with a 22 pt corner and a hairline. Solid, with a stronger
    /// border, under Increase Contrast or Reduce Transparency.
    func paperCard(alert: Bool = false) -> some View {
        modifier(GlowCard(alert: alert))
    }
}

/// Glow's card.
struct GlowCard: ViewModifier {
    var alert = false
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency
    @Environment(\.colorSchemeContrast) private var contrast

    func body(content: Content) -> some View {
        let solid = reduceTransparency || contrast == .increased
        let shape = RoundedRectangle(cornerRadius: Glow.Radius.card, style: .continuous)
        content
            .background {
                if alert {
                    shape.fill(Theme.alertCard.opacity(solid ? 1 : 0.85))
                } else if solid {
                    shape.fill(Theme.solidCard)
                } else {
                    shape.fill(Theme.card).background(.ultraThinMaterial, in: shape)
                }
            }
            .overlay(shape.strokeBorder(
                alert ? Theme.alert.opacity(0.25) : (solid ? Theme.text.opacity(0.35) : Theme.border.opacity(0.7)),
                lineWidth: 1))
    }
}
