// Generated from design/tokens.json by `cargo run -p ink-shader --bin ink-tokens`.
// Do not edit: change the tokens and regenerate. A core test fails while this is stale.

/// An sRGB colour from the design tokens: its hex value, `0xRRGGBB`, and its opacity.
struct GlowColor: Equatable, Sendable {
    let hex: UInt32
    let alpha: Double

    init(_ hex: UInt32, alpha: Double = 1) {
        self.hex = hex
        self.alpha = alpha
    }

    var red: Double { Double((hex >> 16) & 0xFF) / 255 }
    var green: Double { Double((hex >> 8) & 0xFF) / 255 }
    var blue: Double { Double(hex & 0xFF) / 255 }
}

/// One mode's colours.
struct GlowPalette: Equatable, Sendable {
    /// The window's background.
    let background: GlowColor
    /// The page behind the window.
    let page: GlowColor
    /// A card's fill, over what is behind it, blurred.
    let card: GlowColor
    /// Text.
    let text: GlowColor
    /// Secondary text.
    let secondary: GlowColor
    /// Borders and hairlines.
    let border: GlowColor
    /// A button's fill.
    let buttonFill: GlowColor
    /// A button's label.
    let buttonLabel: GlowColor
    /// Something needs the user.
    let alert: GlowColor
    /// The fill of a card about something that needs the user.
    let alertCard: GlowColor
    /// The ink drop the orb blots down to.
    let ink: GlowColor
    /// The orb at rest.
    let idleOrb: GlowColor
    /// The blur behind a card, in points.
    let cardBlur: Double
}

/// A dot colour preset: your colour and the far end's.
struct GlowPreset: Equatable, Sendable, Identifiable {
    /// What `appearance.dots.light` and `appearance.dots.dark` store.
    let id: String
    /// What Settings shows.
    let name: String
    let you: GlowColor
    let them: GlowColor
}

/// Where the orb sits in its view: its centre as shares of the width and of the height (from
/// the top), and its unit (the orb shader's `unit`) as a share of the shorter side.
struct GlowOrbPlacement: Equatable, Sendable {
    let x: Double
    let y: Double
    let unit: Double
}

/// One stroke of the edge glow: its width in points, and its opacity.
struct GlowStroke: Equatable, Sendable {
    let width: Double
    let alpha: Double
}

/// The design tokens (design/tokens.json).
enum GlowTokens {
    /// The light mode's colours.
    static let light = GlowPalette(
        background: GlowColor(0xFBF8F4),
        page: GlowColor(0xEDE7DF),
        card: GlowColor(0xFFFFFF, alpha: 0.72),
        text: GlowColor(0x1D1B2E),
        secondary: GlowColor(0x6A6577),
        border: GlowColor(0xE2DACE),
        buttonFill: GlowColor(0x1D1B2E),
        buttonLabel: GlowColor(0xFBF8F4),
        alert: GlowColor(0xB23A26),
        alertCard: GlowColor(0xFFF1EC),
        ink: GlowColor(0x1D1B2E),
        idleOrb: GlowColor(0xC7BFDB),
        cardBlur: 22
    )

    /// The dark mode's colours.
    static let dark = GlowPalette(
        background: GlowColor(0x121118),
        page: GlowColor(0x0B0A0F),
        card: GlowColor(0x1C1A24, alpha: 0.62),
        text: GlowColor(0xEDEAF2),
        secondary: GlowColor(0xA49FB4),
        border: GlowColor(0x2A2833),
        buttonFill: GlowColor(0xEDEAF2),
        buttonLabel: GlowColor(0x121118),
        alert: GlowColor(0xFF9A80),
        alertCard: GlowColor(0x2E181E),
        ink: GlowColor(0xF0EBE3),
        idleOrb: GlowColor(0x5C5470),
        cardBlur: 22
    )

