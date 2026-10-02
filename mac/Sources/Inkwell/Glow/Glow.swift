// Glow's design tokens, as the Mac shell uses them: the colours of each mode, the dot presets, the
// radii, the type sizes, the edge glow's strokes and where the orb sits.
//
// The values are design/tokens.json's, which both shells share. Until its generated Swift file is
// in this tree they are written out here, value for value; this enum is the one place the screens
// read them from either way.
import AppKit
import InkRenderer
import SwiftUI

enum Glow {
    /// One mode's colours.
    struct Mode: Equatable, Sendable {
        /// Behind the window's content.
        let background: Swatch
        /// Behind the window (the Drop's shadowed page, the canvas's board).
        let page: Swatch
        /// A card, at `cardAlpha` over the orb.
        let card: Swatch
        let cardAlpha: Double
        let text: Swatch
        let secondary: Swatch
        let border: Swatch
        /// A prominent button: filled with the text colour, labelled in the background's.
        let buttonFill: Swatch
        let buttonLabel: Swatch
        let alert: Swatch
        /// A card that needs the user.
        let alertCard: Swatch
        /// The blotted drop.
        let ink: Swatch
        /// The orb at rest.
        let idleOrb: Swatch
    }

    static let day = Mode(
        background: Swatch(0xFBF8F4), page: Swatch(0xEDE7DF), card: Swatch(0xFFFFFF), cardAlpha: 0.72,
        text: Swatch(0x1D1B2E), secondary: Swatch(0x6A6577), border: Swatch(0xE2DACE),
        buttonFill: Swatch(0x1D1B2E), buttonLabel: Swatch(0xFBF8F4), alert: Swatch(0xB23A26),
        alertCard: Swatch(0xFFF1EC), ink: Swatch(0x1D1B2E), idleOrb: Swatch(0xC7BFDB))

    static let night = Mode(
        background: Swatch(0x121118), page: Swatch(0x0B0A0F), card: Swatch(0x1C1A24), cardAlpha: 0.62,
        text: Swatch(0xEDEAF2), secondary: Swatch(0xA49FB4), border: Swatch(0x2A2833),
        buttonFill: Swatch(0xEDEAF2), buttonLabel: Swatch(0x121118), alert: Swatch(0xFF9A80),
        alertCard: Swatch(0x2E181E), ink: Swatch(0xF0EBE3), idleOrb: Swatch(0x5C5470))

    static func mode(dark: Bool) -> Mode { dark ? night : day }

    /// A card's background blur, in points.
    static let cardBlur: CGFloat = 22

    enum Radius {
        static let card: CGFloat = 22
        static let window: CGFloat = 26
        /// Pill buttons: fully round ends.
        static let pill: CGFloat = 999
    }

    /// The type roles' sizes, in points. Platform faces only: New York for display, SF Pro for the
    /// interface, SF Mono for times.
    enum Size {
        static let greeting: CGFloat = 66
        static let screenTitle: CGFloat = 34
        static let heading: CGFloat = 22
        static let body: CGFloat = 15
        static let caption: CGFloat = 13
        static let eyebrow: CGFloat = 12
    }

    /// A pair of dot colours: yours and theirs.
    struct Preset: Equatable, Identifiable, Sendable {
        let id: String
        let name: String
        let you: Swatch
        let them: Swatch
    }

    static let presets = [
        Preset(id: "indigo", name: "Indigo & Coral", you: Swatch(0x6B5CFF), them: Swatch(0xFFA34D)),
        Preset(id: "dusk", name: "Dusk", you: Swatch(0x406BFF), them: Swatch(0xF24DC7)),
        Preset(id: "lagoon", name: "Lagoon", you: Swatch(0x1FBFAD), them: Swatch(0xFFBF47)),
        Preset(id: "aurora", name: "Aurora", you: Swatch(0x4DF2A6), them: Swatch(0xB366FF)),
        Preset(id: "citrus", name: "Citrus", you: Swatch(0x2E52EB), them: Swatch(0xFF9E26)),
        Preset(id: "rosewater", name: "Rosewater", you: Swatch(0x998CF2), them: Swatch(0xFA8CA6)),
        Preset(id: "ink_sand", name: "Ink & Sand", you: Swatch(0x33384D), them: Swatch(0xD9855C)),
    ]