    /// The dot colour presets, in the order Settings lists them.
    static let presets: [GlowPreset] = [
        GlowPreset(
            id: "indigo", name: "Indigo & Coral",
            you: GlowColor(0x6B5CFF), them: GlowColor(0xFFA34D)),
        GlowPreset(
            id: "dusk", name: "Dusk",
            you: GlowColor(0x406BFF), them: GlowColor(0xF24DC7)),
        GlowPreset(
            id: "lagoon", name: "Lagoon",
            you: GlowColor(0x1FBFAD), them: GlowColor(0xFFBF47)),
        GlowPreset(
            id: "aurora", name: "Aurora",
            you: GlowColor(0x4DF2A6), them: GlowColor(0xB366FF)),
        GlowPreset(
            id: "citrus", name: "Citrus",
            you: GlowColor(0x2E52EB), them: GlowColor(0xFF9E26)),
        GlowPreset(
            id: "rosewater", name: "Rosewater",
            you: GlowColor(0x998CF2), them: GlowColor(0xFA8CA6)),
        GlowPreset(
            id: "ink_sand", name: "Ink & Sand",
            you: GlowColor(0x33384D), them: GlowColor(0xD9855C)),
    ]

    /// How a dot colour is fitted to the mode. Luminance is lumaRed * r + lumaGreen * g +
    /// lumaBlue * b, each channel 0 to 1.
    enum Fit {
        static let lumaRed: Double = 0.299
        static let lumaGreen: Double = 0.587
        static let lumaBlue: Double = 0.114
        /// In dark mode, a colour whose luminance is under this is lifted toward white,
        static let darkLiftBelow: Double = 0.3
        /// by this share of the way: c + (1 - c) * darkLift, per channel.
        static let darkLift: Double = 0.45
        /// In light mode, a colour whose luminance is over this is dimmed,
        static let lightDimAbove: Double = 0.85
        /// to c * lightDim, per channel.
        static let lightDim: Double = 0.7
        /// A colour's second shade in the orb: c + (1 - c) * partnerLift, per channel.
        static let partnerLift: Double = 0.4
    }

    /// Corner radii, in points.
    enum Radius {
        /// A card.
        static let card: Double = 22
        /// The window.
        static let window: Double = 26
        /// A pill button: fully round ends.
        static let pill: Double = 999
    }

    /// The type roles: the system's own faces, none bundled. Sizes in points.
    enum TypeRole {
        /// Titles and the greeting.
        static let display = "New York"
        /// The interface.
        static let ui = "SF Pro"
        /// Times, counts and versions.
        static let mono = "SF Mono"
        /// Today's greeting.
        static let greeting: Double = 66
        /// A screen's title.
        static let screenTitle: Double = 34
        /// A heading.
        static let heading: Double = 22
        /// Body text.
        static let body: Double = 15
        /// Captions.
        static let caption: Double = 13
        /// The small line over a heading.
        static let eyebrow: Double = 12
    }

    /// The glow round the window's edge.
    enum EdgeGlow {
        /// Its strokes, from the widest and faintest to the narrowest.
        static let strokes: [GlowStroke] = [
            GlowStroke(width: 44, alpha: 0.06),
            GlowStroke(width: 28, alpha: 0.11),
            GlowStroke(width: 16, alpha: 0.22),
            GlowStroke(width: 7, alpha: 0.45),
            GlowStroke(width: 2, alpha: 1),
        ]
        /// Half the blend between the two colours, as a share of the width.
        static let blendBand: Double = 0.26
        /// The blend's centre is 0.5 + lean * (you - them), as a share of the width.
        static let lean: Double = 0.2
        /// How far the blend drifts, as a share of the width,
        static let flowAmplitude: Double = 0.08
        /// and how fast, in radians per second.
        static let flowRate: Double = 0.7
    }

    /// Where the orb sits.
    enum Orb {
        /// Behind the main window's content.
        static let main = GlowOrbPlacement(x: 0.56, y: 0.26, unit: 0.72)
        /// In the Drop.
        static let drop = GlowOrbPlacement(x: 0.5, y: 0.5, unit: 1.15)
    }
}