    /// The preset with `id`, else the first (the default).
    static func preset(_ id: String?) -> Preset {
        presets.first { $0.id == id } ?? presets[0]
    }

    /// The edge glow: its strokes wide to narrow, the blend band, the lean and the flow.
    static let edge = EdgeGlowStyle(
        strokes: [
            .init(width: 44, alpha: 0.06), .init(width: 28, alpha: 0.11), .init(width: 16, alpha: 0.22),
            .init(width: 7, alpha: 0.45), .init(width: 2, alpha: 1),
        ],
        band: 0.26, lean: 0.2, flowAmplitude: 0.08, flowSpeed: 0.7, cornerRadius: Radius.window)

    /// Where the orb sits behind the main window, and in the Drop.
    enum Orb {
        static let main = OrbPlacement(x: 0.56, yFromTop: 0.26, unit: 0.72)
        static let drop = OrbPlacement(x: 0.5, yFromTop: 0.5, unit: 1.15)
    }
}

// MARK: - Resolving colours

/// The dot colours, resolved the same way on both shells: the mode's preset, a colour of the
/// user's own in its place, fitted so it shows on the mode's background, and a lighter partner for
/// the orb's second shade.
enum GlowColours {
    typealias RGB = SIMD3<Double>

    static func rgb(_ swatch: Swatch) -> RGB { RGB(swatch.red, swatch.green, swatch.blue) }

    /// "#rrggbb" (lowercase, as the settings store it), or nil.
    static func parse(_ hex: String?) -> RGB? {
        guard let hex, hex.count == 7, hex.first == "#",
              hex.dropFirst().allSatisfy({ $0.isHexDigit && !$0.isUppercase }),
              let value = UInt32(hex.dropFirst(), radix: 16)
        else { return nil }
        return rgb(Swatch(value))
    }

    /// "#rrggbb", lowercase.
    static func hex(_ c: RGB) -> String {
        let bytes = [c.x, c.y, c.z].map { Int((min(1, max(0, $0)) * 255).rounded()) }
        return "#" + bytes.map { String(format: "%02x", $0) }.joined()
    }

    static func luminance(_ c: RGB) -> Double { 0.299 * c.x + 0.587 * c.y + 0.114 * c.z }

    /// Too dark for night is lifted; too pale for day is deepened.
    static func fit(_ c: RGB, dark: Bool) -> RGB {
        if dark, luminance(c) < 0.3 { return c + (RGB(1, 1, 1) - c) * 0.45 }
        if !dark, luminance(c) > 0.85 { return c * 0.7 }
        return c
    }

    /// The orb's second shade: the same colour, lighter.
    static func partner(_ c: RGB) -> RGB { c + (RGB(1, 1, 1) - c) * 0.4 }

    /// The colours one mode shows: you and them as chosen and fitted.
    static func dots(preset: Glow.Preset, you: String?, them: String?, dark: Bool) -> (you: RGB, them: RGB) {
        let you = parse(you) ?? rgb(preset.you)
        let them = parse(them) ?? rgb(preset.them)
        return (fit(you, dark: dark), fit(them, dark: dark))
    }

    /// The orb's palette for one mode.
    static func palette(preset: Glow.Preset, you: String?, them: String?, dark: Bool) -> OrbPalette {
        let dots = dots(preset: preset, you: you, them: them, dark: dark)
        let mode = Glow.mode(dark: dark)
        func f(_ c: RGB) -> SIMD3<Float> { SIMD3<Float>(c) }
        return OrbPalette(
            yA: f(dots.you), yB: f(partner(dots.you)), tA: f(dots.them), tB: f(partner(dots.them)),
            idle: f(rgb(mode.idleOrb)), ink: f(rgb(mode.ink)), dark: dark)
    }

    static func color(_ c: RGB) -> Color {
        Color(nsColor: nsColor(c))
    }

    static func nsColor(_ c: RGB) -> NSColor {
        NSColor(srgbRed: c.x, green: c.y, blue: c.z, alpha: 1)
    }
}
